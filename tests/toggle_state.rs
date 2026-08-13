mod support;

use herdr_simple_prompts::editor::{Editor, EditorSnapshot};
use herdr_simple_prompts::herdr::HerdrClient;
use herdr_simple_prompts::model::Attachment;
use herdr_simple_prompts::paste::{CompactPromptOverride, LARGE_PASTE_CHARS};
use herdr_simple_prompts::state::DraftWriter;
use herdr_simple_prompts::state::StateStore;
use herdr_simple_prompts::toggle::toggle;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;

const DAY_MS: u64 = 24 * 60 * 60 * 1_000;

fn test_state_directory(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "herdr-simple-prompts-{label}-{}",
        std::process::id(),
    ))
}

fn namespace_path(directory: &std::path::Path, pane_id: &str) -> std::path::PathBuf {
    directory
        .join("panes")
        .join(format!("{}.json", pane_id.replace(':', "_")))
}

fn write_namespace(
    directory: &std::path::Path,
    pane_id: &str,
    session_id: &str,
    last_verified_ms: u64,
    orphaned_since_ms: Option<u64>,
) {
    std::fs::create_dir_all(directory.join("panes")).unwrap();
    std::fs::write(
        namespace_path(directory, pane_id),
        serde_json::to_vec(&json!({
            "version": 1,
            "source_pane": pane_id,
            "session_id": session_id,
            "last_verified_ms": last_verified_ms,
            "orphaned_since_ms": orphaned_since_ms,
        }))
        .unwrap(),
    )
    .unwrap();
}

fn agent_info(pane_id: &str, session_id: &str) -> serde_json::Value {
    json!({
        "type": "agent_info",
        "agent": {
            "pane_id": pane_id,
            "agent_status": "idle",
            "foreground_cwd": "/tmp/project",
            "agent_session": {"kind": "id", "agent": "codex", "value": session_id}
        }
    })
}

fn create_scoped_state(
    store: &StateStore,
    directory: &std::path::Path,
    pane_id: &str,
    session_id: &str,
) -> std::path::PathBuf {
    store.save_overlay(pane_id, "w1:p9").unwrap();
    store
        .save_editor_draft(
            pane_id,
            Some(session_id),
            &EditorSnapshot::plain("private draft"),
            &[],
            &[],
        )
        .unwrap();
    let journal = store.history_journal(pane_id, session_id).unwrap();
    std::fs::create_dir_all(journal.path().parent().unwrap()).unwrap();
    std::fs::write(journal.path(), b"journal\n").unwrap();
    write_namespace(directory, pane_id, session_id, 1_000, None);
    journal.path().to_owned()
}

#[test]
fn chunked_draft_reopens_with_full_source_behind_compact_token() {
    let directory = test_state_directory("chunked-draft");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    let pasted = "draft-log-line\n".repeat(LARGE_PASTE_CHARS);
    let mut editor = Editor::default();
    editor.insert_paste(&pasted);

    store
        .save_editor_draft("w1:p1", None, &editor.snapshot(), &[], &[])
        .unwrap();

    let state = store.load_draft("w1:p1").unwrap();
    let mut restored = Editor::default();
    restored.replace_snapshot(state.editor);
    assert_eq!(restored.submission_text(), pasted);
    assert!(restored.display_text().contains("Pasted Content"));
    assert!(!restored.display_text().contains("draft-log-line"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn compact_history_metadata_does_not_persist_pasted_body() {
    let directory = test_state_directory("history-metadata");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    let pasted = "private-log-line\n".repeat(LARGE_PASTE_CHARS);
    let mut editor = Editor::default();
    editor.insert_paste(&pasted);
    let submission = editor.take_editor_submission();
    let summary = CompactPromptOverride::new(
        "session-1",
        "native-1",
        &submission.complete_text,
        submission.paste_ranges.clone(),
    );

    store
        .save_editor_draft(
            "w1:p1",
            Some("session-1"),
            &EditorSnapshot::default(),
            &[],
            &[summary],
        )
        .unwrap();

    let serialized = std::fs::read_to_string(directory.join("draft-w1_p1.json")).unwrap();
    assert!(!serialized.contains("private-log-line"));
    let state = store.load_draft("w1:p1").unwrap();
    assert_eq!(state.prompt_displays.len(), 1);
    assert_eq!(
        state.prompt_displays[0]
            .compact_text(&submission.complete_text)
            .unwrap(),
        submission.display_text,
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn legacy_string_draft_loads_as_plain_editor_snapshot() {
    let directory = test_state_directory("legacy-draft");
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("draft-w1_p1.json"),
        r#"{"text":"old\ndraft","attachments":[]}"#,
    )
    .unwrap();

    let state = StateStore::at(&directory).load_draft("w1:p1").unwrap();

    assert_eq!(state.editor, EditorSnapshot::plain("old\ndraft"));
    assert!(state.prompt_displays.is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unsupported_version_with_legacy_fields_is_quarantined() {
    let directory = test_state_directory("unsupported-draft-version");
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("draft-w1_p1.json"),
        r#"{"version":4,"text":"x","attachments":[]}"#,
    )
    .unwrap();
    let store = StateStore::at(&directory);

    assert!(store.load_draft("w1:p1").is_err());
    assert!(!directory.join("draft-w1_p1.json").exists());
    assert!(directory.join("draft-w1_p1.json.invalid").exists());
    assert_eq!(store.load_draft("w1:p1").unwrap(), Default::default());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn state_is_private_and_supports_reverse_overlay_lookup() {
    let directory =
        std::env::temp_dir().join(format!("herdr-simple-prompts-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);

    let journal = store.history_journal("w1:p1", "session-1").unwrap();
    assert_eq!(
        journal.path(),
        directory.join("history/w1_p1/session-1.jsonl")
    );

    store.save_overlay("w1:p1", "w1:p9").unwrap();

    assert_eq!(
        store.overlay_for_source("w1:p1").unwrap().as_deref(),
        Some("w1:p9")
    );
    assert_eq!(
        store.source_for_overlay("w1:p9").unwrap().as_deref(),
        Some("w1:p1")
    );
    let mode = std::fs::metadata(directory.join("registry.json"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    store
        .save_draft(
            "w1:p1",
            "multiline\ndraft",
            &[Attachment {
                id: "i1".into(),
                display: "screen.png".into(),
                native_path: Some("/private/tmp/herdr-staged/screen.png".into()),
            }],
        )
        .unwrap();
    let draft = store.load_draft("w1:p1").unwrap();
    assert_eq!(draft.text, "multiline\ndraft");
    assert_eq!(draft.attachments[0].display, "screen.png");
    assert!(draft.attachments[0].native_path.is_none());
    let serialized = std::fs::read_to_string(directory.join("draft-w1_p1.json")).unwrap();
    assert!(!serialized.contains("native_path"));
    assert!(!serialized.contains("herdr-staged"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn corrupt_draft_is_quarantined() {
    let directory = std::env::temp_dir().join(format!(
        "herdr-simple-prompts-corrupt-draft-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("draft-w1_p1.json"), b"not json").unwrap();
    let store = StateStore::at(&directory);

    assert!(store.load_draft("w1:p1").is_err());
    assert!(!directory.join("draft-w1_p1.json").exists());
    assert!(directory.join("draft-w1_p1.json.invalid").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn draft_writer_keeps_disk_io_off_the_caller_and_coalesces_latest_state() {
    let directory = std::env::temp_dir().join(format!(
        "herdr-simple-prompts-draft-writer-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    let writer = DraftWriter::spawn(store.clone(), "w1:p1".into(), None);
    let large = "x".repeat(2_000_000);

    let started = std::time::Instant::now();
    writer.queue(large, vec![]);
    writer.queue("latest".into(), vec![]);
    assert!(started.elapsed() < std::time::Duration::from_millis(50));
    drop(writer);

    assert_eq!(store.load_draft("w1:p1").unwrap().text, "latest");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn toggle_from_overlay_closes_and_refocuses_source() {
    let directory = std::env::temp_dir().join(format!(
        "herdr-simple-prompts-toggle-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    store.save_overlay("w1:p1", "w1:p9").unwrap();
    store.save_draft("w1:p1", "keep me", &[]).unwrap();
    let journal = store.history_journal("w1:p1", "session-1").unwrap();
    std::fs::create_dir_all(journal.path().parent().unwrap()).unwrap();
    std::fs::write(journal.path(), b"keep me too\n").unwrap();
    let fake = support::ScriptedHerdr::start(vec![
        json!({"type":"pane_info","pane":{"pane_id":"w1:p9"}}),
        json!({"type":"plugin_pane_closed"}),
        json!({"type":"pane_focused"}),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    toggle(&client, &store, "w1:p9").unwrap();

    let methods = fake
        .requests()
        .into_iter()
        .map(|request| request["method"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(methods, ["pane.get", "plugin.pane.close", "pane.focus"]);
    assert!(store.overlay_for_source("w1:p1").unwrap().is_none());
    assert_eq!(store.load_draft("w1:p1").unwrap().text, "keep me");
    assert!(journal.path().exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn failed_registry_write_closes_the_new_overlay() {
    let directory = std::env::temp_dir().join(format!(
        "herdr-simple-prompts-toggle-save-failure-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&directory);
    std::fs::write(&directory, b"blocks directory creation").unwrap();
    let store = StateStore::at(&directory);
    let fake = support::ScriptedHerdr::start(vec![
        json!({
            "type":"agent_info",
            "agent": {
                "pane_id":"w1:p1",
                "agent_status":"idle",
                "foreground_cwd":"/tmp/project",
                "agent_session":{"kind":"id","agent":"codex","value":"session-1"}
            }
        }),
        json!({"plugin_pane":{"pane":{"pane_id":"w1:p9"}}}),
        json!({"type":"plugin_pane_closed"}),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    assert!(toggle(&client, &store, "w1:p1").is_err());

    let methods = fake
        .requests()
        .into_iter()
        .map(|request| request["method"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        methods,
        ["agent.get", "plugin.pane.open", "plugin.pane.close"]
    );
    std::fs::remove_file(directory).unwrap();
}

#[test]
fn stale_overlay_is_replaced_without_disturbing_other_sources() {
    let directory = std::env::temp_dir().join(format!(
        "herdr-simple-prompts-stale-overlay-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    store.save_overlay("w1:p1", "w1:stale").unwrap();
    store.save_overlay("w1:p2", "w1:other").unwrap();
    let fake = support::ScriptedHerdr::start_responses(vec![
        Err(json!({"code":"not_found","message":"pane missing"})),
        Ok(json!({
            "type":"agent_info",
            "agent": {
                "pane_id":"w1:p1",
                "agent_status":"idle",
                "foreground_cwd":"/tmp/project",
                "agent_session":{"kind":"id","agent":"codex","value":"session-1"}
            }
        })),
        Ok(json!({"plugin_pane":{"pane":{"pane_id":"w1:new"}}})),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    toggle(&client, &store, "w1:p1").unwrap();

    assert_eq!(
        store.overlay_for_source("w1:p1").unwrap().as_deref(),
        Some("w1:new")
    );
    assert_eq!(
        store.overlay_for_source("w1:p2").unwrap().as_deref(),
        Some("w1:other")
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn draft_v3_persists_session_binding_and_v2_loads_unbound() {
    let directory = test_state_directory("draft-v3-session");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);

    store
        .save_editor_draft(
            "w1:p1",
            Some("session-1"),
            &EditorSnapshot::plain("bound"),
            &[],
            &[],
        )
        .unwrap();
    let serialized = std::fs::read_to_string(directory.join("draft-w1_p1.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    assert_eq!(value["version"], 3);
    assert_eq!(value["session_id"], "session-1");
    assert_eq!(
        store.load_draft("w1:p1").unwrap().session_id.as_deref(),
        Some("session-1")
    );

    std::fs::write(
        directory.join("draft-w1_p2.json"),
        r#"{"version":2,"editor":{"text":"legacy","chunks":[]},"attachments":[],"prompt_displays":[]}"#,
    )
    .unwrap();
    assert_eq!(store.load_draft("w1:p2").unwrap().session_id, None);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn namespace_validation_keeps_same_session_and_clears_orphan_marker() {
    let directory = test_state_directory("namespace-same-session");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    let journal = create_scoped_state(&store, &directory, "w1:p1", "session-1");
    write_namespace(&directory, "w1:p1", "session-1", 1_000, Some(2_000));
    let fake = support::ScriptedHerdr::start(vec![agent_info("w1:p1", "session-1")]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    store.validate_saved_namespaces(&client, 5_000).unwrap();

    let namespace: serde_json::Value =
        serde_json::from_slice(&std::fs::read(namespace_path(&directory, "w1:p1")).unwrap())
            .unwrap();
    assert_eq!(namespace["last_verified_ms"], 5_000);
    assert!(namespace["orphaned_since_ms"].is_null());
    assert!(journal.exists());
    assert_eq!(store.load_draft("w1:p1").unwrap().text, "private draft");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn namespace_validation_removes_state_only_for_proven_not_found() {
    let directory = test_state_directory("namespace-not-found");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    let journal = create_scoped_state(&store, &directory, "w1:p1", "session-1");
    let fake = support::ScriptedHerdr::start_responses(vec![Err(json!({
        "code": "not_found",
        "message": "pane missing"
    }))]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    store.validate_saved_namespaces(&client, 5_000).unwrap();

    assert!(store.overlay_for_source("w1:p1").unwrap().is_none());
    assert!(!directory.join("draft-w1_p1.json").exists());
    assert!(!journal.exists());
    assert!(!namespace_path(&directory, "w1:p1").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn namespace_validation_removes_old_session_but_keeps_new_session_draft() {
    let directory = test_state_directory("namespace-replacement-session");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    let old_journal = create_scoped_state(&store, &directory, "w1:p1", "session-old");
    store
        .save_editor_draft(
            "w1:p1",
            Some("session-new"),
            &EditorSnapshot::plain("new session draft"),
            &[],
            &[],
        )
        .unwrap();
    let fake = support::ScriptedHerdr::start(vec![agent_info("w1:p1", "session-new")]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    store.validate_saved_namespaces(&client, 5_000).unwrap();

    assert!(!old_journal.exists());
    assert!(!namespace_path(&directory, "w1:p1").exists());
    let draft = store.load_draft("w1:p1").unwrap();
    assert_eq!(draft.text, "new session draft");
    assert_eq!(draft.session_id.as_deref(), Some("session-new"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unavailable_namespace_is_orphaned_then_removed_after_seven_full_days() {
    let directory = test_state_directory("namespace-orphan-expiry");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    let journal = create_scoped_state(&store, &directory, "w1:p1", "session-1");
    let first = support::ScriptedHerdr::start_responses(vec![Err(json!({
        "code": "temporarily_unavailable",
        "message": "retry later"
    }))]);
    let first_client = HerdrClient::connect(first.socket_path()).unwrap();

    store
        .validate_saved_namespaces(&first_client, 10_000)
        .unwrap();

    let namespace: serde_json::Value =
        serde_json::from_slice(&std::fs::read(namespace_path(&directory, "w1:p1")).unwrap())
            .unwrap();
    assert_eq!(namespace["orphaned_since_ms"], 10_000);
    assert!(journal.exists());
    assert!(directory.join("draft-w1_p1.json").exists());

    let second = support::ScriptedHerdr::start_responses(vec![Err(json!({
        "code": "temporarily_unavailable",
        "message": "still unavailable"
    }))]);
    let second_client = HerdrClient::connect(second.socket_path()).unwrap();
    store
        .validate_saved_namespaces(&second_client, 10_000 + 7 * DAY_MS - 1)
        .unwrap();
    assert!(namespace_path(&directory, "w1:p1").exists());

    let third = support::ScriptedHerdr::start_responses(vec![Err(json!({
        "code": "temporarily_unavailable",
        "message": "still unavailable"
    }))]);
    let third_client = HerdrClient::connect(third.socket_path()).unwrap();
    store
        .validate_saved_namespaces(&third_client, 10_000 + 7 * DAY_MS)
        .unwrap();
    assert!(!namespace_path(&directory, "w1:p1").exists());
    assert!(!journal.exists());
    assert!(!directory.join("draft-w1_p1.json").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn permission_and_temporary_api_failures_never_delete_scoped_state() {
    for (label, code) in [
        ("namespace-permission", "permission_denied"),
        ("namespace-temporary", "temporarily_unavailable"),
    ] {
        let directory = test_state_directory(label);
        let _ = std::fs::remove_dir_all(&directory);
        let store = StateStore::at(&directory);
        let journal = create_scoped_state(&store, &directory, "w1:p1", "session-1");
        let fake = support::ScriptedHerdr::start_responses(vec![Err(json!({
            "code": code,
            "message": "unavailable"
        }))]);
        let client = HerdrClient::connect(fake.socket_path()).unwrap();

        store.validate_saved_namespaces(&client, 5_000).unwrap();

        assert!(namespace_path(&directory, "w1:p1").exists());
        assert!(journal.exists());
        assert!(directory.join("draft-w1_p1.json").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn missing_socket_stamps_and_ages_an_orphan_namespace() {
    let directory = test_state_directory("namespace-missing-socket");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    let journal = create_scoped_state(&store, &directory, "w1:p1", "session-1");
    let socket = directory.join("missing-herdr.sock");
    let client = HerdrClient::connect(&socket).unwrap();

    store.validate_saved_namespaces(&client, 10_000).unwrap();
    let namespace: serde_json::Value =
        serde_json::from_slice(&std::fs::read(namespace_path(&directory, "w1:p1")).unwrap())
            .unwrap();
    assert_eq!(namespace["orphaned_since_ms"], 10_000);
    assert!(journal.exists());

    store
        .validate_saved_namespaces(&client, 10_000 + 7 * DAY_MS)
        .unwrap();
    assert!(!namespace_path(&directory, "w1:p1").exists());
    assert!(!journal.exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn remove_pane_state_is_exact_and_rejects_hostile_targets() {
    let directory = test_state_directory("namespace-safe-cleanup");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    let first_journal = create_scoped_state(&store, &directory, "w1:p1", "session-1");
    let other_journal = create_scoped_state(&store, &directory, "w1:p10", "session-10");

    for hostile in ["..", "../w1:p1", "w1/p1", "w1_p1", "w1:p1:extra"] {
        assert!(store.remove_pane_state(hostile).is_err());
        assert!(first_journal.exists());
        assert!(other_journal.exists());
    }

    store.remove_pane_state("w1:p1").unwrap();
    assert!(!first_journal.exists());
    assert!(other_journal.exists());
    assert!(namespace_path(&directory, "w1:p10").exists());
    assert!(directory.join("draft-w1_p10.json").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn failed_replacement_cleanup_keeps_namespace_for_a_later_retry() {
    let directory = test_state_directory("replacement-cleanup-retry");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    create_scoped_state(&store, &directory, "w1:p1", "session-old");
    std::fs::write(directory.join("draft-w1_p1.json"), b"invalid draft").unwrap();
    let fake = support::ScriptedHerdr::start(vec![agent_info("w1:p1", "session-new")]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    assert!(store.validate_saved_namespaces(&client, 5_000).is_err());

    assert!(namespace_path(&directory, "w1:p1").exists());
    assert_eq!(
        store.overlay_for_source("w1:p1").unwrap().as_deref(),
        Some("w1:p9")
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn binding_a_verified_namespace_rewrites_a_v2_draft_as_bound_v3() {
    let directory = test_state_directory("bind-v2-draft");
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("draft-w1_p1.json"),
        r#"{"version":2,"editor":{"chunks":[{"kind":"text","value":"legacy"}]},"attachments":[],"prompt_displays":[]}"#,
    )
    .unwrap();
    let store = StateStore::at(&directory);

    store
        .bind_verified_namespace("w1:p1", "session-1", 5_000)
        .unwrap();

    let draft: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("draft-w1_p1.json")).unwrap())
            .unwrap();
    assert_eq!(draft["version"], 3);
    assert_eq!(draft["session_id"], "session-1");
    assert_eq!(store.load_draft("w1:p1").unwrap().text, "legacy");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn opening_an_overlay_persists_verified_session_namespace() {
    let directory = test_state_directory("toggle-binds-namespace");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    let fake = support::ScriptedHerdr::start(vec![
        agent_info("w1:p1", "session-1"),
        json!({"plugin_pane":{"pane":{"pane_id":"w1:p9"}}}),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    toggle(&client, &store, "w1:p1").unwrap();

    let namespace: serde_json::Value =
        serde_json::from_slice(&std::fs::read(namespace_path(&directory, "w1:p1")).unwrap())
            .unwrap();
    assert_eq!(namespace["source_pane"], "w1:p1");
    assert_eq!(namespace["session_id"], "session-1");
    assert!(namespace["orphaned_since_ms"].is_null());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cleanup_refuses_symlinked_history_namespace_and_preserves_external_target() {
    let directory = test_state_directory("cleanup-history-symlink");
    let external = test_state_directory("cleanup-history-external");
    let _ = std::fs::remove_dir_all(&directory);
    let _ = std::fs::remove_dir_all(&external);
    std::fs::create_dir_all(directory.join("history")).unwrap();
    std::fs::create_dir_all(&external).unwrap();
    std::fs::write(external.join("keep.jsonl"), b"external\n").unwrap();
    symlink(&external, directory.join("history/w1_p1")).unwrap();
    let store = StateStore::at(&directory);
    store.save_overlay("w1:p1", "w1:p9").unwrap();

    assert!(store.remove_pane_state("w1:p1").is_err());
    assert_eq!(
        std::fs::read(external.join("keep.jsonl")).unwrap(),
        b"external\n"
    );
    assert_eq!(
        store.overlay_for_source("w1:p1").unwrap().as_deref(),
        Some("w1:p9")
    );
    std::fs::remove_dir_all(directory).unwrap();
    std::fs::remove_dir_all(external).unwrap();
}

#[test]
fn validation_refuses_symlinked_namespace_metadata_and_preserves_external_target() {
    let directory = test_state_directory("metadata-symlink");
    let external = test_state_directory("metadata-symlink-external");
    let _ = std::fs::remove_dir_all(&directory);
    let _ = std::fs::remove_dir_all(&external);
    std::fs::create_dir_all(directory.join("panes")).unwrap();
    std::fs::create_dir_all(&external).unwrap();
    let external_metadata = external.join("keep.json");
    std::fs::write(&external_metadata, b"external\n").unwrap();
    symlink(&external_metadata, namespace_path(&directory, "w1:p1")).unwrap();
    let store = StateStore::at(&directory);
    let fake = support::ScriptedHerdr::start(vec![]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    assert!(store.validate_saved_namespaces(&client, 5_000).is_err());
    assert_eq!(std::fs::read(&external_metadata).unwrap(), b"external\n");
    std::fs::remove_dir_all(directory).unwrap();
    std::fs::remove_dir_all(external).unwrap();
}

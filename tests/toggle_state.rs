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

fn test_state_directory(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "herdr-simple-prompts-{label}-{}",
        std::process::id(),
    ))
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
        .save_editor_draft("w1:p1", &editor.snapshot(), &[], &[])
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
        .save_editor_draft("w1:p1", &EditorSnapshot::default(), &[], &[summary])
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
        r#"{"version":3,"text":"x","attachments":[]}"#,
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
    let writer = DraftWriter::spawn(store.clone(), "w1:p1".into());
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

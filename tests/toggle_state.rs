mod support;

use herdr_simple_prompts::herdr::HerdrClient;
use herdr_simple_prompts::model::Attachment;
use herdr_simple_prompts::state::DraftWriter;
use herdr_simple_prompts::state::StateStore;
use herdr_simple_prompts::toggle::toggle;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;

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

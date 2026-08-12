mod support;

use herdr_simple_prompts::herdr::HerdrClient;
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

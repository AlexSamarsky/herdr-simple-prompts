use herdr_simple_prompts::agent::codex::CodexAdapter;
use herdr_simple_prompts::model::ConversationEvent;

fn parse_fixture(path: &str) -> Vec<ConversationEvent> {
    let mut adapter = CodexAdapter;
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .enumerate()
        .filter_map(|(index, line)| adapter.ingest_line(index as u64 + 1, line).unwrap())
        .collect()
}

#[test]
fn emits_only_user_and_final_answer() {
    let events = parse_fixture("tests/fixtures/codex/complete.jsonl");

    assert_eq!(events.len(), 2);
    assert!(matches!(
        &events[0],
        ConversationEvent::User(message)
            if message.text == "build it" && message.timestamp_ms == Some(1_786_528_800_000)
    ));
    assert!(matches!(
        &events[1],
        ConversationEvent::Final(message) if message.text == "Built and verified."
    ));
}

#[test]
fn excludes_internal_events_and_keeps_user_images() {
    let events = parse_fixture("tests/fixtures/codex/filtered.jsonl");

    assert_eq!(events.len(), 1);
    let ConversationEvent::User(message) = &events[0] else {
        panic!("expected a user event");
    };
    assert_eq!(message.text, "with image");
    assert_eq!(message.attachments.len(), 1);
    assert_eq!(message.attachments[0].display, "a.png");
}

#[test]
fn preserves_native_compact_paste_marker_exactly() {
    let mut adapter = CodexAdapter;
    let text = "inspect\n[Pasted Content 1000 chars]";
    let line = serde_json::json!({
        "type": "event_msg",
        "payload": {
            "type": "user_message",
            "message": text,
        },
    })
    .to_string();

    let event = adapter.ingest_line(1, &line).unwrap().unwrap();

    assert!(matches!(
        event,
        ConversationEvent::User(message) if message.text == text
    ));
}

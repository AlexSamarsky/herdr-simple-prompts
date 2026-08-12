use herdr_simple_prompts::agent::claude::ClaudeAdapter;
use herdr_simple_prompts::model::ConversationEvent;

fn ingest_fixture(adapter: &mut ClaudeAdapter, path: &str) -> Vec<ConversationEvent> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .enumerate()
        .flat_map(|(index, line)| adapter.ingest_line(index as u64 + 1, line).unwrap())
        .collect()
}

#[test]
fn simple_answer_is_committed_only_when_the_turn_finishes() {
    let mut adapter = ClaudeAdapter::default();
    let events = ingest_fixture(&mut adapter, "tests/fixtures/claude/simple.jsonl");

    assert_eq!(events.len(), 1);
    assert!(
        matches!(&events[0], ConversationEvent::User(message) if message.text == "hello claude")
    );
    assert!(matches!(
        adapter.finalize_pending(),
        Some(ConversationEvent::Final(message)) if message.text == "Hello back."
    ));
}

#[test]
fn tool_cycle_commits_only_terminal_visible_text() {
    let mut adapter = ClaudeAdapter::default();
    let events = ingest_fixture(&mut adapter, "tests/fixtures/claude/tool_cycle.jsonl");

    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], ConversationEvent::User(message) if message.text == "fix it"));
    assert!(matches!(
        adapter.finalize_pending(),
        Some(ConversationEvent::Final(message)) if message.text == "Fixed and tested."
    ));
}

#[test]
fn excludes_meta_thinking_progress_and_sidechains_but_keeps_images() {
    let mut adapter = ClaudeAdapter::default();
    let events = ingest_fixture(&mut adapter, "tests/fixtures/claude/filtered.jsonl");

    assert_eq!(events.len(), 1);
    let ConversationEvent::User(message) = &events[0] else {
        panic!("expected a user event");
    };
    assert_eq!(message.text, "look at this");
    assert_eq!(message.attachments.len(), 1);
    assert!(adapter.finalize_pending().is_none());
}

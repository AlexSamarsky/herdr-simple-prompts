mod support;

use herdr_simple_prompts::agent::AgentStatus;
use herdr_simple_prompts::agent::claude::ClaudeAdapter;
use herdr_simple_prompts::agent::codex::CodexAdapter;
use herdr_simple_prompts::agent::follower::{FollowerEvent, TranscriptFollower};

#[test]
fn waits_for_complete_json_line_then_emits_once() {
    let file = support::GrowingFile::new();
    let mut follower = TranscriptFollower::new(file.path(), Box::new(CodexAdapter)).unwrap();
    file.append("{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",");
    assert!(follower.poll().unwrap().is_empty());
    file.append("\"message\":\"hello\",\"images\":[]}}\n");

    let events = follower.poll().unwrap();

    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], FollowerEvent::Conversation(_)));
    assert!(follower.poll().unwrap().is_empty());
}

#[test]
fn replacement_reloads_without_reusing_partial_bytes() {
    let file = support::GrowingFile::new();
    let mut follower = TranscriptFollower::new(file.path(), Box::new(CodexAdapter)).unwrap();
    file.append("partial");
    assert!(follower.poll().unwrap().is_empty());
    file.replace("{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"new\",\"images\":[]}}\n");

    let events = follower.poll().unwrap();

    assert!(matches!(events[0], FollowerEvent::Reloaded));
    assert!(matches!(events[1], FollowerEvent::Conversation(_)));
}

#[test]
fn large_complete_line_is_not_truncated() {
    let file = support::GrowingFile::new();
    let text = "я".repeat(1_000_000);
    file.append(&format!(
        "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":{},\"images\":[]}}}}\n",
        serde_json::to_string(&text).unwrap()
    ));
    let mut follower = TranscriptFollower::new(file.path(), Box::new(CodexAdapter)).unwrap();

    let events = follower.poll().unwrap();

    let FollowerEvent::Conversation(herdr_simple_prompts::model::ConversationEvent::User(message)) =
        &events[0]
    else {
        panic!("expected user message");
    };
    assert_eq!(message.text, text);
}

#[test]
fn initial_idle_claude_session_includes_its_pending_final_answer() {
    let file = support::GrowingFile::new();
    file.append(&std::fs::read_to_string("tests/fixtures/claude/simple.jsonl").unwrap());
    let mut follower =
        TranscriptFollower::new(file.path(), Box::new(ClaudeAdapter::default())).unwrap();

    let events = follower.poll_initial(AgentStatus::Done).unwrap();

    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], FollowerEvent::Conversation(_)));
    assert!(matches!(events[1], FollowerEvent::Conversation(_)));
}

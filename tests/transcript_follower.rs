mod support;

use herdr_simple_prompts::agent::AgentStatus;
use herdr_simple_prompts::agent::claude::ClaudeAdapter;
use herdr_simple_prompts::agent::codex::CodexAdapter;
use herdr_simple_prompts::agent::follower::{FollowerEvent, TranscriptFollower};
use herdr_simple_prompts::app::{AppEvent, AppState};
use herdr_simple_prompts::history::VisibleRole;

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
fn same_inode_truncate_and_regrow_reloads_from_the_beginning() {
    let file = support::GrowingFile::new();
    file.append("{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"old\",\"images\":[]}}\n");
    let mut follower = TranscriptFollower::new(file.path(), Box::new(CodexAdapter)).unwrap();
    assert_eq!(follower.poll().unwrap().len(), 1);
    file.truncate_and_regrow(
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"new and deliberately longer than the prior record\",\"images\":[]}}\n",
    );

    let events = follower.poll().unwrap();

    assert!(matches!(events[0], FollowerEvent::Reloaded));
    let FollowerEvent::Conversation(herdr_simple_prompts::model::ConversationEvent::User(message)) =
        &events[1]
    else {
        panic!("expected reloaded user message");
    };
    assert!(message.text.starts_with("new and deliberately"));
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

#[test]
fn initial_blocked_claude_session_keeps_its_pending_answer_unfinalized() {
    let file = support::GrowingFile::new();
    file.append(&std::fs::read_to_string("tests/fixtures/claude/simple.jsonl").unwrap());
    let mut follower =
        TranscriptFollower::new(file.path(), Box::new(ClaudeAdapter::default())).unwrap();

    let events = follower.poll_initial(AgentStatus::Blocked).unwrap();

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        FollowerEvent::Conversation(herdr_simple_prompts::model::ConversationEvent::User(_))
    ));
    let mut app = AppState::default();
    for event in events {
        if let FollowerEvent::Conversation(conversation) = event {
            match conversation {
                herdr_simple_prompts::model::ConversationEvent::User(message) => {
                    app.apply(AppEvent::NativeUser(message));
                }
                herdr_simple_prompts::model::ConversationEvent::Final(message) => {
                    app.apply(AppEvent::NativeFinal(message));
                }
            }
        }
    }
    assert!(
        app.drain_history_upserts()
            .iter()
            .all(|record| record.role != VisibleRole::Final)
    );
}

#[test]
fn claude_finalizes_when_done_arrives_after_the_assistant_line() {
    let file = support::GrowingFile::new();
    file.append(&std::fs::read_to_string("tests/fixtures/claude/simple.jsonl").unwrap());
    let mut follower =
        TranscriptFollower::new(file.path(), Box::new(ClaudeAdapter::default())).unwrap();

    let working = follower.poll_for_status(AgentStatus::Working).unwrap();
    let done = follower.poll_for_status(AgentStatus::Done).unwrap();

    assert_eq!(working.len(), 1);
    assert!(matches!(done.as_slice(), [FollowerEvent::Conversation(_)]));
}

#[test]
fn claude_finalizes_when_done_arrives_before_the_assistant_line() {
    let file = support::GrowingFile::new();
    let fixture = std::fs::read_to_string("tests/fixtures/claude/simple.jsonl").unwrap();
    let mut lines = fixture.lines();
    file.append(&format!("{}\n", lines.next().unwrap()));
    let mut follower =
        TranscriptFollower::new(file.path(), Box::new(ClaudeAdapter::default())).unwrap();
    assert_eq!(
        follower
            .poll_for_status(AgentStatus::Working)
            .unwrap()
            .len(),
        1
    );

    file.append(&format!("{}\n", lines.next().unwrap()));
    let done = follower.poll_for_status(AgentStatus::Done).unwrap();

    assert!(matches!(done.as_slice(), [FollowerEvent::Conversation(_)]));
}

use herdr_simple_prompts::app::{AppEvent, AppState};
use herdr_simple_prompts::model::{Delivery, Message};

#[test]
fn submitted_prompt_is_visible_then_reconciles_without_duplicate() {
    let mut app = AppState::default();
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        text: "ship it".into(),
        attachments: vec![],
        at_ms: 100,
    });
    assert_eq!(app.turns.len(), 1);
    assert!(matches!(app.turns[0].delivery, Delivery::Optimistic { .. }));

    app.apply(AppEvent::NativeUser(Message::text(
        "native-1",
        "ship it",
        Some(120),
    )));

    assert_eq!(app.turns.len(), 1);
    assert_eq!(app.turns[0].prompt.stable_id, "native-1");
    assert_eq!(app.turns[0].delivery, Delivery::Native);
}

#[test]
fn send_failure_restores_draft_and_marks_turn_failed() {
    let mut app = AppState::default();
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        text: "retry exactly\nwith lines".into(),
        attachments: vec![],
        at_ms: 1,
    });

    app.apply(AppEvent::SendFailed {
        local_id: "local-1".into(),
        reason: "pane closed".into(),
    });

    assert_eq!(app.draft, "retry exactly\nwith lines");
    assert!(matches!(
        app.turns[0].delivery,
        Delivery::Failed { ref reason } if reason == "pane closed"
    ));
}

#[test]
fn queued_prompts_and_final_answers_keep_native_order() {
    let mut app = AppState::default();
    for (id, text, at_ms) in [("l1", "first", 1), ("l2", "second", 2)] {
        app.apply(AppEvent::PromptSubmitted {
            local_id: id.into(),
            text: text.into(),
            attachments: vec![],
            at_ms,
        });
    }
    app.apply(AppEvent::NativeUser(Message::text("u1", "first", Some(3))));
    app.apply(AppEvent::NativeFinal(Message::text("a1", "one", Some(4))));
    app.apply(AppEvent::NativeUser(Message::text("u2", "second", Some(5))));
    app.apply(AppEvent::NativeFinal(Message::text("a2", "two", Some(6))));

    assert_eq!(app.turns.len(), 2);
    assert_eq!(app.turns[0].final_answer.as_ref().unwrap().text, "one");
    assert_eq!(app.turns[1].final_answer.as_ref().unwrap().text, "two");
}

#[test]
fn transcript_reload_replaces_native_history_but_keeps_unsent_work() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "old",
        "old prompt",
        Some(1),
    )));
    app.apply(AppEvent::NativeFinal(Message::text(
        "old-final",
        "old answer",
        Some(2),
    )));
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        text: "still sending".into(),
        attachments: vec![],
        at_ms: 3,
    });

    app.apply(AppEvent::TranscriptReloaded);
    app.apply(AppEvent::NativeUser(Message::text(
        "native-local",
        "still sending",
        Some(4),
    )));
    app.apply(AppEvent::TranscriptReplayComplete);

    assert_eq!(app.turns.len(), 1);
    assert_eq!(app.turns[0].prompt.stable_id, "native-local");
    assert_eq!(app.turns[0].delivery, Delivery::Native);
}

#[test]
fn reload_does_not_reconcile_an_old_identical_prompt_with_current_submission() {
    let mut app = AppState::default();
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-current".into(),
        text: "same prompt".into(),
        attachments: vec![],
        at_ms: 100_000,
    });

    app.apply(AppEvent::TranscriptReloaded);
    app.apply(AppEvent::NativeUser(Message::text(
        "native-old",
        "same prompt",
        Some(1_000),
    )));
    app.apply(AppEvent::NativeUser(Message::text(
        "native-current",
        "same prompt",
        Some(100_100),
    )));
    app.apply(AppEvent::TranscriptReplayComplete);

    assert_eq!(app.turns.len(), 2);
    assert_eq!(app.turns[0].prompt.stable_id, "native-old");
    assert_eq!(app.turns[1].prompt.stable_id, "native-current");
    assert_eq!(app.turns[1].delivery, Delivery::Native);
}

#[test]
fn final_answer_skips_an_older_interrupted_turn() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "interrupted",
        Some(1),
    )));
    app.apply(AppEvent::NativeUser(Message::text("u2", "next", Some(2))));

    app.apply(AppEvent::NativeFinal(Message::text("a2", "done", Some(3))));

    assert!(app.turns[0].final_answer.is_none());
    assert_eq!(app.turns[1].final_answer.as_ref().unwrap().text, "done");
}

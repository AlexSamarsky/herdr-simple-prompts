use herdr_simple_prompts::app::{AppEvent, AppState};
use herdr_simple_prompts::editor::{Editor, EditorSnapshot, EditorSubmission};
use herdr_simple_prompts::model::{Delivery, Message};
use herdr_simple_prompts::paste::CompactPromptOverride;

fn compact_submission(source: &str) -> EditorSubmission {
    let mut editor = Editor::default();
    editor.insert_paste(source);
    editor.take_editor_submission()
}

fn plain_submission(source: &str) -> EditorSubmission {
    let mut editor = Editor::default();
    editor.replace(source);
    editor.take_editor_submission()
}

#[test]
fn submitted_prompt_is_visible_then_reconciles_without_duplicate() {
    let mut app = AppState::default();
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        submission: plain_submission("ship it"),
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
fn plain_submission_reconciles_without_persisting_prompt_display_metadata() {
    let mut app = AppState {
        session_id: "session-1".into(),
        ..AppState::default()
    };
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        submission: plain_submission("ship it"),
        attachments: vec![],
        at_ms: 100,
    });

    app.apply(AppEvent::NativeUser(Message::text(
        "native-1",
        "ship it",
        Some(120),
    )));

    assert_eq!(app.turns[0].delivery, Delivery::Native);
    assert!(app.prompt_displays.is_empty());
}

#[test]
fn plain_reconciliation_removes_only_stale_same_key_override() {
    let mut app = AppState {
        session_id: "session-1".into(),
        prompt_displays: vec![
            CompactPromptOverride::new("session-1", "native-1", "stale", vec![]),
            CompactPromptOverride::new("session-1", "native-other", "keep", vec![]),
            CompactPromptOverride::new("session-other", "native-1", "keep", vec![]),
        ],
        ..AppState::default()
    };
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        submission: plain_submission("ship it"),
        attachments: vec![],
        at_ms: 100,
    });

    app.apply(AppEvent::NativeUser(Message::text(
        "native-1",
        "ship it",
        Some(120),
    )));

    assert_eq!(app.turns[0].delivery, Delivery::Native);
    assert_eq!(app.prompt_displays.len(), 2);
    assert!(
        app.prompt_displays.iter().all(|summary| {
            summary.session_id != "session-1" || summary.stable_id != "native-1"
        })
    );
    assert!(app.prompt_displays.iter().any(|summary| {
        summary.session_id == "session-1" && summary.stable_id == "native-other"
    }));
    assert!(app.prompt_displays.iter().any(|summary| {
        summary.session_id == "session-other" && summary.stable_id == "native-1"
    }));
}

#[test]
fn send_failure_restores_draft_and_marks_turn_failed() {
    let mut app = AppState::default();
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        submission: plain_submission("retry exactly\nwith lines"),
        attachments: vec![],
        at_ms: 1,
    });

    app.apply(AppEvent::SendFailed {
        local_id: "local-1".into(),
        reason: "pane closed".into(),
    });

    assert_eq!(
        app.draft,
        EditorSnapshot::plain("retry exactly\nwith lines")
    );
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
            submission: plain_submission(text),
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
        submission: plain_submission("still sending"),
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
        submission: plain_submission("same prompt"),
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

#[test]
fn full_native_source_reconciles_one_optimistic_compact_prompt() {
    let source = "private-log-line\n".repeat(1_000);
    let submission = compact_submission(&source);
    let expected_display = submission.display_text.clone();
    let mut app = AppState {
        session_id: "session-1".into(),
        ..AppState::default()
    };
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        submission,
        attachments: vec![],
        at_ms: 100,
    });

    app.apply(AppEvent::NativeUser(Message::text(
        "native-1",
        source,
        Some(120),
    )));

    assert_eq!(app.turns.len(), 1);
    assert_eq!(app.turns[0].prompt.text, expected_display);
    assert_eq!(app.turns[0].delivery, Delivery::Native);
    assert_eq!(app.prompt_displays.len(), 1);
}

#[test]
fn native_marker_reconciles_and_preserves_provider_text_exactly() {
    let source = "x".repeat(1_000);
    let mut app = AppState {
        session_id: "session-1".into(),
        ..AppState::default()
    };
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        submission: compact_submission(&source),
        attachments: vec![],
        at_ms: 100,
    });

    app.apply(AppEvent::NativeUser(Message::text(
        "native-1",
        "[Pasted Content 1000 chars]",
        Some(120),
    )));

    assert_eq!(app.turns.len(), 1);
    assert_eq!(app.turns[0].prompt.text, "[Pasted Content 1000 chars]");
    assert_eq!(app.turns[0].delivery, Delivery::Native);
    assert_eq!(app.prompt_displays.len(), 1);
}

#[test]
fn replay_applies_compact_override_to_full_provider_source() {
    let source = "private-log-line\n".repeat(1_000);
    let submission = compact_submission(&source);
    let expected_display = submission.display_text.clone();
    let mut app = AppState {
        session_id: "session-1".into(),
        prompt_displays: vec![CompactPromptOverride::new(
            "session-1",
            "native-1",
            &source,
            submission.paste_ranges,
        )],
        ..AppState::default()
    };

    app.apply(AppEvent::NativeUser(Message::text(
        "native-1",
        source,
        Some(120),
    )));

    assert_eq!(app.turns.len(), 1);
    assert_eq!(app.turns[0].prompt.text, expected_display);
    assert!(!app.turns[0].prompt.text.contains("private-log-line"));
}

#[test]
fn send_failure_restores_exact_snapshot_with_two_large_pastes() {
    let first = "first-private-log\n".repeat(1_000);
    let second = "second-private-log\n".repeat(1_000);
    let mut editor = Editor::default();
    editor.insert_char('>');
    editor.insert_paste(&first);
    editor.insert_char('|');
    editor.insert_paste(&second);
    editor.insert_char('<');
    let submission = editor.take_editor_submission();
    let recovery = submission.recovery.clone();
    let mut app = AppState::default();
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        submission,
        attachments: vec![],
        at_ms: 100,
    });

    app.apply(AppEvent::SendFailed {
        local_id: "local-1".into(),
        reason: "pane closed".into(),
    });

    assert_eq!(app.draft, recovery);
    let mut restored = Editor::default();
    restored.replace_snapshot(app.draft.clone());
    assert_eq!(restored.display_text().matches("Pasted Content").count(), 2);
    assert!(!restored.display_text().contains("private-log"));
}

#[test]
fn history_revision_changes_only_when_history_mutates() {
    let mut app = AppState::default();

    app.apply(AppEvent::TranscriptReplayComplete);
    app.apply(AppEvent::NativeFinal(Message::text(
        "a0",
        "orphan",
        Some(1),
    )));
    app.apply(AppEvent::SendFailed {
        local_id: "missing".into(),
        reason: "missing".into(),
    });
    app.apply(AppEvent::TranscriptReloaded);
    assert_eq!(app.history_revision, 0);

    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        submission: plain_submission("first"),
        attachments: vec![],
        at_ms: 2,
    });
    assert_eq!(app.history_revision, 1);

    app.apply(AppEvent::NativeUser(Message::text("u1", "first", Some(3))));
    assert_eq!(app.history_revision, 2);

    app.apply(AppEvent::NativeFinal(Message::text("a1", "done", Some(4))));
    assert_eq!(app.history_revision, 3);

    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-2".into(),
        submission: plain_submission("second"),
        attachments: vec![],
        at_ms: 5,
    });
    assert_eq!(app.history_revision, 4);

    app.apply(AppEvent::SendFailed {
        local_id: "local-2".into(),
        reason: "closed".into(),
    });
    assert_eq!(app.history_revision, 5);

    app.apply(AppEvent::TranscriptReloaded);
    assert_eq!(app.history_revision, 6);

    app.apply(AppEvent::TranscriptReplayComplete);
    assert_eq!(app.history_revision, 6);
}

use herdr_simple_prompts::app::{AppEvent, AppState, NOTICE_LINGER};
use herdr_simple_prompts::composer::{ComposerAccess, NativeComposerState};
use herdr_simple_prompts::editor::{Editor, EditorSnapshot, EditorSubmission};
use herdr_simple_prompts::history::{PersistedPresentation, VisibleHistoryRecord, VisibleRole};
use herdr_simple_prompts::model::{Attachment, Delivery, Message};
use herdr_simple_prompts::paste::CompactPromptOverride;
use herdr_simple_prompts::paste::fingerprint;
use herdr_simple_prompts::style::{
    AnsiColor, MessagePresentation, StyleModifiers, StyleRun, StyledText,
};

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

fn native_presentation(text: &str, runs: Vec<StyleRun>) -> MessagePresentation {
    MessagePresentation::NativeAnsi(StyledText {
        text: text.into(),
        runs,
    })
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
fn transcript_reload_reconciles_unsent_work_and_retains_missing_native_history() {
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

    assert_eq!(app.turns.len(), 2);
    assert_eq!(app.turns[0].prompt.stable_id, "native-local");
    assert_eq!(app.turns[0].delivery, Delivery::Native);
    assert_eq!(app.turns[1].prompt.stable_id, "old");
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
fn final_presentation_applies_only_to_the_same_stable_id_and_text_fingerprint() {
    let canonical = "**canonical** [docs](https://example.test)";
    let rendered = "canonical docs";
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "prompt",
        "question",
        Some(1),
    )));
    app.apply(AppEvent::NativeFinal(Message::final_text(
        "answer",
        canonical,
        Some(2),
    )));
    let native = native_presentation(
        rendered,
        vec![StyleRun {
            start_byte: 0,
            end_byte: rendered.len(),
            foreground: Some(AnsiColor::Green),
            background: None,
            modifiers: StyleModifiers::default(),
        }],
    );

    app.apply(AppEvent::FinalPresentation {
        stable_id: "other".into(),
        text_fingerprint: fingerprint(canonical),
        presentation: native.clone(),
    });
    app.apply(AppEvent::FinalPresentation {
        stable_id: "answer".into(),
        text_fingerprint: fingerprint("replaced"),
        presentation: native.clone(),
    });
    assert_eq!(
        app.turns[0].final_answer.as_ref().unwrap().presentation,
        MessagePresentation::MarkdownFallback
    );

    app.apply(AppEvent::FinalPresentation {
        stable_id: "answer".into(),
        text_fingerprint: fingerprint(canonical),
        presentation: native.clone(),
    });

    assert_eq!(
        app.turns[0].final_answer.as_ref().unwrap().presentation,
        native
    );
}

#[test]
fn final_presentation_rejects_controls_and_ranges_invalid_for_rendered_text() {
    let canonical = "x".repeat(120);
    let invalid = [
        native_presentation("unsafe\u{1b}", Vec::new()),
        native_presentation(
            "short",
            vec![StyleRun {
                start_byte: 0,
                end_byte: 99,
                foreground: Some(AnsiColor::Green),
                background: None,
                modifiers: StyleModifiers::default(),
            }],
        ),
    ];

    for presentation in invalid {
        let mut app = AppState::default();
        app.apply(AppEvent::NativeUser(Message::text(
            "prompt",
            "question",
            Some(1),
        )));
        app.apply(AppEvent::NativeFinal(Message::final_text(
            "answer",
            canonical.clone(),
            Some(2),
        )));

        app.apply(AppEvent::FinalPresentation {
            stable_id: "answer".into(),
            text_fingerprint: fingerprint(&canonical),
            presentation,
        });

        assert_eq!(
            app.turns[0].final_answer.as_ref().unwrap().presentation,
            MessagePresentation::MarkdownFallback
        );
    }
}

#[test]
fn final_presentation_rejects_overlapping_or_non_utf8_rendered_ranges() {
    let invalid_runs = [
        vec![StyleRun {
            start_byte: 0,
            end_byte: 99,
            foreground: Some(AnsiColor::Green),
            background: None,
            modifiers: StyleModifiers::default(),
        }],
        vec![
            StyleRun {
                start_byte: 0,
                end_byte: 2,
                foreground: Some(AnsiColor::Green),
                background: None,
                modifiers: StyleModifiers::default(),
            },
            StyleRun {
                start_byte: 1,
                end_byte: 3,
                foreground: Some(AnsiColor::Red),
                background: None,
                modifiers: StyleModifiers::default(),
            },
        ],
        vec![StyleRun {
            start_byte: 1,
            end_byte: 2,
            foreground: Some(AnsiColor::Green),
            background: None,
            modifiers: StyleModifiers::default(),
        }],
    ];

    for runs in invalid_runs {
        let mut app = AppState::default();
        app.apply(AppEvent::NativeUser(Message::text(
            "prompt",
            "question",
            Some(1),
        )));
        app.apply(AppEvent::NativeFinal(Message::final_text(
            "answer",
            "a界b",
            Some(2),
        )));

        app.apply(AppEvent::FinalPresentation {
            stable_id: "answer".into(),
            text_fingerprint: fingerprint("a界b"),
            presentation: native_presentation("a界b", runs),
        });

        assert_eq!(
            app.turns[0].final_answer.as_ref().unwrap().presentation,
            MessagePresentation::MarkdownFallback
        );
    }
}

#[test]
fn native_final_event_downgrades_invalid_owned_presentation() {
    for presentation in [
        native_presentation("unsafe\u{1b}", Vec::new()),
        native_presentation(
            "short",
            vec![StyleRun {
                start_byte: 0,
                end_byte: 99,
                foreground: Some(AnsiColor::Green),
                background: None,
                modifiers: StyleModifiers::default(),
            }],
        ),
    ] {
        let mut app = AppState::default();
        app.apply(AppEvent::NativeUser(Message::text(
            "prompt",
            "question",
            Some(1),
        )));
        let mut final_answer = Message::final_text("answer", "canonical", Some(2));
        final_answer.presentation = presentation;

        app.apply(AppEvent::NativeFinal(final_answer));

        assert_eq!(
            app.turns[0].final_answer.as_ref().unwrap().presentation,
            MessagePresentation::MarkdownFallback
        );
    }
}

#[test]
fn capture_fallback_never_downgrades_an_existing_native_presentation() {
    let native = native_presentation(
        "answer",
        vec![StyleRun {
            start_byte: 0,
            end_byte: "answer".len(),
            foreground: Some(AnsiColor::Green),
            background: None,
            modifiers: StyleModifiers::default(),
        }],
    );
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "prompt",
        "question",
        Some(1),
    )));
    app.apply(AppEvent::NativeFinal(Message::final_text(
        "final",
        "answer",
        Some(2),
    )));
    app.apply(AppEvent::FinalPresentation {
        stable_id: "final".into(),
        text_fingerprint: fingerprint("answer"),
        presentation: native.clone(),
    });

    app.apply(AppEvent::FinalPresentation {
        stable_id: "final".into(),
        text_fingerprint: fingerprint("answer"),
        presentation: MessagePresentation::MarkdownFallback,
    });

    assert_eq!(
        app.turns[0].final_answer.as_ref().unwrap().presentation,
        native
    );
}

fn hydrated_record(
    role: VisibleRole,
    stable_id: &str,
    turn_id: &str,
    order: u64,
    text: &str,
    presentation: PersistedPresentation,
) -> VisibleHistoryRecord {
    VisibleHistoryRecord {
        version: 2,
        role,
        stable_id: stable_id.into(),
        turn_id: turn_id.into(),
        order,
        text: text.into(),
        attachments: Vec::new(),
        timestamp_ms: Some(order),
        text_fingerprint: fingerprint(text),
        presentation,
        rendered_text: None,
        rendered_text_fingerprint: None,
    }
}

fn hydrated_native_record(
    stable_id: &str,
    turn_id: &str,
    order: u64,
    canonical: &str,
    rendered: &str,
    runs: Vec<StyleRun>,
) -> VisibleHistoryRecord {
    let mut record = hydrated_record(
        VisibleRole::Final,
        stable_id,
        turn_id,
        order,
        canonical,
        PersistedPresentation::NativeAnsi(runs),
    );
    record.rendered_text = Some(rendered.into());
    record.rendered_text_fingerprint = Some(fingerprint(rendered));
    record
}

#[test]
fn hydration_restores_ordered_native_turns_and_saved_presentation() {
    let native = vec![StyleRun {
        start_byte: 0,
        end_byte: 6,
        foreground: Some(AnsiColor::Green),
        background: None,
        modifiers: StyleModifiers::default(),
    }];
    let records = vec![
        hydrated_native_record("a1", "u1", 2, "answer", "answer", native.clone()),
        hydrated_record(
            VisibleRole::Prompt,
            "u1",
            "u1",
            1,
            "question",
            PersistedPresentation::Plain,
        ),
        hydrated_record(
            VisibleRole::Prompt,
            "u2",
            "u2",
            3,
            "later",
            PersistedPresentation::Plain,
        ),
    ];
    let mut app = AppState::default();

    app.hydrate_visible_history(records);

    assert_eq!(app.turns.len(), 2);
    assert_eq!(app.turns[0].prompt.stable_id, "u1");
    assert_eq!(app.turns[0].delivery, Delivery::Native);
    assert_eq!(
        app.turns[0].final_answer.as_ref().unwrap().presentation,
        native_presentation("answer", native)
    );
    assert_eq!(app.turns[1].prompt.stable_id, "u2");
    assert!(app.drain_history_upserts().is_empty());
}

#[test]
fn replay_moves_existing_ids_in_native_order_and_retains_missing_saved_turns() {
    let mut app = AppState::default();
    app.hydrate_visible_history(vec![
        hydrated_record(
            VisibleRole::Prompt,
            "saved-missing",
            "saved-missing",
            1,
            "temporarily unreadable",
            PersistedPresentation::Plain,
        ),
        hydrated_record(
            VisibleRole::Prompt,
            "u2",
            "u2",
            2,
            "old second",
            PersistedPresentation::Plain,
        ),
    ]);

    app.apply(AppEvent::TranscriptReloaded);
    app.apply(AppEvent::NativeUser(Message::text(
        "u2",
        "updated second",
        Some(20),
    )));
    app.apply(AppEvent::NativeUser(Message::text("u3", "third", Some(30))));
    app.apply(AppEvent::TranscriptReplayComplete);

    assert_eq!(
        app.turns
            .iter()
            .map(|turn| turn.prompt.stable_id.as_str())
            .collect::<Vec<_>>(),
        ["u2", "u3", "saved-missing"]
    );
    assert_eq!(app.turns[0].prompt.text, "updated second");
}

#[test]
fn replayed_final_preserves_matching_saved_native_style_but_not_stale_style() {
    let canonical = "**answer** [docs](https://example.test)";
    let rendered = "answer docs";
    let native = vec![StyleRun {
        start_byte: 0,
        end_byte: rendered.len(),
        foreground: Some(AnsiColor::Cyan),
        background: None,
        modifiers: StyleModifiers::default(),
    }];
    let mut app = AppState::default();
    app.hydrate_visible_history(vec![
        hydrated_record(
            VisibleRole::Prompt,
            "u1",
            "u1",
            1,
            "question",
            PersistedPresentation::Plain,
        ),
        hydrated_native_record("a1", "u1", 2, canonical, rendered, native.clone()),
    ]);

    app.apply(AppEvent::TranscriptReloaded);
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "question",
        Some(10),
    )));
    app.apply(AppEvent::NativeFinal(Message::final_text(
        "a1",
        canonical,
        Some(11),
    )));
    assert_eq!(
        app.turns[0].final_answer.as_ref().unwrap().presentation,
        native_presentation(rendered, native)
    );
    assert_eq!(app.turns[0].final_answer.as_ref().unwrap().text, canonical);

    app.apply(AppEvent::TranscriptReloaded);
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "question",
        Some(20),
    )));
    app.apply(AppEvent::NativeFinal(Message::final_text(
        "a1",
        "changed",
        Some(21),
    )));
    assert_eq!(
        app.turns[0].final_answer.as_ref().unwrap().presentation,
        MessagePresentation::MarkdownFallback
    );
}

#[test]
fn native_events_queue_monotonic_upserts_and_style_refresh_reuses_order() {
    let mut app = AppState::default();
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        submission: plain_submission("question"),
        attachments: vec![],
        at_ms: 1,
    });
    assert!(app.drain_history_upserts().is_empty());

    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "question",
        Some(2),
    )));
    app.apply(AppEvent::NativeFinal(Message::final_text(
        "a1",
        "answer",
        Some(3),
    )));
    let first = app.drain_history_upserts();
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].order, 1);
    assert_eq!(first[1].order, 2);
    assert_eq!(first[1].turn_id, "u1");

    app.apply(AppEvent::FinalPresentation {
        stable_id: "a1".into(),
        text_fingerprint: fingerprint("answer"),
        presentation: native_presentation(
            "answer",
            vec![StyleRun {
                start_byte: 0,
                end_byte: 6,
                foreground: Some(AnsiColor::Green),
                background: None,
                modifiers: StyleModifiers::default(),
            }],
        ),
    });
    let styled = app.drain_history_upserts();
    assert_eq!(styled.len(), 1);
    assert_eq!(styled[0].order, 2);
    assert!(matches!(
        styled[0].presentation,
        PersistedPresentation::NativeAnsi(_)
    ));
    assert_eq!(styled[0].version, 2);
    assert_eq!(styled[0].rendered_text.as_deref(), Some("answer"));
    assert_eq!(
        styled[0].rendered_text_fingerprint,
        Some(fingerprint("answer"))
    );
}

#[test]
fn replay_preserves_hydrated_compact_text_without_sidecar_metadata() {
    let hidden = format!(
        "private-log-line\n[Pasted Content · 7 chars]\n{}",
        "more-private-log\n".repeat(100)
    );
    let compact = format!(
        "before\n[Pasted Content · {} chars]\nafter",
        hidden.chars().count()
    );
    let native_text = format!("before\n{hidden}after");
    let mut app = AppState::default();
    app.hydrate_visible_history(vec![hydrated_record(
        VisibleRole::Prompt,
        "u1",
        "u1",
        1,
        &compact,
        PersistedPresentation::Plain,
    )]);

    app.apply(AppEvent::TranscriptReloaded);
    let mut replayed = Message::text("u1", native_text.clone(), Some(99));
    replayed
        .attachments
        .push(herdr_simple_prompts::model::Attachment {
            id: "i1".into(),
            display: "updated.png".into(),
            native_path: None,
        });
    app.apply(AppEvent::NativeUser(replayed));
    app.apply(AppEvent::TranscriptReplayComplete);

    assert_eq!(app.turns[0].prompt.text, compact);
    assert_eq!(app.turns[0].prompt.timestamp_ms, Some(99));
    assert_eq!(app.turns[0].prompt.attachments[0].display, "updated.png");
    let queued = app.drain_history_upserts();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].text, compact);
    assert!(
        !serde_json::to_string(&queued)
            .unwrap()
            .contains("private-log-line")
    );
    assert!(!queued[0].text.contains(&native_text));
}

#[test]
fn partial_replay_keeps_existing_final_with_its_saved_owner() {
    let mut app = AppState::default();
    app.hydrate_visible_history(vec![
        hydrated_record(
            VisibleRole::Prompt,
            "u1",
            "u1",
            1,
            "saved owner",
            PersistedPresentation::Plain,
        ),
        hydrated_record(
            VisibleRole::Final,
            "a1",
            "u1",
            2,
            "saved answer",
            PersistedPresentation::Fallback,
        ),
    ]);

    app.apply(AppEvent::TranscriptReloaded);
    app.apply(AppEvent::NativeUser(Message::text(
        "u2",
        "unrelated open turn",
        Some(10),
    )));
    app.apply(AppEvent::NativeFinal(Message::final_text(
        "a1",
        "updated answer",
        Some(11),
    )));
    app.apply(AppEvent::TranscriptReplayComplete);

    let unrelated = app
        .turns
        .iter()
        .find(|turn| turn.prompt.stable_id == "u2")
        .unwrap();
    assert!(unrelated.final_answer.is_none());
    let owner = app
        .turns
        .iter()
        .find(|turn| turn.prompt.stable_id == "u1")
        .unwrap();
    assert_eq!(owner.final_answer.as_ref().unwrap().stable_id, "a1");
    assert_eq!(owner.final_answer.as_ref().unwrap().text, "updated answer");
}

#[test]
fn composer_access_counts_confirmed_and_pending_plugin_images() {
    let attachment = |id: &str| Attachment {
        id: id.into(),
        display: format!("{id}.png"),
        native_path: None,
    };
    let mut app = AppState {
        native_composer: NativeComposerState::OwnedAttachments(2),
        draft_attachments: vec![attachment("confirmed")],
        pending_attachments: vec![attachment("pending")],
        ..AppState::default()
    };

    assert_eq!(app.composer_access(), ComposerAccess::Ready);

    app.native_composer = NativeComposerState::OwnedAttachments(1);
    assert_eq!(app.composer_access(), ComposerAccess::Occupied);

    app.native_composer = NativeComposerState::Clear;
    app.draft_attachments.clear();
    assert_eq!(app.composer_access(), ComposerAccess::Ready);

    app.pending_attachments.clear();
    assert_eq!(app.composer_access(), ComposerAccess::Ready);
}

#[test]
fn occupied_unknown_and_source_close_are_never_ready() {
    let mut app = AppState {
        native_composer: NativeComposerState::Occupied,
        ..AppState::default()
    };
    assert_eq!(app.composer_access(), ComposerAccess::Occupied);

    app.native_composer = NativeComposerState::Unknown;
    assert_eq!(app.composer_access(), ComposerAccess::Unknown);

    app.native_composer = NativeComposerState::Clear;
    app.source_pane_closed();
    assert_eq!(app.native_composer, NativeComposerState::Unknown);
    assert_eq!(app.composer_access(), ComposerAccess::Unknown);
}

/// A notice is about a moment that has passed. Left standing it says the
/// overlay is still in trouble long after it is not — one was seen holding the
/// error line for three minutes after the removal it complained about.
#[test]
fn a_notice_leaves_the_screen_once_it_has_had_its_time() {
    let mut app = AppState {
        background_error: Some("remove image: the image did not go".into()),
        ..AppState::default()
    };

    app.expire_notice();
    assert_eq!(
        app.visible_error(),
        Some("remove image: the image did not go"),
        "it is there to be read first"
    );

    app.notice_shown = app
        .notice_shown
        .map(|(shown, since)| (shown, since - NOTICE_LINGER));
    app.expire_notice();

    assert_eq!(app.visible_error(), None, "and then it goes on its own");
}

/// A lost connection is not a moment but a state: it goes when it is mended,
/// not when it has been read.
#[test]
fn a_lost_connection_stays_on_screen() {
    let mut app = AppState {
        connection_error: Some("source agent session changed".into()),
        ..AppState::default()
    };

    for _ in 0..2 {
        app.expire_notice();
        app.notice_shown = app
            .notice_shown
            .map(|(shown, since)| (shown, since - NOTICE_LINGER));
    }

    assert_eq!(app.visible_error(), Some("source agent session changed"));
}

/// A second thing going wrong is a second notice, and it gets its own time
/// rather than inheriting what is left of the first one's.
#[test]
fn a_new_notice_starts_its_own_clock() {
    let mut app = AppState {
        background_error: Some("history: no space left on device".into()),
        ..AppState::default()
    };
    app.expire_notice();
    app.notice_shown = app
        .notice_shown
        .map(|(shown, since)| (shown, since - NOTICE_LINGER));

    app.background_error = Some("draft: no space left on device".into());
    app.expire_notice();

    assert_eq!(
        app.visible_error(),
        Some("draft: no space left on device"),
        "the newer line is not swept away by the older one's clock"
    );
}

/// A draft or journal write failure must not be shown as a send failure: the
/// prompt did reach the agent, and a "send failed" line invites a resend.
#[test]
fn background_storage_failures_do_not_masquerade_as_send_failures() {
    let mut app = AppState {
        background_error: Some("history: no space left on device".into()),
        ..AppState::default()
    };

    assert_eq!(
        app.visible_error(),
        Some("history: no space left on device")
    );

    app.send_error = Some("native composer contains unsent input".into());
    assert_eq!(
        app.visible_error(),
        Some("native composer contains unsent input"),
    );
}

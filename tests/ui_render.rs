use herdr_simple_prompts::agent::AgentStatus;
use herdr_simple_prompts::app::{AppEvent, AppState};
use herdr_simple_prompts::editor::Editor;
use herdr_simple_prompts::model::Attachment;
use herdr_simple_prompts::model::Message;
use herdr_simple_prompts::style::StyledText;
use herdr_simple_prompts::style::{AnsiColor, MessagePresentation, StyleModifiers, StyleRun};
use herdr_simple_prompts::ui::render::{render_to_buffer, render_to_string};
use herdr_simple_prompts::ui::visual_rows::{
    CellStyle, HistoryDocument, PromptSection, StickyRows, VisualRow, sticky_overlay, wrap_styled,
};
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use std::ops::Range;
use std::time::{Duration, Instant};

fn rendered_buffer(app: &AppState, width: u16, height: u16) -> Buffer {
    render_to_buffer(app, &Editor::default(), width, height)
}

#[test]
fn prompt_band_and_answer_label_distinguish_roles_without_color_only() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "check dns",
        Some(1),
    )));
    app.apply(AppEvent::NativeFinal(Message::text(
        "a1",
        "zone is pending",
        Some(2),
    )));

    let rendered = render_to_string(&app, &Editor::default(), 50, 14);
    let buffer = rendered_buffer(&app, 50, 14);

    assert!(rendered.contains("YOU  check dns"));
    assert!(rendered.contains("ANSWER"));
    let prompt_row = (0..14)
        .find(|&row| buffer[(0, row)].symbol() == "Y")
        .expect("prompt row should start with YOU");
    let prompt_style = buffer[(0, prompt_row)].style();
    assert_eq!(prompt_style.fg, Some(Color::White));
    assert_eq!(prompt_style.bg, Some(Color::DarkGray));
    assert!(prompt_style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn working_prompt_is_above_composer_and_footer() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "run tests",
        Some(1),
    )));
    app.agent_status = AgentStatus::Working;
    app.working_since = Some(Instant::now() - Duration::from_secs(2));
    let editor = Editor::default();

    let rendered = render_to_string(&app, &editor, 80, 24);

    let prompt = rendered.find("run tests").unwrap();
    let working = rendered.find("Working (2s · esc to interrupt)").unwrap();
    let composer = rendered.find("Write a prompt").unwrap();
    assert!(prompt < working && working < composer);
}

#[test]
fn composer_shows_attached_images_before_submission() {
    let mut app = AppState::default();
    app.draft_attachments.push(Attachment {
        id: "image-1".into(),
        display: "screen.png".into(),
        native_path: None,
    });

    let rendered = render_to_string(&app, &Editor::default(), 80, 24);

    assert!(rendered.contains("[Image #1] screen.png"));
}

#[test]
fn composer_marks_images_as_pending_until_native_verification() {
    let mut app = AppState::default();
    app.pending_attachments.push(Attachment {
        id: "pending-1".into(),
        display: "screen.png".into(),
        native_path: Some("/private/tmp/screen.png".into()),
    });

    let rendered = render_to_string(&app, &Editor::default(), 80, 24);

    assert!(rendered.contains("screen.png (verifying…)"));
}

#[test]
fn only_normalized_messages_reach_the_view() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text("u1", "hello", Some(1))));
    app.apply(AppEvent::NativeFinal(Message::text("a1", "done", Some(2))));

    let rendered = render_to_string(&app, &Editor::default(), 80, 24);

    assert!(rendered.contains("hello"));
    assert!(rendered.contains("done"));
    assert!(!rendered.contains("tool_call"));
    assert!(!rendered.contains("reasoning"));
}

#[test]
fn native_ansi_white_brightness_matches_ratatui_cells() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text("u1", "colors", Some(1))));
    app.apply(AppEvent::NativeFinal(Message {
        stable_id: "a1".into(),
        text: "ab".into(),
        presentation: MessagePresentation::NativeAnsi(vec![
            style_run(0..1, Some(AnsiColor::White), Some(AnsiColor::BrightWhite)),
            style_run(1..2, Some(AnsiColor::BrightWhite), Some(AnsiColor::White)),
        ]),
        attachments: Vec::new(),
        timestamp_ms: Some(2),
    }));

    let buffer = rendered_buffer(&app, 40, 14);
    let a = find_cell(&buffer, 40, 14, "a");
    let b = find_cell(&buffer, 40, 14, "b");
    assert_eq!(buffer[a].style().fg, Some(Color::Gray));
    assert_eq!(buffer[a].style().bg, Some(Color::White));
    assert_eq!(buffer[b].style().fg, Some(Color::White));
    assert_eq!(buffer[b].style().bg, Some(Color::Gray));
}

fn style_run(
    range: Range<usize>,
    foreground: Option<AnsiColor>,
    background: Option<AnsiColor>,
) -> StyleRun {
    StyleRun {
        start_byte: range.start,
        end_byte: range.end,
        foreground,
        background,
        modifiers: StyleModifiers::default(),
    }
}

fn find_cell(buffer: &Buffer, width: u16, height: u16, symbol: &str) -> (u16, u16) {
    for y in 0..height {
        for x in 0..width {
            if buffer[(x, y)].symbol() == symbol {
                return (x, y);
            }
        }
    }
    panic!("expected {symbol:?} in rendered buffer");
}

#[test]
fn history_starts_at_the_bottom_and_page_up_moves_toward_older_turns() {
    let mut app = AppState::default();
    for index in 0..20 {
        app.apply(AppEvent::NativeUser(Message::text(
            format!("u{index}"),
            format!("prompt {index}"),
            Some(index),
        )));
        app.apply(AppEvent::NativeFinal(Message::text(
            format!("a{index}"),
            format!("answer {index}"),
            Some(index),
        )));
    }

    let newest = render_to_string(&app, &Editor::default(), 50, 12);
    assert!(newest.contains("prompt 19"));
    assert!(!newest.contains("prompt 0"));

    app.scroll_from_bottom = usize::MAX;
    let oldest = render_to_string(&app, &Editor::default(), 50, 12);
    assert!(oldest.contains("prompt 0"));
    assert!(!oldest.contains("prompt 19"));
}

#[test]
fn narrow_multiword_answer_scrolls_to_its_real_last_visual_row() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "question",
        Some(1),
    )));
    app.apply(AppEvent::NativeFinal(Message::final_text(
        "a1",
        "one two three four five six seven eight nine ten eleven twelve",
        Some(2),
    )));

    let rendered = render_to_string(&app, &Editor::default(), 18, 8);
    assert!(rendered.contains("twelve"));
}

#[test]
fn wrapped_prompt_rows_fill_the_full_band_background() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "a deliberately long prompt that wraps onto continuation rows",
        Some(1),
    )));

    let width = 22;
    let height = 12;
    let buffer = rendered_buffer(&app, width, height);
    let first = (0..height)
        .find(|&row| buffer[(0, row)].symbol() == "Y")
        .expect("prompt row should be visible");
    assert_eq!(buffer[(width - 1, first)].style().bg, Some(Color::DarkGray));
    assert_eq!(
        buffer[(width - 1, first + 1)].style().bg,
        Some(Color::DarkGray)
    );
}

#[test]
fn wrapped_prompt_continuations_keep_the_prefix_width_indent() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "one two three four five six",
        Some(1),
    )));

    let document = HistoryDocument::from_app(&app, 14);
    assert!(document.rows[0].plain_text().starts_with("YOU  "));
    assert_eq!(&document.rows[1].plain_text()[..5], "     ");
}

#[test]
fn visual_rows_preserve_unicode_width_and_style_runs() {
    let bold = CellStyle {
        modifiers: herdr_simple_prompts::style::StyleModifiers {
            bold: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let rows = wrap_styled(
        &herdr_simple_prompts::style::StyledText {
            text: "界界a\nnext".into(),
            runs: vec![herdr_simple_prompts::style::StyleRun {
                start_byte: 0,
                end_byte: "界界".len(),
                foreground: None,
                background: None,
                modifiers: bold.modifiers,
            }],
        },
        4,
    );

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].plain_text(), "界界");
    assert_eq!(rows[0].cell_width(), 4);
    assert_eq!(rows[0].spans[0].style, bold);
    assert_eq!(rows[1].plain_text(), "a");
    assert_eq!(rows[2].plain_text(), "next");
}

#[test]
fn sticky_overlay_only_pins_after_prompt_leaves_natural_view() {
    let sections = [PromptSection {
        start_row: 0,
        prompt_rows: 2,
        end_row: 8,
    }];
    assert_eq!(sticky_overlay(&sections, 0, 4), None);
    assert_eq!(
        sticky_overlay(&sections, 1, 4),
        Some(StickyRows {
            source_start: 0,
            screen_start: 0,
            count: 2,
        })
    );
}

#[test]
fn later_prompt_pushes_sticky_copy_off_one_row_at_a_time() {
    let sections = [
        PromptSection {
            start_row: 0,
            prompt_rows: 4,
            end_row: 10,
        },
        PromptSection {
            start_row: 10,
            prompt_rows: 1,
            end_row: 14,
        },
    ];
    assert_eq!(
        sticky_overlay(&sections, 8, 4),
        Some(StickyRows {
            source_start: 0,
            screen_start: 0,
            count: 2,
        })
    );
    assert_eq!(
        sticky_overlay(&sections, 9, 4),
        Some(StickyRows {
            source_start: 1,
            screen_start: 0,
            count: 1,
        })
    );
    assert_eq!(sticky_overlay(&sections, 10, 4), None);
}

#[test]
fn generated_document_rows_above_u16_max_keep_manual_viewport_and_sticky_rows() {
    let mut document = HistoryDocument {
        rows: (0..70_010)
            .map(|index| VisualRow::plain(format!("row {index}")))
            .collect(),
        prompts: vec![PromptSection {
            start_row: 70_000,
            prompt_rows: 2,
            end_row: 70_006,
        }],
    };
    document.rows[70_000] = VisualRow::plain("prompt first");
    document.rows[70_001] = VisualRow::plain("prompt second");

    assert_eq!(document.viewport(3, 0)[0].plain_text(), "row 70007");
    assert_eq!(
        document
            .viewport(3, 4)
            .iter()
            .map(VisualRow::plain_text)
            .collect::<Vec<_>>(),
        ["prompt first", "prompt second", "row 70005"]
    );
    assert_eq!(
        sticky_overlay(&document.prompts, 70_003, 3),
        Some(StickyRows {
            source_start: 70_000,
            screen_start: 0,
            count: 2,
        })
    );
}

#[test]
fn wrapper_preserves_many_adjacent_and_gapped_runs_across_wrapped_unicode() {
    let rows = wrap_styled(
        &herdr_simple_prompts::style::StyledText {
            text: "界a\n b界c".into(),
            runs: vec![
                style_run(0..3, Some(AnsiColor::Red), None),
                style_run(3..4, Some(AnsiColor::Green), None),
                style_run(6..7, Some(AnsiColor::Blue), None),
                style_run(7..10, Some(AnsiColor::Yellow), None),
                style_run(10..11, Some(AnsiColor::Magenta), None),
            ],
        },
        3,
    );

    let flattened = rows
        .iter()
        .flat_map(|row| {
            row.spans.iter().flat_map(|span| {
                span.text
                    .chars()
                    .map(move |character| (character, span.style))
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        flattened,
        vec![
            (
                '界',
                CellStyle {
                    foreground: Some(AnsiColor::Red),
                    ..Default::default()
                },
            ),
            (
                'a',
                CellStyle {
                    foreground: Some(AnsiColor::Green),
                    ..Default::default()
                },
            ),
            (' ', CellStyle::default()),
            (
                'b',
                CellStyle {
                    foreground: Some(AnsiColor::Blue),
                    ..Default::default()
                },
            ),
            (
                '界',
                CellStyle {
                    foreground: Some(AnsiColor::Yellow),
                    ..Default::default()
                },
            ),
            (
                'c',
                CellStyle {
                    foreground: Some(AnsiColor::Magenta),
                    ..Default::default()
                },
            ),
        ]
    );
}

#[test]
fn sticky_one_row_prompt_pins_one_row_and_short_viewports_keep_natural_content() {
    let sections = [PromptSection {
        start_row: 0,
        prompt_rows: 1,
        end_row: 5,
    }];
    assert_eq!(sticky_overlay(&sections, 1, 3).unwrap().count, 1);
    assert_eq!(sticky_overlay(&sections, 1, 1), None);
    assert!(sticky_overlay(&sections, 1, 2).unwrap().count < 2);

    let document = HistoryDocument {
        rows: (0..5)
            .map(|index| VisualRow::plain(format!("row {index}")))
            .collect(),
        prompts: sections.to_vec(),
    };
    for height in 1..=3 {
        let viewport = document.viewport(height, 3);
        assert_eq!(viewport.len(), height.min(document.rows.len()));
        assert!(
            viewport
                .iter()
                .any(|row| row.plain_text().starts_with("row"))
        );
    }
}

#[test]
fn wrapper_handles_cjk_and_combining_marks() {
    let rows = wrap_styled(
        &herdr_simple_prompts::style::StyledText {
            text: "界e\u{301}界".into(),
            ..Default::default()
        },
        3,
    );
    assert_eq!(
        rows.iter().map(VisualRow::plain_text).collect::<Vec<_>>(),
        ["界e\u{301}", "界"]
    );
}

#[test]
fn image_only_prompt_is_available_as_sticky_context() {
    let mut app = AppState::default();
    let mut image = Message::text("u1", "", Some(1));
    image.attachments.push(Attachment {
        id: "image-1".into(),
        display: "diagram.png".into(),
        native_path: None,
    });
    app.apply(AppEvent::NativeUser(image));
    app.apply(AppEvent::NativeFinal(Message::final_text(
        "a1",
        "one two three four five six seven eight nine ten eleven twelve",
        Some(2),
    )));

    let rendered = render_to_string(&app, &Editor::default(), 20, 9);
    assert!(rendered.contains("[Image #1]"));
}

#[test]
fn compact_paste_marker_is_not_reconstructed_in_history() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "[Pasted Content · 1000 chars]",
        Some(1),
    )));
    let rendered = render_to_string(&app, &Editor::default(), 50, 10);
    assert!(rendered.contains("[Pasted Content · 1000 chars]"));
}

#[test]
fn bottom_offset_uses_document_rows_without_second_wrapping() {
    let document = HistoryDocument {
        rows: (0..8)
            .map(|index| VisualRow::plain(format!("row {index}")))
            .collect(),
        prompts: Vec::new(),
    };
    assert_eq!(document.viewport(3, 0)[0].plain_text(), "row 5");
    assert_eq!(document.viewport(3, 2)[0].plain_text(), "row 3");
}

#[test]
fn disabled_composer_explains_that_the_source_must_be_reopened() {
    let app = AppState {
        input_enabled: false,
        connection_error: Some("source agent session changed".into()),
        ..AppState::default()
    };

    let rendered = render_to_string(&app, &Editor::default(), 80, 24);

    assert!(rendered.contains("Input disabled"));
    assert!(rendered.contains("source agent session changed"));
}

#[test]
fn composer_shows_large_paste_marker_instead_of_log_body() {
    let app = AppState::default();
    let mut editor = Editor::default();
    editor.insert_char('>');
    editor.insert_paste(&"private-log-line\n".repeat(1_000));
    editor.insert_char('<');

    let rendered = render_to_string(&app, &editor, 80, 24);

    assert!(rendered.contains("Pasted Content"));
    assert!(rendered.contains("chars"));
    assert!(rendered.contains('>'));
    assert!(rendered.contains('<'));
    assert!(!rendered.contains("private-log-line"));
}

#[test]
fn markdown_fallback_body_styles_flow_into_rendered_visual_rows() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "show markdown",
        Some(1),
    )));
    app.apply(AppEvent::NativeFinal(Message::final_text(
        "a1",
        "plain **Ω** and `λ`",
        Some(2),
    )));

    let document = HistoryDocument::from_app(&app, 50);
    let omega = document
        .rows
        .iter()
        .flat_map(|row| &row.spans)
        .find(|span| span.text.contains('Ω'))
        .expect("strong Markdown contents should reach visual rows");
    assert!(omega.style.modifiers.bold);
    let lambda = document
        .rows
        .iter()
        .flat_map(|row| &row.spans)
        .find(|span| span.text.contains('λ'))
        .expect("inline code Markdown contents should reach visual rows");
    assert_eq!(lambda.style.foreground, Some(AnsiColor::White));
    assert_eq!(lambda.style.background, Some(AnsiColor::BrightBlack));
    assert_eq!(
        app.turns[0].final_answer.as_ref().unwrap().presentation,
        MessagePresentation::MarkdownFallback
    );
}

#[test]
fn blocked_view_replaces_history_working_row_and_composer_with_native_surface() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "history must be hidden",
        Some(1),
    )));
    app.agent_status = AgentStatus::Blocked;
    app.working_since = Some(Instant::now() - Duration::from_secs(2));
    app.blocked_surface = Some(Ok(StyledText {
        text: "Allow command?\n  Yes\n  No".into(),
        runs: Vec::new(),
    }));
    let mut editor = Editor::default();
    editor.insert_char('d');
    editor.insert_char('r');
    editor.insert_char('a');
    editor.insert_char('f');
    editor.insert_char('t');

    let rendered = render_to_string(&app, &editor, 80, 16);

    assert!(rendered.contains("INTERACTION REQUIRED"));
    assert!(rendered.contains("Allow command?"));
    assert!(rendered.contains("Native Codex/Claude interaction · prefix+m to return"));
    assert!(!rendered.contains("history must be hidden"));
    assert!(!rendered.contains("Working ("));
    assert!(!rendered.contains("draft"));
}

#[test]
fn blocked_snapshot_styles_are_sanitized_and_confined_to_body() {
    let mut app = AppState {
        agent_status: AgentStatus::Blocked,
        ..AppState::default()
    };
    app.blocked_surface = Some(Ok(herdr_simple_prompts::ansi::sanitize_ansi(
        "\u{1b}]0;rewrite-title\u{7}\u{1b}[31;44mDANGER\u{1b}[0m",
    )));

    let buffer = render_to_buffer(&app, &Editor::default(), 72, 8);
    let rendered = render_to_string(&app, &Editor::default(), 72, 8);
    let header = find_cell(&buffer, 72, 8, "I");
    let danger = (0, 1);
    let footer = (0, 7);

    assert!(!rendered.contains("rewrite-title"));
    assert_eq!(buffer[header].style().fg, Some(Color::Yellow));
    assert!(buffer[header].style().add_modifier.contains(Modifier::BOLD));
    assert_eq!(buffer[danger].style().fg, Some(Color::Red));
    assert_eq!(buffer[danger].style().bg, Some(Color::Blue));
    assert_ne!(buffer[footer].style().bg, Some(Color::Blue));
}

#[test]
fn blocked_snapshot_failure_shows_owned_fallback_and_return_hint() {
    let app = AppState {
        agent_status: AgentStatus::Blocked,
        blocked_surface: Some(Err("socket unavailable".into())),
        ..AppState::default()
    };

    let rendered = render_to_string(&app, &Editor::default(), 80, 8);

    assert!(rendered.contains("Unable to read native interaction"));
    assert!(rendered.contains("prefix+m"));
    assert!(!rendered.contains("socket unavailable"));
}

#[test]
fn leaving_blocked_view_restores_the_exact_ordinary_content() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "ordinary history",
        Some(1),
    )));
    let mut editor = Editor::default();
    editor.insert_paste("unchanged draft");
    let ordinary = render_to_string(&app, &editor, 80, 14);

    app.agent_status = AgentStatus::Blocked;
    app.blocked_surface = Some(Ok(StyledText {
        text: "Choose one".into(),
        runs: Vec::new(),
    }));
    let blocked = render_to_string(&app, &editor, 80, 14);
    app.update_blocked_surface(AgentStatus::Done, None);
    let restored = render_to_string(&app, &editor, 80, 14);

    assert!(blocked.contains("Choose one"));
    assert_eq!(restored, ordinary);
}

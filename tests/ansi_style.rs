use herdr_simple_prompts::ansi::sanitize_ansi;
use herdr_simple_prompts::app::{AppEvent, AppState};
use herdr_simple_prompts::model::Message;
use herdr_simple_prompts::style::{
    AnsiColor, MessagePresentation, StyleModifiers, StyleRun, validate_style_runs,
};

#[test]
fn style_ranges_require_ordered_utf8_boundaries_inside_canonical_text() {
    let text = "a界b";
    let valid = vec![StyleRun {
        start_byte: 1,
        end_byte: 4,
        foreground: Some(AnsiColor::Indexed(45)),
        background: None,
        modifiers: StyleModifiers::default(),
    }];
    assert!(validate_style_runs(text, &valid).is_ok());

    let split_scalar = vec![StyleRun {
        end_byte: 2,
        ..valid[0].clone()
    }];
    assert!(validate_style_runs(text, &split_scalar).is_err());
    let overlap = vec![
        valid[0].clone(),
        StyleRun {
            start_byte: 3,
            ..valid[0].clone()
        },
    ];
    assert!(validate_style_runs(text, &overlap).is_err());
}

#[test]
fn fallback_and_native_provenance_are_not_confused() {
    assert_ne!(
        MessagePresentation::MarkdownFallback,
        MessagePresentation::NativeAnsi(vec![])
    );
}

#[test]
fn native_final_plain_messages_are_normalized_to_markdown_fallback() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "prompt",
        "question",
        Some(1),
    )));
    app.apply(AppEvent::NativeFinal(Message::text(
        "answer",
        "answer",
        Some(2),
    )));

    assert_eq!(
        app.turns[0].final_answer.as_ref().unwrap().presentation,
        MessagePresentation::MarkdownFallback
    );
}

#[test]
fn sanitizer_keeps_safe_sgr_and_discards_terminal_controls() {
    let input = concat!(
        "plain ",
        "\x1b[1;38;2;10;20;30mRGB\x1b[22;39m ",
        "\x1b[48;5;236;3;4mstyled\x1b[0m",
        "\x1b]0;stolen title\x07",
        "\x1b]52;c;Y2xpcGJvYXJk\x07",
        "\x1b[2J\x1b[H",
    );
    let styled = sanitize_ansi(input);
    assert_eq!(styled.text, "plain RGB styled");
    assert_eq!(styled.runs.len(), 2);
    assert!(styled.runs[0].modifiers.bold);
    assert_eq!(styled.runs[0].foreground, Some(AnsiColor::Rgb(10, 20, 30)));
    assert_eq!(styled.runs[1].background, Some(AnsiColor::Indexed(236)));
    assert!(styled.runs[1].modifiers.italic);
    assert!(styled.runs[1].modifiers.underline);
    assert!(!styled.text.contains("stolen title"));
    assert!(!styled.text.contains("clipboard"));
    assert!(!styled.text.contains('\u{1b}'));
}

#[test]
fn sanitizer_supports_named_colors_and_individual_resets() {
    let styled = sanitize_ansi("\x1b[31;44;1;2;3;4mA\x1b[22;23;24;39;49mB\x1b[91;104mC\x1b[0m");
    assert_eq!(styled.text, "ABC");
    assert_eq!(styled.runs.len(), 2);
    assert_eq!(styled.runs[0].foreground, Some(AnsiColor::Red));
    assert_eq!(styled.runs[0].background, Some(AnsiColor::Blue));
    assert_eq!(
        styled.runs[0].modifiers,
        StyleModifiers {
            bold: true,
            dim: true,
            italic: true,
            underline: true,
        }
    );
    assert_eq!(styled.runs[1].foreground, Some(AnsiColor::BrightRed));
    assert_eq!(styled.runs[1].background, Some(AnsiColor::BrightBlue));
    assert_eq!(styled.runs[1].start_byte, 2);
    assert_eq!(styled.runs[1].end_byte, 3);
}

#[test]
fn sanitizer_normalizes_lines_discards_controls_and_coalesces_equal_styles() {
    let styled = sanitize_ansi("a\r\nb\rc\n\x01\x7fd\x1b[31mx\x1b[31my\x1b[0m");
    assert_eq!(styled.text, "a\nb\nc\ndxy");
    assert_eq!(styled.runs.len(), 1);
    assert_eq!(styled.runs[0].foreground, Some(AnsiColor::Red));
    assert_eq!(
        &styled.text[styled.runs[0].start_byte..styled.runs[0].end_byte],
        "xy"
    );
}

#[test]
fn sanitizer_discards_unicode_controls_but_keeps_printable_unicode_and_newlines() {
    let styled = sanitize_ansi("界\u{0085}✓\u{009b}é\u{009f}\r\nnext");

    assert_eq!(styled.text, "界✓é\nnext");
    assert!(
        styled
            .text
            .chars()
            .all(|character| character == '\n' || !character.is_control())
    );
    assert!(styled.text.contains('界'));
    assert!(styled.text.contains('✓'));
    assert!(styled.text.contains('é'));
}

#[test]
fn sanitizer_discards_private_and_string_controls_through_bel_st_or_eof() {
    let styled = sanitize_ansi(concat!(
        "safe",
        "\x1b[?25l",
        "\x1bPdiscarded dcs\x1b\\",
        "\x1b_discarded apc\x1b\\",
        "\x1b^discarded pm\x1b\\",
        "\x1b]title\x1b\\",
        "\x1b]truncated",
        "\x1b[31",
    ));
    assert_eq!(styled.text, "safe");
    assert!(styled.runs.is_empty());
}

#[test]
fn sanitizer_discards_single_escape_commands_and_malformed_sgr() {
    let styled = sanitize_ansi("a\x1b7b\x1b8c\x1b(0d\x1b[38;2;1;2me\x1b[mf");
    assert_eq!(styled.text, "abcdef");
    assert!(styled.runs.is_empty());
}

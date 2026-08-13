use herdr_simple_prompts::agent::AgentKind;
use herdr_simple_prompts::ansi::{extract_native_final, sanitize_ansi};
use herdr_simple_prompts::app::{AppEvent, AppState};
use herdr_simple_prompts::markdown::style_markdown;
use herdr_simple_prompts::model::Message;
use herdr_simple_prompts::style::{
    AnsiColor, MessagePresentation, StyleModifiers, StyleRun, StyledText, validate_style_runs,
    validate_styled_text,
};

fn style_at(styled: &StyledText, byte: usize) -> Option<&StyleRun> {
    styled
        .runs
        .iter()
        .find(|run| run.start_byte <= byte && byte < run.end_byte)
}

#[test]
fn style_ranges_require_ordered_utf8_boundaries_inside_styled_text() {
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
fn styled_text_validation_rejects_controls_and_invalid_rendered_ranges() {
    let valid = StyledText {
        text: "safe\n界".into(),
        runs: vec![StyleRun {
            start_byte: "safe\n".len(),
            end_byte: "safe\n界".len(),
            foreground: Some(AnsiColor::Green),
            background: None,
            modifiers: StyleModifiers::default(),
        }],
    };
    assert!(validate_styled_text(&valid).is_ok());

    let control = StyledText {
        text: "unsafe\u{1b}".into(),
        runs: Vec::new(),
    };
    assert!(validate_styled_text(&control).is_err());

    let beyond_rendered = StyledText {
        text: "short".into(),
        runs: vec![StyleRun {
            start_byte: 0,
            end_byte: 99,
            foreground: Some(AnsiColor::Green),
            background: None,
            modifiers: StyleModifiers::default(),
        }],
    };
    assert!(validate_styled_text(&beyond_rendered).is_err());

    let split_scalar = StyledText {
        text: "a界b".into(),
        runs: vec![StyleRun {
            start_byte: 1,
            end_byte: 2,
            foreground: Some(AnsiColor::Green),
            background: None,
            modifiers: StyleModifiers::default(),
        }],
    };
    assert!(validate_styled_text(&split_scalar).is_err());
}

#[test]
fn fallback_and_native_provenance_are_not_confused() {
    assert_ne!(
        MessagePresentation::MarkdownFallback,
        MessagePresentation::NativeAnsi(StyledText::default())
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

#[test]
fn sanitizer_never_splits_a_multibyte_scalar_after_an_unknown_escape() {
    let styled = sanitize_ansi("before\x1bé界after");

    assert_eq!(styled.text, "beforeé界after");
    assert!(styled.runs.is_empty());
}

#[test]
fn markdown_projection_removes_supported_delimiters_and_rebases_styles() {
    let text = concat!(
        "paragraph with `inline` code\n",
        "# Heading\n",
        "## Subheading\n",
        "- list with **bold** text\n",
        "1. numbered with _italic_ text\n",
        "[label](https://example.test)\n",
        "```rust\n",
        "let x = **plain inside fence**;\n",
        "```\n",
        "malformed **open and _open and `open and [label](\n",
    );
    let expected = concat!(
        "paragraph with inline code\n",
        "Heading\n",
        "Subheading\n",
        "- list with bold text\n",
        "1. numbered with italic text\n",
        "label\n",
        "let x = **plain inside fence**;\n",
        "malformed **open and _open and `open and [label](\n",
    );
    let message = Message::final_text("answer", text, Some(1));

    let styled = style_markdown(&message.text);

    assert_eq!(styled.text, expected);
    assert!(validate_style_runs(&styled.text, &styled.runs).is_ok());
    assert!(styled.runs.windows(2).all(|runs| {
        runs[0].end_byte < runs[1].start_byte
            || runs[0].foreground != runs[1].foreground
            || runs[0].background != runs[1].background
            || runs[0].modifiers != runs[1].modifiers
    }));

    let cases = [
        ("heading", "Heading", None, None, true, false, false),
        ("subheading", "Subheading", None, None, true, false, false),
        ("strong", "bold", None, None, true, false, false),
        ("emphasis", "italic", None, None, false, true, false),
        (
            "inline code",
            "inline",
            Some(AnsiColor::White),
            Some(AnsiColor::BrightBlack),
            false,
            false,
            false,
        ),
        (
            "fenced code",
            "let x",
            Some(AnsiColor::White),
            Some(AnsiColor::BrightBlack),
            false,
            false,
            false,
        ),
        (
            "link label",
            "label",
            Some(AnsiColor::Cyan),
            None,
            false,
            false,
            true,
        ),
    ];
    for (name, needle, foreground, background, bold, italic, underline) in cases {
        let byte = styled.text.find(needle).unwrap();
        let style = style_at(&styled, byte).unwrap_or_else(|| panic!("missing {name} style"));
        assert_eq!(style.foreground, foreground, "{name} foreground");
        assert_eq!(style.background, background, "{name} background");
        assert_eq!(style.modifiers.bold, bold, "{name} bold");
        assert_eq!(style.modifiers.italic, italic, "{name} italic");
        assert_eq!(style.modifiers.underline, underline, "{name} underline");
    }

    assert!(!styled.text.contains("https://example.test"));
    assert!(!styled.text.contains("```"));
    let malformed = styled.text.find("**open").unwrap();
    assert!(style_at(&styled, malformed).is_none());
    assert_eq!(message.text, text);
    assert_eq!(message.presentation, MessagePresentation::MarkdownFallback);
}

#[test]
fn markdown_projection_keeps_malformed_constructs_literal() {
    let text = concat!(
        "unclosed **strong\n",
        "unclosed _emphasis\n",
        "unclosed `code\n",
        "[label](not valid)\n",
        "```rust\n",
        "**literal inside unclosed fence**\n",
    );

    let styled = style_markdown(text);

    assert_eq!(styled.text, text);
    assert!(validate_style_runs(&styled.text, &styled.runs).is_ok());
    assert!(styled.runs.is_empty());
}

#[test]
fn markdown_projection_rebases_style_runs_after_removed_unicode_adjacent_markup() {
    let text = "# Привет **мир** — [документация](https://example.test)";
    let styled = style_markdown(text);

    assert_eq!(styled.text, "Привет мир — документация");
    assert!(validate_style_runs(&styled.text, &styled.runs).is_ok());

    let world = styled.text.find("мир").unwrap();
    let world_style = style_at(&styled, world).unwrap();
    assert!(world_style.modifiers.bold);

    let link = styled.text.find("документация").unwrap();
    let link_style = style_at(&styled, link).unwrap();
    assert_eq!(link_style.foreground, Some(AnsiColor::Cyan));
    assert!(link_style.modifiers.underline);
}

#[test]
fn markdown_inline_constructs_never_cross_newlines_and_precedence_is_deterministic() {
    let text = concat!(
        "`code **not bold** _not italic_ [not link](url)`\n",
        "**strong _does not override_**\n",
        "**crosses\nline** _also\nplain_\n",
        "```\n",
        "`fenced` **still code**\n",
        "```\n",
    );
    let expected = concat!(
        "code **not bold** _not italic_ [not link](url)\n",
        "strong _does not override_\n",
        "**crosses\nline** _also\nplain_\n",
        "`fenced` **still code**\n",
    );

    let styled = style_markdown(text);

    assert_eq!(styled.text, expected);
    assert!(validate_style_runs(&styled.text, &styled.runs).is_ok());
    let code_bold_words = styled.text.find("not bold").unwrap();
    let code_style = style_at(&styled, code_bold_words).unwrap();
    assert_eq!(code_style.background, Some(AnsiColor::BrightBlack));
    assert!(!code_style.modifiers.bold);
    assert!(!code_style.modifiers.italic);
    assert!(!code_style.modifiers.underline);

    let strong_inner = styled.text.find("does not override").unwrap();
    let strong_style = style_at(&styled, strong_inner).unwrap();
    assert!(strong_style.modifiers.bold);
    assert!(!strong_style.modifiers.italic);

    for malformed in ["crosses", "line", "also", "plain"] {
        let byte = styled.text.find(malformed).unwrap();
        assert!(
            style_at(&styled, byte).is_none(),
            "{malformed} stayed plain"
        );
    }

    let fenced_inline = styled.text.rfind("still code").unwrap();
    let fenced_style = style_at(&styled, fenced_inline).unwrap();
    assert_eq!(fenced_style.background, Some(AnsiColor::BrightBlack));
    assert!(!fenced_style.modifiers.bold);
}

#[test]
fn markdown_links_recover_after_nested_or_invalid_candidates() {
    let text = concat!(
        "[broken then [label](https://example.test)\n",
        "[bad]([nested](https://nested.test)\n",
        "[bad whitespace](before [deep](https://deep.test)\n",
        "[](empty-label) then [second](https://second.test)\n",
        "[empty-url]() then [third](https://third.test)\n",
        "[bad-url](not valid) then [fourth](https://fourth.test)\n",
    );
    let expected = concat!(
        "[broken then label\n",
        "[bad](nested\n",
        "[bad whitespace](before deep\n",
        "[](empty-label) then second\n",
        "[empty-url]() then third\n",
        "[bad-url](not valid) then fourth\n",
    );

    let styled = style_markdown(text);

    assert_eq!(styled.text, expected);
    assert!(validate_style_runs(&styled.text, &styled.runs).is_ok());
    assert!(style_at(&styled, styled.text.find("broken then").unwrap()).is_none());
    for label in ["label", "nested", "deep", "second", "third", "fourth"] {
        let label_style = style_at(&styled, styled.text.find(label).unwrap()).unwrap();
        assert_eq!(label_style.foreground, Some(AnsiColor::Cyan));
        assert!(label_style.modifiers.underline);
    }
    for invalid in [
        "bad",
        "bad whitespace",
        "before",
        "empty-label",
        "empty-url",
        "bad-url",
        "not valid",
    ] {
        assert!(
            style_at(&styled, styled.text.find(invalid).unwrap()).is_none(),
            "{invalid} stayed literal and plain"
        );
    }
    assert!(!styled.text.contains("https://"));
}

#[test]
fn markdown_valid_link_owns_inline_like_destination_syntax() {
    let styled = style_markdown("[label](a`code`b)");

    assert_eq!(styled.text, "label");
    assert!(validate_style_runs(&styled.text, &styled.runs).is_ok());
    let label_style = style_at(&styled, 0).unwrap();
    assert_eq!(label_style.foreground, Some(AnsiColor::Cyan));
    assert!(label_style.modifiers.underline);
}

#[test]
fn markdown_inline_code_outranks_a_valid_link_label() {
    let styled = style_markdown("[left `code` right](url)");

    assert_eq!(styled.text, "left code right");
    assert!(validate_style_runs(&styled.text, &styled.runs).is_ok());

    for label_part in ["left", "right"] {
        let label_style = style_at(&styled, styled.text.find(label_part).unwrap()).unwrap();
        assert_eq!(label_style.foreground, Some(AnsiColor::Cyan));
        assert!(label_style.modifiers.underline);
    }

    let code_style = style_at(&styled, styled.text.find("code").unwrap()).unwrap();
    assert_eq!(code_style.foreground, Some(AnsiColor::White));
    assert_eq!(code_style.background, Some(AnsiColor::BrightBlack));
    assert!(!code_style.modifiers.underline);
}

#[test]
fn markdown_malformed_link_candidate_stays_atomically_literal() {
    let text = "[**label**](bad url)";
    let styled = style_markdown(text);

    assert_eq!(styled.text, text);
    assert!(validate_style_runs(&styled.text, &styled.runs).is_ok());
    assert!(styled.runs.is_empty());
}

#[test]
fn exact_codex_final_capture_removes_only_known_chrome_and_preserves_styles() {
    let ansi = concat!(
        "tool output\n",
        "────────\n",
        "\u{1b}[32m• Final heading\u{1b}[0m\n",
        "  body\n",
        "────────\n",
        "› Write a prompt",
    );

    let captured = extract_native_final(ansi, "Final heading\nbody", AgentKind::Codex).unwrap();

    assert_eq!(captured.text, "Final heading\nbody");
    assert_eq!(captured.runs[0].foreground, Some(AnsiColor::Green));
    assert_eq!(
        &captured.text[captured.runs[0].start_byte..captured.runs[0].end_byte],
        "Final heading"
    );
    assert!(validate_style_runs(&captured.text, &captured.runs).is_ok());
}

#[test]
fn native_final_capture_matches_projected_visible_text_and_keeps_native_styles() {
    let canonical = "# Final **heading**\nUse [docs](https://example.test) and `cargo test`.";
    let projected = style_markdown(canonical);
    assert_eq!(projected.text, "Final heading\nUse docs and cargo test.");
    let ansi = concat!(
        "tool output\n",
        "────────\n",
        "\u{1b}[1;36m• Final heading\u{1b}[0m\n",
        "  Use \u{1b}[4;34mdocs\u{1b}[0m and \u{1b}[30;47mcargo test\u{1b}[0m.\n",
        "────────\n",
        "› Write a prompt",
    );

    let captured = extract_native_final(ansi, &projected.text, AgentKind::Codex).unwrap();

    assert_eq!(captured.text, projected.text);
    assert!(!captured.text.contains("https://example.test"));
    assert!(!captured.text.contains('`'));
    assert!(validate_styled_text(&captured).is_ok());

    let heading = style_at(&captured, captured.text.find("Final heading").unwrap()).unwrap();
    assert_eq!(heading.foreground, Some(AnsiColor::Cyan));
    assert!(heading.modifiers.bold);
    let docs = style_at(&captured, captured.text.find("docs").unwrap()).unwrap();
    assert_eq!(docs.foreground, Some(AnsiColor::Blue));
    assert!(docs.modifiers.underline);
    let code = style_at(&captured, captured.text.find("cargo test").unwrap()).unwrap();
    assert_eq!(code.foreground, Some(AnsiColor::Black));
    assert_eq!(code.background, Some(AnsiColor::White));
}

#[test]
fn native_final_capture_ignores_physical_wraps_without_a_leading_separator() {
    let expected = "Use herdr agent list |\njq '.result.agents[]'";
    let ansi = concat!(
        "earlier output\n",
        "\u{1b}[36m• Use herdr agent \u{1b}[0m\n",
        "  \u{1b}[36mlist |\u{1b}[0m\n",
        "  \u{1b}[33mjq '.result.agents[]'\u{1b}[0m\n",
        "─ Worked for 2s ────────\n",
        "› Write a prompt",
    );

    let captured = extract_native_final(ansi, expected, AgentKind::Codex).unwrap();

    assert_eq!(captured.text, expected);
    assert!(validate_styled_text(&captured).is_ok());
    assert_eq!(
        style_at(&captured, captured.text.find("herdr").unwrap())
            .unwrap()
            .foreground,
        Some(AnsiColor::Cyan),
    );
    assert_eq!(
        style_at(&captured, captured.text.find("jq").unwrap())
            .unwrap()
            .foreground,
        Some(AnsiColor::Yellow),
    );
}

#[test]
fn native_final_capture_maps_wrapped_shell_styles_to_projected_markdown() {
    let projected = style_markdown(concat!(
        "Run:\n",
        "```sh\n",
        "herdr agent list |\n",
        "  jq '.result'\n",
        "```",
    ));
    let ansi = concat!(
        "• Run:\n",
        "  \u{1b}[36mherdr agent \u{1b}[0m\n",
        "  \u{1b}[36mlist |\u{1b}[0m\n",
        "  \u{1b}[33mjq '.result'\u{1b}[0m\n",
        "────────\n",
        "› Write a prompt",
    );

    let captured = extract_native_final(ansi, &projected.text, AgentKind::Codex).unwrap();

    assert_eq!(captured.text, projected.text);
    assert_eq!(
        style_at(&captured, captured.text.find("herdr").unwrap())
            .unwrap()
            .foreground,
        Some(AnsiColor::Cyan),
    );
    assert_eq!(
        style_at(&captured, captured.text.find("jq").unwrap())
            .unwrap()
            .foreground,
        Some(AnsiColor::Yellow),
    );
}

#[test]
fn width_independent_native_capture_maps_multibyte_scalars_safely() {
    let expected = "Привет мир\n界 test";
    let ansi = concat!(
        "\u{1b}[36m• Привет \u{1b}[0m\n",
        "  \u{1b}[32mмир\u{1b}[0m\n",
        "  \u{1b}[33m界\u{1b}[0m test\n",
        "────────\n",
        "› Write a prompt",
    );

    let captured = extract_native_final(ansi, expected, AgentKind::Codex).unwrap();

    assert_eq!(captured.text, expected);
    assert!(validate_styled_text(&captured).is_ok());
    assert_eq!(
        style_at(&captured, captured.text.find("Привет").unwrap())
            .unwrap()
            .foreground,
        Some(AnsiColor::Cyan),
    );
    assert_eq!(
        style_at(&captured, captured.text.find("мир").unwrap())
            .unwrap()
            .foreground,
        Some(AnsiColor::Green),
    );
    assert_eq!(
        style_at(&captured, captured.text.find('界').unwrap())
            .unwrap()
            .foreground,
        Some(AnsiColor::Yellow),
    );
}

#[test]
fn width_independent_native_capture_rejects_content_changes_and_duplicates() {
    let changed = "• same answer\n  changed token\n────────\n› Write a prompt";
    assert!(
        extract_native_final(changed, "same answer\nexpected token", AgentKind::Codex,).is_none()
    );

    let duplicate = concat!(
        "• same answer\n────────\n› Write a prompt\n",
        "• same answer\n────────\n› Write a prompt",
    );
    assert!(extract_native_final(duplicate, "same answer", AgentKind::Codex).is_none());
}

#[test]
fn exact_claude_final_capture_uses_claude_boundaries() {
    let ansi = concat!(
        "earlier output\n",
        "────────────────────────────────\n",
        "\u{1b}[1;36m⏺ Final heading\u{1b}[0m\n",
        "  body\n",
        "────────────────────────────────\n",
        "❯ ",
    );

    let captured = extract_native_final(ansi, "Final heading\nbody", AgentKind::Claude).unwrap();

    assert_eq!(captured.text, "Final heading\nbody");
    assert_eq!(captured.runs[0].foreground, Some(AnsiColor::Cyan));
    assert!(captured.runs[0].modifiers.bold);
}

#[test]
fn native_final_capture_rejects_unsafe_or_non_exact_candidates() {
    let canonical = "same answer\nsecond line";
    let unsafe_reads = [
        // User prompt.
        "────────\n› same answer\n  second line\n────────\n› Write a prompt",
        // Commentary / working item, without an accepted final boundary.
        "────────\n• Working (2s)\n  same answer\n  second line\n────────\n› Write a prompt",
        // Tool result.
        "────────\n• Ran command\n  same answer\n  second line\n────────\n› Write a prompt",
        // Native composer contents.
        "────────\n› same answer\n  second line",
        // Text mismatch.
        "────────\n• same answer\n  different line\n────────\n› Write a prompt",
        // Partial scrollback misses the trailing composer boundary.
        "────────\n• same answer\n  second line",
        // Two complete candidates are ambiguous.
        concat!(
            "────────\n• same answer\n  second line\n────────\n› Write a prompt\n",
            "────────\n• same answer\n  second line\n────────\n› Write a prompt",
        ),
    ];

    for ansi in unsafe_reads {
        assert!(
            extract_native_final(ansi, canonical, AgentKind::Codex).is_none(),
            "unsafe read was accepted: {ansi:?}"
        );
    }
}

#[test]
fn canonical_ansi_looking_literals_are_matched_as_text_not_executed_controls() {
    let ansi = "────────\n• literal \\x1b[31m red\n────────\n› Write a prompt";

    let captured = extract_native_final(ansi, "literal \\x1b[31m red", AgentKind::Codex).unwrap();

    assert_eq!(captured.text, "literal \\x1b[31m red");
    assert!(captured.runs.is_empty());
}

#[test]
fn native_final_capture_preserves_blank_lines_and_canonical_indentation() {
    let ansi = concat!(
        "────────\n",
        "\u{1b}[32m• first\u{1b}[0m\n",
        "\n",
        "    indented\n",
        "────────\n",
        "› Write a prompt",
    );

    let captured = extract_native_final(ansi, "first\n\n  indented", AgentKind::Codex).unwrap();

    assert_eq!(captured.text, "first\n\n  indented");
    assert!(validate_style_runs(&captured.text, &captured.runs).is_ok());
}

#[test]
fn native_final_capture_accepts_resized_reviewed_agent_boundaries() {
    for width in [8, 24, 80] {
        let separator = "─".repeat(width);
        let ansi = format!("{separator}\n• answer\n{separator}\n› Write a prompt");
        assert!(extract_native_final(&ansi, "answer", AgentKind::Codex).is_some());
    }
    for width in [16, 32, 96] {
        let separator = "─".repeat(width);
        let ansi = format!("{separator}\n⏺ answer\n{separator}\n❯ ");
        assert!(extract_native_final(&ansi, "answer", AgentKind::Claude).is_some());
    }

    let decorated = concat!(
        "────────────────────────\n",
        "• answer\n",
        "─ Worked for 58m 35s ─────────\n",
        "› Write a prompt",
    );
    assert!(extract_native_final(decorated, "answer", AgentKind::Codex).is_some());

    for unsafe_read in [
        "───────\n• answer\n───────\n› Write a prompt",
        "━━━━━━━━\n• answer\n━━━━━━━━\n› Write a prompt",
        "────────\n• answer\n- Worked for 2s -\n› Write a prompt",
        "────────\n• answer\n─ Worked for eventually ────────\n› Write a prompt",
        "───────────────\n⏺ answer\n───────────────\n❯ ",
    ] {
        assert!(extract_native_final(unsafe_read, "answer", AgentKind::Codex).is_none());
        assert!(extract_native_final(unsafe_read, "answer", AgentKind::Claude).is_none());
    }
}

#[test]
fn native_final_capture_accepts_only_known_optional_agent_footers() {
    let codex = concat!(
        "────────\n",
        "• answer\n",
        "────────\n",
        "› Write a prompt\n",
        "gpt-5.6-sol xhigh · /repo",
    );
    let claude = concat!(
        "────────────────────────────────\n",
        "⏺ answer\n",
        "────────────────────────────────\n",
        "❯ \n",
        "Claude Opus · /repo",
    );

    assert!(extract_native_final(codex, "answer", AgentKind::Codex).is_some());
    assert!(extract_native_final(claude, "answer", AgentKind::Claude).is_some());

    let arbitrary = concat!(
        "────────\n",
        "• answer\n",
        "────────\n",
        "› Write a prompt\n",
        "unreviewed footer",
    );
    assert!(extract_native_final(arbitrary, "answer", AgentKind::Codex).is_none());

    for (kind, unsafe_footer) in [
        (AgentKind::Codex, "gpt-unreviewed payload"),
        (AgentKind::Codex, "gpt-unreviewed · payload"),
        (AgentKind::Claude, "ClaudeInjected · /repo"),
        (AgentKind::Claude, "OpusInjected · /repo"),
    ] {
        let prefix = match kind {
            AgentKind::Codex => "• answer\n────────\n› Write a prompt\n",
            AgentKind::Claude => "⏺ answer\n────────────────────────────────\n❯ \n",
        };
        let ansi = format!("{prefix}{unsafe_footer}");
        assert!(
            extract_native_final(&ansi, "answer", kind).is_none(),
            "unsafe footer was accepted: {unsafe_footer:?}",
        );
    }
}

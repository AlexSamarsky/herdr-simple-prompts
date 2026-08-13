# Native Rendered Presentations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render final answers with the same visible Markdown projection as the native Codex or Claude terminal, while retaining canonical transcript Markdown for identity and persisting exact captured display text with its ANSI-derived styles.

**Architecture:** Keep `Message.text` as canonical transcript Markdown and move every length-changing display value into `MessagePresentation::NativeAnsi(StyledText)`. A dependency-free projector produces the fallback `StyledText` and the exact visible string used for native ANSI matching; journal v2 stores native rendered text and its fingerprint alongside canonical text, while safely loading journal v1.

**Tech Stack:** Rust 1.85+, serde/serde_json, Ratatui, existing ANSI sanitizer and visual-row engine; no new crate or runtime network access.

---

## File map

| File | Responsibility in this change |
|---|---|
| `src/markdown.rs` | Deterministically project supported transcript Markdown into visible text and style runs over that projected text. |
| `src/style.rs` | Let native presentation own a complete `StyledText` and validate rendered text plus its runs. |
| `src/ansi.rs` | Match known Codex/Claude final boundaries against expected projected visible text and return the sanitized native `StyledText`. |
| `src/ui/runtime.rs` | Compute the projection once per final capture, retry exact matching, and emit either captured `StyledText` or Markdown fallback provenance. |
| `src/app.rs` | Validate and reduce native rendered presentations without changing canonical message identity or allowing fallback to downgrade native capture. |
| `src/ui/visual_rows.rs` | Render stored native text directly; compute projection only for fallback finals. |
| `src/history.rs` | Write journal v2 native rendered text/fingerprint and safely restore v2 or migrate v1 records. |
| `tests/ansi_style.rs` | Projector, UTF-8 style rebasing, malformed syntax, and projected native-match contracts. |
| `tests/app_state.rs` | Reducer identity, validation, replay, and journal-upsert contracts for owned rendered text. |
| `tests/ui_render.rs` | Visible output and style parity: no supported Markdown delimiters or link destinations in final answers. |
| `tests/history_journal.rs` | Journal v2 round-trip, integrity checks, and v1 compatibility/downgrade behavior. |
| `src/ui/mod.rs` | Update in-module journal fixtures for the v2 record shape. |
| `README.md` | Explain canonical-versus-visible text, exact projected matching, fallback behavior, and the live smoke case. |
| `docs/superpowers/specs/2026-08-12-herdr-simple-prompts-design.md` | Already updated approved design; verify implementation and test wording still agrees before merge. |

## Task 1: Project supported Markdown into visible styled text

**Files:**
- Modify: `src/markdown.rs`
- Test: `tests/ansi_style.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before changing production code.
- Invoke `superpowers:requesting-code-review` after the projector batch.
- Invoke `superpowers:verification-before-completion` before marking the task complete.

- [ ] **Step 1: Replace the old non-rewriting fallback assertion with a projected-text contract**

In `tests/ansi_style.rs`, rename the existing fallback test to
`markdown_projection_removes_supported_delimiters_and_rebases_styles` and use
the following source and exact visible expectation:

```rust
#[test]
fn markdown_projection_removes_supported_delimiters_and_rebases_styles() {
    let canonical = concat!(
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
    let visible = concat!(
        "paragraph with inline code\n",
        "Heading\n",
        "Subheading\n",
        "- list with bold text\n",
        "1. numbered with italic text\n",
        "label\n",
        "let x = **plain inside fence**;\n",
        "malformed **open and _open and `open and [label](\n",
    );

    let styled = style_markdown(canonical);

    assert_eq!(styled.text, visible);
    assert!(validate_style_runs(&styled.text, &styled.runs).is_ok());
    assert!(!styled.text.contains("https://example.test"));
    assert!(!styled.text.contains("```"));

    let cases = [
        ("Heading", None, None, true, false, false),
        ("Subheading", None, None, true, false, false),
        ("bold", None, None, true, false, false),
        ("italic", None, None, false, true, false),
        (
            "inline",
            Some(AnsiColor::White),
            Some(AnsiColor::BrightBlack),
            false,
            false,
            false,
        ),
        (
            "let x",
            Some(AnsiColor::White),
            Some(AnsiColor::BrightBlack),
            false,
            false,
            false,
        ),
        ("label", Some(AnsiColor::Cyan), None, false, false, true),
    ];
    for (needle, foreground, background, bold, italic, underline) in cases {
        let byte = styled.text.find(needle).unwrap();
        let style = style_at(&styled, byte).unwrap_or_else(|| panic!("missing {needle} style"));
        assert_eq!(style.foreground, foreground, "{needle} foreground");
        assert_eq!(style.background, background, "{needle} background");
        assert_eq!(style.modifiers.bold, bold, "{needle} bold");
        assert_eq!(style.modifiers.italic, italic, "{needle} italic");
        assert_eq!(style.modifiers.underline, underline, "{needle} underline");
    }

    let message = Message::final_text("answer", canonical, Some(1));
    assert_eq!(message.text, canonical);
    assert_eq!(message.presentation, MessagePresentation::MarkdownFallback);
}
```

- [ ] **Step 2: Add malformed-syntax and Unicode rebasing tests**

Keep malformed or unsupported constructs literal, and prove that style ranges
are indexed into projected UTF-8 bytes rather than canonical bytes:

```rust
#[test]
fn markdown_projection_keeps_malformed_constructs_literal() {
    let canonical = concat!(
        "before **unclosed\n",
        "before _unclosed\n",
        "before `unclosed\n",
        "before [label](bad url)\n",
        "```rust\n",
        "unclosed fence\n",
    );

    let styled = style_markdown(canonical);

    assert_eq!(styled.text, canonical);
    assert!(validate_style_runs(&styled.text, &styled.runs).is_ok());
}

#[test]
fn markdown_projection_rebases_style_runs_after_removed_unicode_adjacent_markup() {
    let styled = style_markdown("# Привет **мир** — [документация](https://example.test)");

    assert_eq!(styled.text, "Привет мир — документация");
    assert!(validate_style_runs(&styled.text, &styled.runs).is_ok());
    let world = styled.text.find("мир").unwrap();
    assert!(style_at(&styled, world).unwrap().modifiers.bold);
    let link = styled.text.find("документация").unwrap();
    let link_style = style_at(&styled, link).unwrap();
    assert_eq!(link_style.foreground, Some(AnsiColor::Cyan));
    assert!(link_style.modifiers.underline);
}
```

Update the existing precedence test to find needles in `styled.text`. Its exact
visible output must be:

```rust
assert_eq!(
    styled.text,
    concat!(
        "code **not bold** _not italic_ [not link](url)\n",
        "strong _does not override_\n",
        "**crosses\nline** _also\nplain_\n",
        "`fenced` **still code**\n",
    )
);
```

Update link-recovery expectations so only syntactically valid recovered links
lose their destination and delimiters; invalid candidates remain byte-for-byte
literal. Locate every style assertion through `styled.text.find(...)`.

- [ ] **Step 3: Run the projector tests and observe the intended failure**

Run:

```bash
cargo test --test ansi_style markdown_projection -- --nocapture
```

Expected: the new tests fail because `style_markdown` still returns canonical
text and style offsets over canonical bytes.

- [ ] **Step 4: Implement a projection mask while retaining the existing deterministic style precedence**

In `src/markdown.rs`, retain `StyleSlot` and the existing style helpers, add a
parallel visibility mask, and build output/runs only from visible UTF-8 scalar
starts:

```rust
pub fn style_markdown(text: &str) -> StyledText {
    let mut slots = vec![StyleSlot::default(); text.len()];
    let mut visible = vec![true; text.len()];
    let lines = line_ranges(text);
    let mut line_index = 0;

    while line_index < lines.len() {
        let line = lines[line_index];
        let source = &text[line.start..line.end];
        if is_opening_fence(source)
            && let Some(close_index) = (line_index + 1..lines.len())
                .find(|candidate| is_closing_fence(&text[lines[*candidate].start..lines[*candidate].end]))
        {
            discard(&mut visible, line.start, line.after_end);
            for code_line in &lines[line_index + 1..close_index] {
                apply_style(
                    &mut slots,
                    code_line.start,
                    code_line.end,
                    FENCED_CODE_PRIORITY,
                    code_style(),
                );
            }
            let close = lines[close_index];
            discard(&mut visible, close.start, close.after_end);
            line_index = close_index + 1;
            continue;
        }

        let (content_offset, heading) = block_content(source);
        let content_start = line.start + content_offset;
        if heading {
            discard(&mut visible, line.start, content_start);
            apply_style(
                &mut slots,
                content_start,
                line.end,
                BLOCK_PRIORITY,
                bold_style(),
            );
        }
        style_inline(text, content_start, line.end, &mut slots, &mut visible);
        line_index += 1;
    }

    project_visible(text, &slots, &visible)
}

#[derive(Clone, Copy)]
struct LineRange {
    start: usize,
    end: usize,
    after_end: usize,
}

fn line_ranges(text: &str) -> Vec<LineRange> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (end, _) in text.match_indices('\n') {
        lines.push(LineRange {
            start,
            end,
            after_end: end + 1,
        });
        start = end + 1;
    }
    lines.push(LineRange {
        start,
        end: text.len(),
        after_end: text.len(),
    });
    lines
}

fn discard(visible: &mut [bool], start: usize, end: usize) {
    visible[start..end].fill(false);
}

fn project_visible(text: &str, slots: &[StyleSlot], visible: &[bool]) -> StyledText {
    let mut projected = String::with_capacity(text.len());
    let mut builder = StyleRunBuilder::new();
    for (byte, character) in text.char_indices() {
        if visible[byte] {
            builder.set_style(slots[byte].style, projected.len());
            projected.push(character);
        }
    }
    StyledText {
        runs: builder.finish(projected.len()),
        text: projected,
    }
}
```

Change `style_inline` and each valid inline recognizer to accept
`visible: &mut [bool]`. Only after a closing delimiter and validity checks have
succeeded, mark these exact canonical byte ranges hidden:

```rust
// `inline`
discard(visible, open, open + 1);
discard(visible, close, close + 1);

// **strong**
discard(visible, open, open + 2);
discard(visible, close, close + 2);

// _emphasis_
discard(visible, open, open + 1);
discard(visible, close, close + 1);

// [label](destination)
discard(visible, open, label_start);
discard(visible, label_end, close + 1);
```

Do not hide list prefixes. Do not hide any bytes for an unclosed inline
construct, whitespace-containing/empty URL, empty label, or unclosed fence.
Keep precedence exactly `fenced code > inline code > link label > strong >
emphasis > block` by retaining the existing priority constants and
`apply_style` behavior.

- [ ] **Step 5: Verify the complete Markdown/ANSI test file**

Run:

```bash
cargo fmt --check
cargo test --test ansi_style
cargo clippy --test ansi_style -- -D warnings
```

Expected: all `ansi_style` tests pass, all projected runs validate, malformed
constructs remain literal, and formatting/lint checks pass.

- [ ] **Step 6: Review, verify, and commit Task 1**

Invoke `superpowers:requesting-code-review`, fix every Critical or Important
finding, then invoke `superpowers:verification-before-completion` and rerun the
three commands from Step 5.

Commit:

```bash
git add src/markdown.rs tests/ansi_style.rs
git commit -m "project markdown display text"
```

## Task 2: Capture native ANSI against projected visible text

**Files:**
- Modify: `src/style.rs`
- Modify: `src/ansi.rs`
- Modify: `src/ui/runtime.rs`
- Test: `tests/ansi_style.rs`
- Test: `src/ui/runtime.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before changing production code.
- Invoke `superpowers:requesting-code-review` after the type/capture batch.
- Invoke `superpowers:verification-before-completion` before marking the task complete.

- [ ] **Step 1: Add projected native-match and owned-presentation tests**

Add this exact capture case to `tests/ansi_style.rs`:

```rust
#[test]
fn native_final_capture_matches_projected_visible_text_and_keeps_native_styles() {
    let canonical = "# Final **heading**\nUse [docs](https://example.test) and `cargo test`.";
    let projected = style_markdown(canonical);
    assert_eq!(projected.text, "Final heading\nUse docs and cargo test.");
    let ansi = concat!(
        "────────\n",
        "\u{1b}[1;36m• Final heading\u{1b}[0m\n",
        "  Use \u{1b}[4;34mdocs\u{1b}[0m and \u{1b}[47;30mcargo test\u{1b}[0m.\n",
        "────────\n",
        "› Write a prompt",
    );

    let captured = extract_native_final(ansi, &projected.text, AgentKind::Codex).unwrap();

    assert_eq!(captured.text, projected.text);
    assert!(validate_style_runs(&captured.text, &captured.runs).is_ok());
    assert!(!captured.text.contains("https://example.test"));
    assert!(!captured.text.contains('`'));
    let heading = style_at(&captured, captured.text.find("Final heading").unwrap()).unwrap();
    assert_eq!(heading.foreground, Some(AnsiColor::Cyan));
    assert!(heading.modifiers.bold);
}
```

Update existing `extract_native_final` tests to name their second argument
`expected_visible`; their existing plain-text fixtures remain unchanged.

In the `src/ui/runtime.rs` test module, replace the old `NativeAnsi(runs)`
destructure with an owned `StyledText` assertion and add a canonical Markdown
case:

```rust
#[test]
fn capture_resolution_projects_canonical_markdown_before_exact_match() {
    let (presentation, diagnostic) = resolve_capture(
        AgentKind::Codex,
        "# Final **answer** with [docs](https://example.test)",
        || Ok("────────\n\u{1b}[32m• Final answer with docs\u{1b}[0m\n────────\n› Write a prompt".into()),
        1,
        Duration::ZERO,
    );

    assert!(diagnostic.is_none());
    let MessagePresentation::NativeAnsi(styled) = presentation else {
        panic!("projected exact capture must keep native presentation")
    };
    assert_eq!(styled.text, "Final answer with docs");
    assert_eq!(styled.runs[0].foreground, Some(AnsiColor::Green));
}
```

- [ ] **Step 2: Run the focused tests and observe type/match failures**

Run:

```bash
cargo test --test ansi_style native_final_capture_matches_projected_visible_text_and_keeps_native_styles -- --exact
cargo test ui::runtime::tests::capture_resolution_projects_canonical_markdown_before_exact_match -- --exact
```

Expected: the first test cannot match canonical Markdown to transformed native
text, and the second test fails to compile until native presentation owns
`StyledText`.

- [ ] **Step 3: Make native presentation own rendered text and validate it**

In `src/style.rs`, change the enum and add rendered-data validation:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MessagePresentation {
    Plain,
    NativeAnsi(StyledText),
    MarkdownFallback,
}

pub fn validate_styled_text(styled: &StyledText) -> Result<(), String> {
    if styled
        .text
        .chars()
        .any(|character| character != '\n' && character.is_control())
    {
        return Err("styled text contains a terminal control character".into());
    }
    validate_style_runs(&styled.text, &styled.runs)
}
```

Change `validate_style_runs`' out-of-bounds error wording from `canonical text`
to `styled text`, because the same invariant now applies to fallback and native
display values.

- [ ] **Step 4: Match ANSI against expected visible text and return it unchanged**

In `src/ansi.rs`, replace `extract_native_final` with the complete
visible-text version below. It retains all accepted Codex/Claude boundaries,
unique-candidate checks, footer checks, sanitizer behavior, and
source-to-destination run slicing:

```rust
pub fn extract_native_final(
    ansi: &str,
    expected_visible: &str,
    kind: AgentKind,
) -> Option<StyledText> {
    if expected_visible.is_empty() || expected_visible.contains('\r') {
        return None;
    }
    let sanitized = sanitize_ansi(ansi);
    let lines = line_ranges(&sanitized.text);
    let chrome = match kind {
        AgentKind::Codex => &CODEX_CHROME,
        AgentKind::Claude => &CLAUDE_CHROME,
    };
    let expected_lines: Vec<&str> = expected_visible.split('\n').collect();
    let mut candidates = Vec::new();

    for boundary in 0..lines.len() {
        if !is_pure_separator(
            line_text(&sanitized.text, lines[boundary]),
            chrome.separator_min_width,
        ) {
            continue;
        }
        let first = boundary + 1;
        let trailing = first + expected_lines.len();
        let composer = trailing + 1;
        if composer >= lines.len()
            || !is_trailing_boundary(line_text(&sanitized.text, lines[trailing]), chrome)
            || !starts_with_any(
                line_text(&sanitized.text, lines[composer]),
                chrome.composer_prefixes,
            )
        {
            continue;
        }

        let mut mappings = Vec::with_capacity(expected_lines.len() * 2);
        let mut destination = 0;
        let mut exact = true;
        for (offset, expected_line) in expected_lines.iter().enumerate() {
            let range = lines[first + offset];
            let source_line = line_text(&sanitized.text, range);
            let prefixes = if offset == 0 {
                chrome.role_prefixes
            } else {
                chrome.continuation_prefixes
            };
            let prefix = if source_line.is_empty() && expected_line.is_empty() {
                ""
            } else if let Some(prefix) = prefixes
                .iter()
                .copied()
                .find(|prefix| source_line.starts_with(prefix))
            {
                prefix
            } else {
                exact = false;
                break;
            };
            let content_start = range.start + prefix.len();
            if &sanitized.text[content_start..range.end] != *expected_line {
                exact = false;
                break;
            }
            mappings.push((content_start, range.end, destination));
            destination += expected_line.len();
            if offset + 1 < expected_lines.len() {
                let newline_end = range.end.checked_add(1)?;
                if sanitized.text.as_bytes().get(range.end) != Some(&b'\n') {
                    exact = false;
                    break;
                }
                mappings.push((range.end, newline_end, destination));
                destination += 1;
            }
        }
        if exact {
            candidates.push((slice_mapped_runs(&sanitized.runs, &mappings), composer));
        }
    }

    if candidates.len() != 1 {
        return None;
    }
    let (runs, composer) = candidates.pop().expect("one candidate");
    if lines[composer + 1..]
        .iter()
        .map(|range| line_text(&sanitized.text, *range))
        .filter(|line| !line.is_empty())
        .any(|line| !starts_with_any(line, chrome.footer_prefixes))
    {
        return None;
    }
    Some(StyledText {
        text: expected_visible.to_owned(),
        runs,
    })
}
```

- [ ] **Step 5: Project once per retry sequence and emit the complete native presentation**

In `src/ui/runtime.rs`, import `crate::markdown::style_markdown` and update
`resolve_capture` exactly as follows:

```rust
fn resolve_capture(
    kind: AgentKind,
    canonical_text: &str,
    mut read: impl FnMut() -> AppResult<String>,
    attempts: usize,
    retry_delay: Duration,
) -> (MessagePresentation, Option<String>) {
    let fallback = style_markdown(canonical_text);
    let mut last_error = None;
    for attempt in 0..attempts {
        match read() {
            Ok(ansi) => {
                if let Some(styled) = extract_native_final(&ansi, &fallback.text, kind) {
                    return (MessagePresentation::NativeAnsi(styled), None);
                }
            }
            Err(error) => last_error = Some(error.to_string()),
        }
        if attempt + 1 < attempts {
            thread::sleep(retry_delay);
        }
    }
    (MessagePresentation::MarkdownFallback, last_error)
}
```

Keep `resolve_capture_command`'s fingerprint based on `canonical_text`; rendered
text must never replace canonical transcript identity.

- [ ] **Step 6: Update all Task 2 test constructors and verify capture behavior**

Use this shape anywhere a runtime or ANSI test needs native presentation:

```rust
MessagePresentation::NativeAnsi(StyledText {
    text: "answer".into(),
    runs: vec![StyleRun {
        start_byte: 0,
        end_byte: "answer".len(),
        foreground: Some(AnsiColor::Green),
        background: None,
        modifiers: StyleModifiers::default(),
    }],
})
```

Run:

```bash
cargo fmt --check
cargo test --test ansi_style
cargo test ui::runtime::tests::capture_resolution -- --nocapture
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: plain and projected exact captures pass, mismatched/ambiguous/unsafe
candidates still fall back, retry count remains bounded, and strict Clippy
passes.

- [ ] **Step 7: Review, verify, and commit Task 2**

Invoke `superpowers:requesting-code-review`, fix every Critical or Important
finding, then invoke `superpowers:verification-before-completion` and rerun Step
6.

Commit:

```bash
git add src/style.rs src/ansi.rs src/ui/runtime.rs tests/ansi_style.rs
git commit -m "capture projected native answers"
```

## Task 3: Reduce and render owned native display text

**Files:**
- Modify: `src/app.rs`
- Modify: `src/ui/visual_rows.rs`
- Test: `tests/app_state.rs`
- Test: `tests/ui_render.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before changing production code.
- Invoke `superpowers:requesting-code-review` after reducer/render changes.
- Invoke `superpowers:verification-before-completion` before marking the task complete.

- [ ] **Step 1: Add reducer tests for canonical identity and rendered-data validation**

In `tests/app_state.rs`, import `StyledText` and use a local helper:

```rust
fn native_presentation(text: &str, runs: Vec<StyleRun>) -> MessagePresentation {
    MessagePresentation::NativeAnsi(StyledText {
        text: text.into(),
        runs,
    })
}
```

Update `final_presentation_applies_only_to_the_same_stable_id_and_text_fingerprint`
so its canonical final is
`"**canonical** [docs](https://example.test)"`, its native rendered text is
`"canonical docs"`, and `text_fingerprint` is always computed from canonical
Markdown. Add this validation case:

```rust
#[test]
fn final_presentation_rejects_controls_and_ranges_invalid_for_rendered_text() {
    let invalid = [
        StyledText {
            text: "rendered\u{1b}[31m".into(),
            runs: Vec::new(),
        },
        StyledText {
            text: "short".into(),
            runs: vec![StyleRun {
                start_byte: 0,
                end_byte: 99,
                foreground: Some(AnsiColor::Green),
                background: None,
                modifiers: StyleModifiers::default(),
            }],
        },
    ];

    for styled in invalid {
        let mut app = AppState::default();
        app.apply(AppEvent::NativeUser(Message::text("prompt", "question", Some(1))));
        app.apply(AppEvent::NativeFinal(Message::final_text("answer", "**canonical**", Some(2))));
        app.apply(AppEvent::FinalPresentation {
            stable_id: "answer".into(),
            text_fingerprint: fingerprint("**canonical**"),
            presentation: MessagePresentation::NativeAnsi(styled),
        });
        assert_eq!(
            app.turns[0].final_answer.as_ref().unwrap().presentation,
            MessagePresentation::MarkdownFallback
        );
    }
}
```

Keep the existing test proving `MarkdownFallback` never downgrades an already
accepted native presentation.

- [ ] **Step 2: Add render tests that distinguish canonical and displayed text**

Extend `markdown_fallback_body_styles_flow_into_rendered_visual_rows` in
`tests/ui_render.rs` with a link and exact visible-text assertions:

```rust
app.apply(AppEvent::NativeFinal(Message::final_text(
    "a1",
    "# Result\nplain **Ω** and `λ`; read [docs](https://example.test)",
    Some(2),
)));

let rendered = render_to_string(&app, &Editor::default(), 80, 16);
assert!(rendered.contains("Result"));
assert!(rendered.contains("plain Ω and λ; read docs"));
assert!(!rendered.contains("https://example.test"));
assert!(!rendered.contains("**"));
assert!(!rendered.contains('`'));
```

Add a native presentation case whose canonical text contains Markdown but whose
stored rendered text differs deliberately:

```rust
#[test]
fn native_presentation_renders_owned_visible_text_not_canonical_markdown() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text("u1", "show result", Some(1))));
    app.apply(AppEvent::NativeFinal(Message {
        stable_id: "a1".into(),
        text: "# [canonical](https://example.test)".into(),
        presentation: MessagePresentation::NativeAnsi(StyledText {
            text: "Native label".into(),
            runs: vec![StyleRun {
                start_byte: 0,
                end_byte: "Native label".len(),
                foreground: Some(AnsiColor::Cyan),
                background: None,
                modifiers: StyleModifiers::default(),
            }],
        }),
        attachments: Vec::new(),
        timestamp_ms: Some(2),
    }));

    let rendered = render_to_string(&app, &Editor::default(), 80, 12);
    assert!(rendered.contains("Native label"));
    assert!(!rendered.contains("canonical"));
    assert!(!rendered.contains("example.test"));
}
```

- [ ] **Step 3: Run focused tests and observe the expected failures**

Run:

```bash
cargo test --test app_state final_presentation -- --nocapture
cargo test --test ui_render markdown_fallback_body_styles_flow_into_rendered_visual_rows -- --exact
cargo test --test ui_render native_presentation_renders_owned_visible_text_not_canonical_markdown -- --exact
```

Expected: reducer code validates runs against canonical text, and visual rows
reconstruct native text from `Message.text` instead of using owned rendered
text.

- [ ] **Step 4: Validate native styled text and keep canonical identity in the reducer**

In `src/app.rs`, import `validate_styled_text` instead of
`validate_style_runs`, and change only the native validation arm:

```rust
let valid = match &presentation {
    MessagePresentation::NativeAnsi(styled) => validate_styled_text(styled).is_ok(),
    MessagePresentation::MarkdownFallback => {
        !matches!(message.presentation, MessagePresentation::NativeAnsi(_))
    }
    MessagePresentation::Plain => false,
};
```

Do not change lookup by `stable_id` and `fingerprint(&message.text)`. Keep replay
preservation conditional on equal canonical message text/fingerprint so stale
native presentations cannot attach to a replaced transcript answer.

- [ ] **Step 5: Render presentation-owned text for native answers**

In `src/ui/visual_rows.rs`, make `answer_lines` use exactly one display source:

```rust
fn answer_lines(message: &Message) -> Vec<StyledText> {
    let source = match &message.presentation {
        MessagePresentation::NativeAnsi(styled) => styled.clone(),
        MessagePresentation::MarkdownFallback => crate::markdown::style_markdown(&message.text),
        MessagePresentation::Plain => StyledText {
            text: message.text.clone(),
            runs: Vec::new(),
        },
    };
    split_styled_lines(&source)
}
```

Do not change wrapping, Unicode-width calculation, scrolling, sticky prompt
geometry, prompt backgrounds, composer, or blocked view.

- [ ] **Step 6: Update all reducer/render test constructors and verify**

Mechanically wrap every `MessagePresentation::NativeAnsi(runs)` test fixture in
`StyledText { text, runs }`, choosing `text` to match the run offsets. Keep
canonical Markdown separate only in tests explicitly exercising that contract.

Run:

```bash
cargo fmt --check
cargo test --test app_state
cargo test --test ui_render
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: reducer and UI suites pass; supported Markdown syntax and link
destinations are absent from fallback output; prompt, sticky, scroll, composer,
large-paste marker, and blocked-mode tests remain green.

- [ ] **Step 7: Review, verify, and commit Task 3**

Invoke `superpowers:requesting-code-review`, fix every Critical or Important
finding, then invoke `superpowers:verification-before-completion` and rerun Step
6.

Commit:

```bash
git add src/app.rs src/ui/visual_rows.rs tests/app_state.rs tests/ui_render.rs
git commit -m "render presentation-owned final text"
```

## Task 4: Persist exact native rendered text in journal v2

**Files:**
- Modify: `src/history.rs`
- Modify: `src/ui/mod.rs`
- Test: `tests/history_journal.rs`
- Test: `tests/app_state.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before changing production code.
- Invoke `superpowers:requesting-code-review` after journal migration changes.
- Invoke `superpowers:verification-before-completion` before marking the task complete.

- [ ] **Step 1: Extend journal fixtures and write v2 round-trip tests first**

In `tests/history_journal.rs`, update current helper records to `version: 2` and
add absent rendered fields:

```rust
rendered_text: None,
rendered_text_fingerprint: None,
```

Add this helper and use it for every current native record:

```rust
fn set_native(record: &mut VisibleHistoryRecord, rendered: &str) {
    record.presentation = PersistedPresentation::NativeAnsi(vec![native_run(rendered)]);
    record.rendered_text = Some(rendered.into());
    record.rendered_text_fingerprint = Some(fingerprint(rendered));
}
```

Add a v2 exact round-trip test:

```rust
#[test]
fn v2_native_record_round_trips_canonical_and_rendered_text_separately() {
    let root = test_root("v2-rendered-roundtrip");
    let _ = std::fs::remove_dir_all(&root);
    let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();
    let mut record = final_record(
        "a1",
        "u1",
        2,
        "# **Answer** [docs](https://example.test)",
    );
    set_native(&mut record, "Answer docs");

    journal.append(&record).unwrap();
    let loaded = journal.load().unwrap();
    assert_eq!(loaded, vec![record.clone()]);
    let mut app = AppState::default();
    app.hydrate_visible_history(vec![
        prompt("u1", "u1", 1, "question"),
        loaded.into_iter().next().unwrap(),
    ]);
    let message = app.turns[0].final_answer.as_ref().unwrap();
    assert_eq!(message.text, "# **Answer** [docs](https://example.test)");
    let MessagePresentation::NativeAnsi(styled) = &message.presentation else {
        panic!("v2 native presentation must be restored")
    };
    assert_eq!(styled.text, "Answer docs");
    std::fs::remove_dir_all(root).unwrap();
}
```

Use `AppState::hydrate_visible_history` for all restoration assertions. Keep
`VisibleHistoryRecord::into_message` crate-private; do not widen production API
visibility for integration tests.

- [ ] **Step 2: Add v2 integrity and v1 migration tests**

Add cases covering every migration rule:

```rust
#[test]
fn v2_native_record_requires_matching_safe_rendered_payload() {
    let base = final_record("a1", "u1", 2, "**canonical**");
    let invalid = [
        {
            let mut record = base.clone();
            record.presentation = PersistedPresentation::NativeAnsi(vec![native_run("visible")]);
            record
        },
        {
            let mut record = base.clone();
            record.presentation = PersistedPresentation::NativeAnsi(vec![native_run("visible")]);
            record.rendered_text = Some("visible".into());
            record.rendered_text_fingerprint = Some(fingerprint("different"));
            record
        },
        {
            let mut record = base.clone();
            record.presentation = PersistedPresentation::NativeAnsi(Vec::new());
            record.rendered_text = Some("bad\u{1b}[31m".into());
            record.rendered_text_fingerprint = Some(fingerprint("bad\u{1b}[31m"));
            record
        },
    ];

    for record in invalid {
        assert!(record.validate().is_err());
    }
}
```

For legacy records, add this exact helper after `final_record`:

```rust
fn legacy_native_record(
    id: &str,
    turn_id: &str,
    order: u64,
    text: &str,
    runs: Vec<StyleRun>,
) -> VisibleHistoryRecord {
    VisibleHistoryRecord {
        version: 1,
        role: VisibleRole::Final,
        stable_id: id.into(),
        turn_id: turn_id.into(),
        order,
        text: text.into(),
        attachments: Vec::new(),
        timestamp_ms: Some(order),
        text_fingerprint: fingerprint(text),
        presentation: PersistedPresentation::NativeAnsi(runs),
        rendered_text: None,
        rendered_text_fingerprint: None,
    }
}
```

Cover both compatibility paths with paired prompt records:

```rust
#[test]
fn v1_native_plain_text_keeps_styles_but_v1_markdown_downgrades_to_fallback() {
    let plain = legacy_native_record(
        "a1",
        "u1",
        2,
        "plain answer",
        vec![native_run("plain answer")],
    );
    let markdown = legacy_native_record(
        "a2",
        "u2",
        4,
        "**bold answer**",
        vec![StyleRun {
            start_byte: 2,
            end_byte: 13,
            foreground: Some(AnsiColor::Green),
            background: None,
            modifiers: StyleModifiers::default(),
        }],
    );

    let mut app = AppState::default();
    app.hydrate_visible_history(vec![
        prompt("u1", "u1", 1, "first"),
        plain,
        prompt("u2", "u2", 3, "second"),
        markdown,
    ]);

    let MessagePresentation::NativeAnsi(styled) =
        &app.turns[0].final_answer.as_ref().unwrap().presentation
    else {
        panic!("byte-identical v1 projection may retain native runs")
    };
    assert_eq!(styled.text, "plain answer");
    assert_eq!(
        app.turns[1].final_answer.as_ref().unwrap().presentation,
        MessagePresentation::MarkdownFallback
    );
}
```

Add the exact boundary-validation cases:

```rust
#[test]
fn rendered_fields_are_native_v2_only_and_runs_use_rendered_boundaries() {
    let mut fallback_with_rendered = final_record("a1", "u1", 2, "answer");
    fallback_with_rendered.rendered_text = Some("answer".into());
    fallback_with_rendered.rendered_text_fingerprint = Some(fingerprint("answer"));
    assert!(fallback_with_rendered.validate().is_err());

    let mut legacy_with_rendered = legacy_native_record(
        "a2",
        "u2",
        4,
        "answer",
        vec![native_run("answer")],
    );
    legacy_with_rendered.rendered_text = Some("answer".into());
    legacy_with_rendered.rendered_text_fingerprint = Some(fingerprint("answer"));
    assert!(legacy_with_rendered.validate().is_err());

    let mut rendered_relative = final_record(
        "a3",
        "u3",
        6,
        "# [long canonical label](https://example.test)",
    );
    set_native(&mut rendered_relative, "label");
    assert!(rendered_relative.validate().is_ok());

    let mut canonical_relative = rendered_relative.clone();
    canonical_relative.presentation = PersistedPresentation::NativeAnsi(vec![StyleRun {
        start_byte: 2,
        end_byte: 20,
        ..native_run("long canonical label")
    }]);
    assert!(canonical_relative.validate().is_err());
}
```

In `invalid_upserts_do_not_poison_the_last_valid_record`, change the deliberately
unsupported version to `3` after current helpers move to version 2:

```rust
let mut invalid_version = valid.clone();
invalid_version.version = 3;
append_json_line(journal.path(), &invalid_version);
```

- [ ] **Step 3: Run journal/reducer tests and observe schema failures**

Run:

```bash
cargo test --test history_journal -- --nocapture
cargo test --test app_state hydration -- --nocapture
cargo test ui::tests -- --nocapture
```

Expected: current structs lack rendered fields/version 2 support, and native
restoration cannot create an owned `StyledText`.

- [ ] **Step 4: Implement the v2 record shape and strict validation**

In `src/history.rs`, set current/legacy versions and extend the record:

```rust
const HISTORY_VERSION: u8 = 2;
const LEGACY_HISTORY_VERSION: u8 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct VisibleHistoryRecord {
    pub version: u8,
    pub role: VisibleRole,
    pub stable_id: String,
    pub turn_id: String,
    pub order: u64,
    pub text: String,
    pub attachments: Vec<VisibleAttachment>,
    pub timestamp_ms: Option<u64>,
    pub text_fingerprint: u64,
    pub presentation: PersistedPresentation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rendered_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rendered_text_fingerprint: Option<u64>,
}
```

Validation must first accept only versions 1 and 2, then retain all existing
identifier, canonical fingerprint, role/presentation, attachment, and style-run
checks. Add these exact presentation rules:

```rust
match (&self.role, &self.presentation, self.version) {
    (VisibleRole::Prompt, PersistedPresentation::Plain, _)
    | (VisibleRole::Final, PersistedPresentation::Fallback, _) => {
        if self.rendered_text.is_some() || self.rendered_text_fingerprint.is_some() {
            return Err(AppError::new(
                "history journal",
                "non-native record carries rendered presentation data",
            ));
        }
    }
    (VisibleRole::Final, PersistedPresentation::NativeAnsi(runs), HISTORY_VERSION) => {
        let rendered_text = self.rendered_text.as_ref().ok_or_else(|| {
            AppError::new("history journal", "native record is missing rendered text")
        })?;
        let rendered_fingerprint = self.rendered_text_fingerprint.ok_or_else(|| {
            AppError::new("history journal", "native record is missing rendered fingerprint")
        })?;
        if rendered_fingerprint != fingerprint(rendered_text) {
            return Err(AppError::new(
                "history journal",
                "rendered text fingerprint does not match",
            ));
        }
        validate_styled_text(&StyledText {
            text: rendered_text.clone(),
            runs: runs.clone(),
        })
        .map_err(|error| AppError::new("history journal", error))?;
    }
    (VisibleRole::Final, PersistedPresentation::NativeAnsi(runs), LEGACY_HISTORY_VERSION) => {
        if self.rendered_text.is_some() || self.rendered_text_fingerprint.is_some() {
            return Err(AppError::new(
                "history journal",
                "legacy native record carries v2 rendered data",
            ));
        }
        validate_style_runs(&self.text, runs)
            .map_err(|error| AppError::new("history journal", error))?;
    }
    (VisibleRole::Prompt, _, _) => {
        return Err(AppError::new(
            "history journal",
            "prompt carries answer-only presentation",
        ));
    }
    (VisibleRole::Final, PersistedPresentation::Plain, _) => {
        return Err(AppError::new(
            "history journal",
            "final presentation cannot be plain",
        ));
    }
}
```

- [ ] **Step 5: Write v2 native payloads and restore v1/v2 safely**

In `VisibleHistoryRecord::final_answer`, derive persisted presentation and
rendered fields from `message.presentation`:

```rust
let (presentation, rendered_text, rendered_text_fingerprint) = match &message.presentation {
    MessagePresentation::NativeAnsi(styled) => (
        PersistedPresentation::NativeAnsi(styled.runs.clone()),
        Some(styled.text.clone()),
        Some(fingerprint(&styled.text)),
    ),
    MessagePresentation::MarkdownFallback => (
        PersistedPresentation::Fallback,
        None,
        None,
    ),
    MessagePresentation::Plain => {
        return Err(AppError::new(
            "history journal",
            "final presentation cannot be plain",
        ));
    }
};
```

Pass both rendered values into `from_message`; prompts always pass `None,
None`. Restore presentation with this exact compatibility policy before moving
the other record fields:

```rust
let projected = crate::markdown::style_markdown(&self.text);
let presentation = match self.presentation {
    PersistedPresentation::Plain => MessagePresentation::Plain,
    PersistedPresentation::Fallback => MessagePresentation::MarkdownFallback,
    PersistedPresentation::NativeAnsi(runs) if self.version == HISTORY_VERSION => {
        match self.rendered_text {
            Some(text) => MessagePresentation::NativeAnsi(StyledText { text, runs }),
            None => MessagePresentation::MarkdownFallback,
        }
    }
    PersistedPresentation::NativeAnsi(runs) if projected.text == self.text => {
        MessagePresentation::NativeAnsi(StyledText {
            text: self.text.clone(),
            runs,
        })
    }
    PersistedPresentation::NativeAnsi(_) => MessagePresentation::MarkdownFallback,
};
```

This retains legacy native styles only when the new projection is byte-for-byte
canonical. A legacy Markdown answer that would change length becomes fallback
and can later be upgraded by a new exact terminal capture.

- [ ] **Step 6: Update remaining record literals and run migration coverage**

Add `rendered_text` and `rendered_text_fingerprint` to every record literal in
`tests/app_state.rs`, `tests/history_journal.rs`, `src/history.rs` unit tests,
and `src/ui/mod.rs` unit tests. Current records use version 2; explicit legacy
tests alone use version 1.

Run:

```bash
cargo fmt --check
cargo test --test history_journal
cargo test --test app_state
cargo test ui::tests
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: v2 exact rendered text/styles survive close/reopen, v1 plain native
styles survive, v1 length-changing Markdown downgrades safely, corrupt records
are ignored, and journal permissions/lifecycle/compaction tests remain green.

- [ ] **Step 7: Review, verify, and commit Task 4**

Invoke `superpowers:requesting-code-review`, fix every Critical or Important
finding, then invoke `superpowers:verification-before-completion` and rerun Step
6.

Commit:

```bash
git add src/history.rs src/ui/mod.rs tests/history_journal.rs tests/app_state.rs
git commit -m "persist rendered final presentations"
```

## Task 5: Document, verify, merge, and refresh the live plugin

**Files:**
- Modify: `README.md`
- Verify: `docs/superpowers/specs/2026-08-12-herdr-simple-prompts-design.md`
- Verify: all source and test files changed by Tasks 1-4

**Required skill checkpoints:**
- This task's README edit is documentation-only, so a tester-oriented coding checkpoint does not apply to that edit.
- Invoke `superpowers:requesting-code-review` for the complete diff before merge.
- Invoke `superpowers:verification-before-completion` before any completion claim.
- Invoke `superpowers:finishing-a-development-branch` after all verification passes.

- [ ] **Step 1: Update public behavior documentation**

Replace the two final-answer paragraphs in README's `Conversation view` with:

```markdown
For every final answer, the transcript remains the canonical Markdown value used
for identity and replay. Simple Prompts separately projects that Markdown into
the visible text a terminal renderer shows: supported heading, emphasis,
inline-code, and fenced-code delimiters are removed, and a Markdown link is
shown as its label without the destination.

For a newly observed final answer, Simple Prompts reads recent ANSI output from
the source agent and accepts a styled block only when its sanitized visible text
exactly matches that deterministic projection at one unique known Codex or
Claude boundary. The captured presentation owns both the visible text and safe
SGR colors/bold/dim/italic/underline styles. Cursor movement, alternate-screen
commands, OSC titles, hyperlinks, clipboard commands, and other terminal
controls are discarded and never replayed.

When exact native ANSI is unavailable, the same dependency-free projected text
is shown with deterministic fallback styles. Captured native visible text and
styles are saved together in the pane/session journal; older journal records
remain readable and are downgraded to fallback when their legacy style offsets
cannot safely describe the new visible projection.
```

In the live verification matrix, add a final-answer prompt that requests a
heading, bold/emphasis, inline code, a fenced code block, and a Markdown link.
The acceptance condition is: Simple Prompts shows the same visible words as the
native pane, hides the link destination and supported delimiters, retains
native colors/emphasis when exact capture succeeds, remains scrollable to the
last line, and restores the same result after overlay close/reopen.

- [ ] **Step 2: Run focused regression groups**

Run:

```bash
cargo test --test ansi_style
cargo test --test app_state
cargo test --test history_journal
cargo test --test ui_render
cargo test ui::runtime::tests::capture_resolution -- --nocapture
```

Expected: every focused group passes with no ignored failure.

- [ ] **Step 3: Run the complete source-only quality gate**

Invoke `superpowers:verification-before-completion`, then run from the task
worktree:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
git diff --check
git status --short
```

Expected: format, strict Clippy, all targets/features, locked release build, and
whitespace check pass. `git status --short` contains only the intended README
change before its commit.

- [ ] **Step 4: Review and commit documentation**

Invoke `superpowers:requesting-code-review` for the complete branch diff against
`main`. Fix every Critical or Important finding and repeat Step 3 if code or
tests change.

Commit:

```bash
git add README.md
git commit -m "document rendered final answers"
```

- [ ] **Step 5: Perform live Codex parity verification from the task build**

Build and link the task worktree:

```bash
cargo build --locked --release
herdr plugin link .
```

In a current Codex pane:

1. close any existing Simple Prompts overlay and reopen it with `prefix+m` so a
   new overlay process loads the rebuilt executable;
2. request a final containing `# Heading`, `**bold**`, `_italic_`, `` `code` ``,
   a fenced code block, and `[docs](https://example.test)`;
3. compare native and Simple Prompts: visible words/order match, `#`, paired
   emphasis/code delimiters, and `https://example.test` are absent from Simple
   Prompts, while visible label/code/emphasis styles remain;
4. scroll to the final line and back to bottom to prove the projected length did
   not break history geometry;
5. close/reopen the overlay and confirm exact rendered text/styles survive from
   journal v2;
6. submit another prompt and confirm canonical identity/reconciliation still
   attaches the next final to the correct turn.

Record whether the result used native capture or deterministic fallback. Both
must have correct visible text; only successful exact capture may claim native
style parity.

- [ ] **Step 6: Perform live Claude parity verification when available**

Repeat Step 5 in a current Claude Code pane, including close/reopen persistence.
If no live Claude session is available, report that prerequisite explicitly;
passing Claude fixtures does not constitute a live Claude smoke result.

- [ ] **Step 7: Merge the verified branch into `main` and relink the installed plugin**

Invoke `superpowers:finishing-a-development-branch`. Because the user requested
the live plugin to be updated, merge the verified task branch into `main`
non-interactively, then build and relink from the main checkout:

```bash
git -C /Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts merge --ff-only fix/native-rendered-presentation
cd /Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts
cargo build --locked --release
herdr plugin link .
```

Do not delete the task worktree until the merged main SHA, linked plugin source,
and one newly reopened Simple Prompts overlay have been confirmed.

- [ ] **Step 8: Reverify the exact merged artifact and clean the worktree**

From main, run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
git status --short --branch
git log -1 --oneline
herdr plugin list --plugin herdr.simple-prompts
```

Expected: main is clean, all checks pass at the reported merged SHA, and Herdr
lists `herdr.simple-prompts` linked to the main checkout. Reopen the overlay once
more after relinking; only then remove the completed worktree and task branch.

## Final acceptance checklist

- [ ] Canonical transcript Markdown remains in `Message.text` and is still the only final identity/fingerprint input.
- [ ] Fallback visible text removes only recognized heading, emphasis, inline-code, and fenced-code delimiters and hides valid link destinations.
- [ ] Lists, paragraphs, malformed constructs, compact paste markers, and Unicode content remain intact.
- [ ] Every style run indexes the presentation's visible UTF-8 text and passes non-empty/ordered/non-overlapping/bounds/boundary validation.
- [ ] Native capture requires one unique reviewed Codex/Claude boundary whose sanitized text exactly equals the deterministic projection.
- [ ] Native presentation stores sanitized rendered text and its style runs; OSC URLs and all terminal controls remain absent.
- [ ] Reducer matching/replay remains based on stable id plus canonical fingerprint, and fallback never downgrades a native presentation.
- [ ] Visual rows use native presentation text directly and compute Markdown projection only for fallback answers.
- [ ] Journal v2 requires canonical fingerprint, rendered fingerprint, safe rendered text, and rendered-relative style runs for native records.
- [ ] Journal v1 remains readable; byte-identical projections retain native styles and length-changing projections downgrade to fallback.
- [ ] Prompt bands, sticky context, bottom scrolling, large-paste compaction, composer, working state, and blocked interaction remain unchanged.
- [ ] No dependency, binary artifact, HTTP client, telemetry, or runtime network path is added.
- [ ] Strict format, Clippy, all-target tests, locked release build, live Codex parity, available live Claude parity, merge to main, and final relink are evidence-backed.

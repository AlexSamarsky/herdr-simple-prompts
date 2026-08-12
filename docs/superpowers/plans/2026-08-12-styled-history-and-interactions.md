# Styled History and Native Interaction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Simple Prompts scroll and wrap exactly once, preserve safe native Codex/Claude final-answer styling, retain only the visible prompt/final subset for the current pane/session, and pass native blocked questions and approvals through without losing the draft.

**Architecture:** Keep transcript text authoritative and store presentation as validated style ranges over that exact text. Convert styled messages into one explicit Unicode cell-row document used by rendering, bottom scrolling, full-width prompt bands, and sticky prompt push-off. Capture native final styling with bounded ANSI reads and exact text matching; fall back to a dependency-free Markdown styler. Persist only pane/session-scoped visible records, and use Herdr status plus `events.wait` for temporary blocked passthrough and source-pane cleanup.

**Tech Stack:** Rust 1.85+, Ratatui 0.29, Crossterm 0.28, Serde/Serde JSON, `unicode-width` 0.2, standard-library sockets/files/threads/channels, Herdr 0.7.5 socket API, existing fake Unix-socket and terminal-buffer tests.

---

## Starting point and constraints

This plan starts at commit `6c70d00` on `feature/compact-sticky-prompts` and supersedes Task 4 onward in `2026-08-12-sticky-prompt-hierarchy.md`. The lossless chunked editor, compact large-paste projection, optimistic/native reconciliation, persisted draft metadata, and initial role labels are already implemented. Do not repeat those tasks.

The first implementation batch must remove the current intermediate history path in `src/ui/render.rs`: `wrapped_history_height` and Ratatui `Paragraph::wrap` cannot both participate in history layout. All history indices are `usize`; conversion to `u16` occurs only for a bounded viewport coordinate.

No crate may be added. ANSI sanitization, exact matching, Markdown fallback, journal framing, lifecycle cleanup, and interaction mapping remain visible Rust source. Synthetic fixtures must not contain real transcripts or secrets.

## File structure

- Create `src/style.rs`: serializable colors, modifiers, validated byte-range style runs, presentation provenance, and style slicing.
- Create `src/ansi.rs`: conservative ANSI tokenizer/sanitizer, SGR state machine, exact canonical final-block extraction, and native-chrome boundary rules.
- Create `src/markdown.rs`: deterministic dependency-free fallback styling for the approved Markdown subset.
- Create `src/history.rs`: versioned JSONL visible-history records, journal loading/upserts, private asynchronous writer, and namespace metadata.
- Create `src/ui/visual_rows.rs`: the only history wrapper, full-width row padding, document sections, bottom-offset viewport, and two-row sticky push-off.
- Create `src/ui/interaction.rs`: blocked-surface model and Crossterm-to-Herdr input mapping.
- Modify `src/lib.rs`, `src/model.rs`, `src/app.rs`, `src/state.rs`, `src/herdr/client.rs`, `src/transport.rs`, `src/toggle.rs`, `src/ui/mod.rs`, `src/ui/runtime.rs`, and `src/ui/render.rs` to integrate those focused modules.
- Create `tests/ansi_style.rs`, `tests/history_journal.rs`, and `tests/blocked_interaction.rs`; extend `tests/herdr_client.rs`, `tests/transport_status.rs`, `tests/app_state.rs`, `tests/ui_render.rs`, and `tests/toggle_state.rs`.
- Modify `README.md` to describe styling provenance, scrolling, blocked passthrough, private history, lifecycle, and limitations.

### Task 1: Define presentation data and sanitize ANSI safely

**Files:**
- Create: `src/style.rs`
- Create: `src/ansi.rs`
- Modify: `src/lib.rs`
- Modify: `src/model.rs`
- Create: `tests/ansi_style.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before editing production code.
- Invoke `superpowers:requesting-code-review` after the coding batch.
- Invoke `superpowers:verification-before-completion` before marking the task complete.

- [ ] **Step 1: Write failing style-range validation tests**

Create `tests/ansi_style.rs` with the public contract:

```rust
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

    let split_scalar = vec![StyleRun { end_byte: 2, ..valid[0].clone() }];
    assert!(validate_style_runs(text, &split_scalar).is_err());
    let overlap = vec![valid[0].clone(), StyleRun { start_byte: 3, ..valid[0].clone() }];
    assert!(validate_style_runs(text, &overlap).is_err());
}

#[test]
fn fallback_and_native_provenance_are_not_confused() {
    assert_ne!(MessagePresentation::MarkdownFallback, MessagePresentation::NativeAnsi(vec![]));
}
```

- [ ] **Step 2: Write failing safe-SGR and hostile-control tests**

Add tests that lock the supported subset and prove controls are never replayed:

```rust
use herdr_simple_prompts::ansi::sanitize_ansi;

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
```

Also cover named foreground/background colors, `22/23/24/39/49` resets, CR/LF normalization, C0 controls, CSI private modes, OSC ST termination, malformed/truncated sequences, and adjacent equal-style run coalescing.

- [ ] **Step 3: Run the focused test and verify RED**

Run:

```bash
cargo test --test ansi_style
```

Expected: compilation fails because `style`, `ansi`, `AnsiColor`, `StyleRun`, and `sanitize_ansi` do not exist.

- [ ] **Step 4a: Add the serializable presentation types**

Create `src/style.rs` with these exact shapes:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnsiColor {
    Black, Red, Green, Yellow, Blue, Magenta, Cyan, White,
    BrightBlack, BrightRed, BrightGreen, BrightYellow,
    BrightBlue, BrightMagenta, BrightCyan, BrightWhite,
    Indexed(u8), Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct StyleModifiers {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct StyleRun {
    pub start_byte: usize,
    pub end_byte: usize,
    pub foreground: Option<AnsiColor>,
    pub background: Option<AnsiColor>,
    pub modifiers: StyleModifiers,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MessagePresentation {
    Plain,
    NativeAnsi(Vec<StyleRun>),
    MarkdownFallback,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StyledText {
    pub text: String,
    pub runs: Vec<StyleRun>,
}
```

Export the types from `src/style.rs` but do not connect them to the message model yet.

- [ ] **Step 4b: Implement style validation and coalescing**

Implement `validate_style_runs(text, runs)` so every range is non-empty, ordered, non-overlapping, within `text.len()`, and starts/ends on UTF-8 boundaries. Implement a run builder that closes a run whenever SGR state changes and merges adjacent equal styles. Run the style-range tests before proceeding.

- [ ] **Step 4c: Add presentation to the canonical message model**

Add `presentation: MessagePresentation` to `Message`; keep `Message::text` as `Plain`, and add `Message::final_text` as `MarkdownFallback`. Make `AppEvent::NativeFinal` normalize a plain adapter message to `MarkdownFallback`, so existing adapters cannot accidentally present a final as a prompt even before they migrate to the helper.

- [ ] **Step 5a: Implement control-sequence stripping and printable-text mapping**

In `src/ansi.rs`, scan bytes without executing escape sequences:

```rust
pub fn sanitize_ansi(input: &str) -> StyledText
```

Accept printable UTF-8 and `\n`; normalize CRLF/CR to LF. Strip every CSI sequence for now, all OSC sequences terminated by BEL or ST, DCS/APC/PM strings, single-character ESC commands, DEL, and unsupported C0 controls. Preserve an output-byte to source-style-state mapping for the next step. A malformed sequence is discarded to its terminator or end-of-input, never copied.

- [ ] **Step 5b: Decode only the approved SGR subset**

Allow CSI sequences ending in `m` to update style state. Decode SGR `0,1,2,3,4,22,23,24,30..37,39,40..47,49,90..97,100..107`, `38;5;n`, `48;5;n`, `38;2;r;g;b`, and `48;2;r;g;b`. Ignore unsupported numeric parameters without copying them. Close/coalesce style runs through the shared builder.

- [ ] **Step 5c: Export modules and re-run hostile-control cases**

Export `style` and `ansi` from `src/lib.rs`.

- [ ] **Step 6: Run focused and regression tests and verify GREEN**

Run:

```bash
cargo test --test ansi_style
cargo test --test codex_parser --test claude_parser --test app_state
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: all pass; existing semantic transcript filtering remains unchanged.

- [ ] **Step 7: Request review, address findings, re-run Task 1 gates, and commit**

Commit only Task 1 files:

```bash
git add src/lib.rs src/model.rs src/style.rs src/ansi.rs tests/ansi_style.rs
git commit -m "sanitize native answer styles"
```

### Task 2: Replace double wrapping with one visual-row and sticky engine

**Files:**
- Create: `src/ui/visual_rows.rs`
- Modify: `src/ui/render.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/app.rs`
- Modify: `tests/ui_render.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before editing production code.
- Invoke `superpowers:requesting-code-review` after the coding batch.
- Invoke `superpowers:verification-before-completion` before marking the task complete.

- [ ] **Step 1: Add regressions for the two confirmed rendering defects**

Add tests that fail against commit `6c70d00`:

```rust
#[test]
fn narrow_multiword_answer_scrolls_to_its_real_last_visual_row() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text("u1", "question", Some(1))));
    app.apply(AppEvent::NativeFinal(Message::final_text(
        "a1",
        "one two three four five six seven eight nine ten eleven twelve",
        Some(2),
    )));

    let rendered = render_to_string(&app, &Editor::default(), 18, 8);
    assert!(rendered.contains("twelve"));
}

#[test]
fn every_wrapped_prompt_cell_has_the_prompt_background() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "first wrapped prompt line with several words",
        Some(1),
    )));

    let buffer = rendered_buffer(&app, 20, 10);
    for y in 0..3 {
        assert!((0..20).all(|x| buffer[(x, y)].style().bg == Some(Color::DarkGray)));
    }
}
```

Use row discovery by the `YOU` label rather than assuming row zero if the existing layout reserves a line.

- [ ] **Step 2: Add pure wrapper and sticky geometry tests**

Expose testable, Ratatui-independent contracts from `src/ui/visual_rows.rs`:

```rust
#[test]
fn wrap_preserves_unicode_width_styles_and_explicit_newlines() {
    let source = StyledText {
        text: "界界a\nnext".into(),
        runs: vec![StyleRun {
            start_byte: 0,
            end_byte: "界界".len(),
            foreground: Some(AnsiColor::Cyan),
            background: None,
            modifiers: StyleModifiers { bold: true, ..Default::default() },
        }],
    };
    let rows = wrap_styled(&source, 4);
    assert_eq!(
        rows.iter().map(VisualRow::plain_text).collect::<Vec<_>>(),
        vec!["界界".to_owned(), "a".to_owned(), "next".to_owned()],
    );
    assert_eq!(rows[0].cell_width(), 4);
    assert!(rows[0].spans[0].style.modifiers.bold);
}

#[test]
fn next_prompt_pushes_a_two_row_sticky_header_one_row_at_a_time() {
    let sections = vec![
        PromptSection { start_row: 0, prompt_rows: 4, end_row: 10 },
        PromptSection { start_row: 10, prompt_rows: 1, end_row: 14 },
    ];
    assert_eq!(sticky_overlay(&sections, 8, 5), Some(StickyRows { source_start: 0, screen_start: 0, count: 2 }));
    assert_eq!(sticky_overlay(&sections, 9, 5), Some(StickyRows { source_start: 1, screen_start: 0, count: 1 }));
    assert_eq!(sticky_overlay(&sections, 10, 5), None);
}
```

Add cases for: no duplicate while the natural first prompt row is visible; one-row prompt; CJK plus combining mark; image-only prompt; compact `[Pasted Content · 1000 chars]`; history heights 1, 2, and 3; `usize` histories beyond 65,535 rows; PageUp/PageDown; mouse wheel; and bottom auto-scroll.

- [ ] **Step 3: Run the UI test and verify RED**

Run:

```bash
cargo test --test ui_render
```

Expected: the multiword tail is missing or unreachable, right-edge prompt cells lack `DarkGray`, and the new visual-row API is absent.

- [ ] **Step 4a: Add Ratatui-independent row and section types**

Create these core types with `usize` document coordinates:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualSpan {
    pub text: String,
    pub style: CellStyle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualRow {
    pub spans: Vec<VisualSpan>,
    pub fill: Option<CellStyle>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromptSection {
    pub start_row: usize,
    pub prompt_rows: usize,
    pub end_row: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HistoryDocument {
    pub rows: Vec<VisualRow>,
    pub prompts: Vec<PromptSection>,
}
```

Keep all document coordinates and row counts as `usize`.

- [ ] **Step 4b: Implement style-preserving Unicode cell wrapping**

`wrap_styled(source, width)` iterates Unicode scalar boundaries, uses `unicode_width::UnicodeWidthChar`, keeps zero-width continuations attached to the preceding printable scalar, breaks before a scalar that would exceed the row width, honors explicit newlines, and guarantees at least one row per logical line. Split style runs only on validated UTF-8 boundaries. A cell wider than a one-cell viewport is placed once and clipped by Ratatui rather than looped forever.

- [ ] **Step 4c: Build prompt and answer rows from the wrapper**

Build prompt rows with the plugin-owned `YOU  ` prefix, apply the prompt style as `fill`, and pad during Ratatui conversion to the exact viewport width. Build answer rows from the answer label plus canonical styled spans. Do not call `.wrap(...)` or calculate a second wrapped height when rendering history:

```rust
let visible = document.viewport(history_height, app.scroll_from_bottom);
let text = Text::from(visible.into_iter().map(VisualRow::into_ratatui_line).collect::<Vec<_>>());
frame.render_widget(Paragraph::new(text), history_area);
```

- [ ] **Step 4d: Replace history rendering and bottom-scroll arithmetic**

Change `AppState::scroll_from_bottom` to `usize`. Bound it against `document.rows.len().saturating_sub(viewport_height)` at render/event time; convert only the final on-screen row/column to `u16`.

- [ ] **Step 5: Implement sticky selection and push-off from the same rows**

For a viewport top `top` and height `height`:

1. reserve at least one natural history row, so `sticky_limit = 2.min(height.saturating_sub(1))`;
2. select the latest prompt with `section.start_row < top && top < section.end_row`;
3. take its first `sticky_limit.min(section.prompt_rows)` rows;
4. if the next prompt begins `distance = next.start_row - top` rows below the top and `distance < sticky_count`, hide `sticky_count - distance` rows from the top of the sticky copy;
5. overlay only the remaining sticky suffix at screen row zero;
6. render no sticky copy when its natural first row is still visible.

The natural window and sticky copy are produced by `HistoryDocument::viewport`; `render.rs` must not duplicate this geometry.

- [ ] **Step 6: Run focused and regression tests and verify GREEN**

Run:

```bash
cargo test --test ui_render
cargo test --test editor --test app_state
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: newest answer rows are reachable, prompt backgrounds reach the right edge, and sticky tests pass without `u16` saturation.

- [ ] **Step 7: Request review, address findings, re-run Task 2 gates, and commit**

Commit only Task 2 files:

```bash
git add src/app.rs src/ui/mod.rs src/ui/render.rs src/ui/visual_rows.rs tests/ui_render.rs
git commit -m "render history from explicit visual rows"
```

### Task 3: Add deterministic styled-Markdown fallback

**Files:**
- Create: `src/markdown.rs`
- Modify: `src/lib.rs`
- Modify: `src/ui/visual_rows.rs`
- Modify: `tests/ansi_style.rs`
- Modify: `tests/ui_render.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before editing production code.
- Invoke `superpowers:requesting-code-review` after the coding batch.
- Invoke `superpowers:verification-before-completion` before marking the task complete.

- [ ] **Step 1: Write failing fallback coverage**

Add one table-driven fixture containing paragraphs, `#`/`##` headings, `-` and numbered list items, inline code, fenced code, `**bold**`, `_italic_`, and `[label](https://example.test)`. Assert:

- `style_markdown(text).text == text` exactly;
- delimiters remain visible unless the rule explicitly styles their contents;
- headings and strong text are bold;
- inline/fenced code uses a distinct neutral foreground/background style;
- emphasis is italic;
- link labels are underlined/cyan while the URL remains canonical text;
- malformed/unclosed markers remain plain;
- the result validates with `validate_style_runs`;
- `MessagePresentation::MarkdownFallback` stays the provenance even after runs are computed.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test --test ansi_style --test ui_render markdown
```

Expected: compilation fails because `markdown::style_markdown` is absent and fallback messages render as unstyled plain text.

- [ ] **Step 3a: Implement block-level Markdown styling**

Create:

```rust
pub fn style_markdown(text: &str) -> StyledText
```

Use a line pass for fenced blocks, headings, and lists. Do not rewrite `text`; produce non-overlapping style runs only. Fenced-code state ends at a matching triple-backtick line or EOF.

- [ ] **Step 3b: Implement inline Markdown styling**

Use a byte-boundary inline pass for code, strong, emphasis, and links. Inline constructs never span a newline. When constructs overlap, precedence is fenced code, inline code, link, strong, emphasis. Invalid syntax contributes no style run. Coalesce adjacent equal runs through the shared style builder.

- [ ] **Step 3c: Connect fallback presentation to the visual-row engine**

Map `MessagePresentation::MarkdownFallback` to these runs while `NativeAnsi` uses its stored runs and `Plain` uses no body runs. The plugin-owned green `ANSWER` label is independent of body provenance.

- [ ] **Step 4: Run focused and regression tests and verify GREEN**

Run:

```bash
cargo test --test ansi_style --test ui_render
cargo clippy --all-targets --all-features -- -D warnings
```

- [ ] **Step 5: Request review, address findings, re-run Task 3 gates, and commit**

```bash
git add src/lib.rs src/markdown.rs src/ui/visual_rows.rs tests/ansi_style.rs tests/ui_render.rs
git commit -m "style fallback final answers"
```

### Task 4: Capture native final-answer ANSI by exact canonical text

**Files:**
- Modify: `src/herdr/client.rs`
- Modify: `src/transport.rs`
- Modify: `src/ansi.rs`
- Modify: `src/app.rs`
- Modify: `src/ui/runtime.rs`
- Modify: `src/ui/mod.rs`
- Modify: `tests/herdr_client.rs`
- Modify: `tests/transport_status.rs`
- Modify: `tests/ansi_style.rs`
- Modify: `tests/app_state.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before editing production code.
- Invoke `superpowers:requesting-code-review` after the coding batch.
- Invoke `superpowers:verification-before-completion` before marking the task complete.

- [ ] **Step 1: Write failing Herdr read-contract tests**

Use `ScriptedHerdr` to assert this exact request for final capture:

```json
{
  "method": "agent.read",
  "params": {
    "target": "w1:p1",
    "source": "recent_unwrapped",
    "lines": 240,
    "format": "ansi",
    "strip_ansi": false
  }
}
```

Add a visible blocked-read assertion for `pane.read` with `source: "visible"`, `format: "ansi"`, and `strip_ansi: false`. Verify `/read/text` extraction and a structured error when it is absent.

- [ ] **Step 2: Write failing exact-match safety tests**

Add fixtures where canonical final text also appears in a user prompt, commentary, a tool result, a neighboring answer, and the native composer. Assert `extract_native_final(ansi, canonical, kind)` returns styles only when one complete canonical block exists at an accepted Codex/Claude answer boundary. Add mismatch, partial-scrollback, duplicate-ambiguous, and canonical-text-with-ANSI-looking-literals cases; all unsafe cases return `None`.

Use an accepted case such as:

```rust
let ansi = "tool output\n────────\n\x1b[32m• Final heading\x1b[0m\n  body\n────────\n› Write a prompt";
let captured = extract_native_final(ansi, "Final heading\nbody", AgentKind::Codex).unwrap();
assert_eq!(captured.text, "Final heading\nbody");
assert_eq!(captured.runs[0].foreground, Some(AnsiColor::Green));
```

Keep the accepted native chrome prefixes in a small agent-specific table. Selection must be canonical-text equality after stripping only those known prefixes and indentation; never use a fuzzy similarity score.

- [ ] **Step 3: Run focused tests and verify RED**

Run:

```bash
cargo test --test herdr_client --test transport_status --test ansi_style --test app_state
```

Expected: ANSI read helpers, exact extraction, capture events, and presentation replacement do not exist.

- [ ] **Step 4: Add typed Herdr read helpers**

Add:

```rust
pub fn agent_read_recent_unwrapped_ansi(&self, target: &str, lines: u32) -> Result<String, HerdrError>;
pub fn pane_read_visible_text(&self, pane_id: &str, lines: u32) -> Result<String, HerdrError>;
pub fn pane_read_visible_ansi(&self, pane_id: &str, lines: u32) -> Result<String, HerdrError>;
```

Keep `call` as the single bounded request/response primitive. `AgentTransport::recent_unwrapped_ansi` and `visible_source_ansi` must call `validate_source` before reading.

- [ ] **Step 5a: Preserve sanitized byte mappings while removing known chrome**

Sanitize the complete ANSI read first while retaining the output byte mapping. Add the small reviewed Codex/Claude tables for role prefix, continuation indentation, separator, composer, and footer boundaries. Remove only those known boundary tokens.

- [ ] **Step 5b: Select one exact canonical candidate and slice its runs**

Enumerate only line-aligned candidate blocks whose agent-specific leading chrome can be removed. A candidate is accepted only when its normalized line content equals canonical transcript text exactly and its byte mapping can slice the sanitizer's runs safely. If zero or more than one candidate remains, return `None`. Return `StyledText { text: canonical.to_owned(), runs: sliced_runs }`; never return the terminal's altered text.

- [ ] **Step 6a: Add the bounded capture command/event contracts**

Add a dedicated capture worker and bounded channel to `UiRuntime`. When a `FollowerEvent::Conversation(ConversationEvent::Final(message))` reaches the UI, apply the semantic message immediately as `MarkdownFallback`, then enqueue:

```rust
CaptureCommand {
    stable_id: message.stable_id.clone(),
    canonical_text: message.text.clone(),
}
```

Add a separate bounded channel and thread for capture work. The worker tries at most 8 reads, 75 ms apart, over 240 recent-unwrapped lines. It stops at the first exact match and emits:

```rust
RuntimeEvent::FinalPresentation {
    stable_id: String,
    text_fingerprint: u64,
    presentation: MessagePresentation,
}
```

- [ ] **Step 6b: Apply captures only to the same canonical final**

On exhaustion it emits `MarkdownFallback`; a read error is reported as capture diagnostics without replacing the canonical final or disabling the composer. `AppEvent::FinalPresentation` applies only when stable id and current text fingerprint both match, preventing a late result from styling a replaced message.

- [ ] **Step 7: Run focused and regression tests and verify GREEN**

```bash
cargo test --test herdr_client --test transport_status --test ansi_style --test app_state
cargo test --test ui_render --test codex_parser --test claude_parser
cargo clippy --all-targets --all-features -- -D warnings
```

- [ ] **Step 8: Request review, address findings, re-run Task 4 gates, and commit**

```bash
git add src/ansi.rs src/app.rs src/herdr/client.rs src/transport.rs src/ui/mod.rs src/ui/runtime.rs tests/ansi_style.rs tests/app_state.rs tests/herdr_client.rs tests/transport_status.rs
git commit -m "capture native final answer styles"
```

### Task 5: Persist and reconcile the pane/session visible-history journal

**Files:**
- Create: `src/history.rs`
- Modify: `src/lib.rs`
- Modify: `src/model.rs`
- Modify: `src/app.rs`
- Modify: `src/state.rs`
- Modify: `src/ui/mod.rs`
- Create: `tests/history_journal.rs`
- Modify: `tests/app_state.rs`
- Modify: `tests/toggle_state.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before editing production code.
- Invoke `superpowers:requesting-code-review` after the coding batch.
- Invoke `superpowers:verification-before-completion` before marking the task complete.

- [ ] **Step 1: Write failing journal privacy and recovery tests**

Define a record contract with role plus turn ownership:

```rust
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
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
}
```

Add tests proving:

- the path is `history/w1_p1/session-1.jsonl`;
- directories are `0700` and file is `0600`;
- the latest valid record for `(role, stable_id)` wins;
- a later native-style upsert replaces fallback presentation;
- an incomplete final line is ignored;
- invalid version/fingerprint/style boundaries/overlaps are rejected without poisoning earlier valid records;
- attachment persistence keeps only id and sanitized display label, never `native_path`;
- a compact prompt record contains `[Pasted Content · N chars]` and never the hidden pasted body;
- reasoning, interaction snapshots, working text, and tool data have no record variants;
- dropping the asynchronous writer flushes the latest queued upserts.

- [ ] **Step 2: Run the journal test and verify RED**

```bash
cargo test --test history_journal
```

Expected: the `history` module and record/writer contracts do not exist.

- [ ] **Step 3a: Implement safe journal paths and record validation**

Create `HistoryJournal::at(state_root, source_pane, session_id)`. Sanitize each path component with the existing allowlist `[A-Za-z0-9_-]`, replacing every other scalar with `_`; reject empty sanitized session ids. Load with `BufRead::read_until(b'\n')`, parse only newline-terminated records, validate version `1`, fingerprint, attachments, and style ranges, and keep the latest valid value in a `BTreeMap<(VisibleRole, String), (line_number, record)>`. Sort returned records by `(order, line_number)`.

- [ ] **Step 3b: Implement private append and tolerant reload**

Append one JSON object plus `\n` using `OpenOptions::append(true).create(true).mode(0o600)`, `write_all`, and `sync_data`. Create and chmod the root, `history`, and pane directory to `0700`; chmod the file to `0600` after opening. Use a `HistoryWriter` with the same condvar/coalescing pattern as `DraftWriter`, but queue a map of latest record per stable key so no visible update is lost while disk I/O is in flight.

- [ ] **Step 3c: Add the non-blocking coalescing writer**

Move append calls behind `HistoryWriter`. On each wake, take the complete pending map, append records in deterministic key order, and keep the first write error for the UI to surface. `Drop` requests shutdown, flushes the remaining map, and joins the worker.

Persist `PersistedPresentation::Plain` for prompts and either `PersistedPresentation::NativeAnsi(Vec<StyleRun>)` or `PersistedPresentation::Fallback` for final answers; reject a final marked `Plain` or a prompt carrying answer-only presentation. Never serialize Ratatui types.

- [ ] **Step 4a: Hydrate ordered journal turns before transcript polling**

Load journal records before `follower.poll_initial`. Add `AppState::hydrate_visible_history(records)` to construct ordered turns.

- [ ] **Step 4b: Reconcile transcript replay by stable id and insertion point**

During transcript replay:

- an existing prompt stable id is updated/moved to the current replay insertion point instead of duplicated;
- an existing final stable id updates canonical text/metadata while preserving valid saved native presentation when its fingerprint matches;
- transcript-only prompts/finals are inserted in native order;
- saved records not present in a temporarily unreadable/truncated replay are retained;
- `TranscriptReloaded` no longer blindly deletes hydrated native history;
- optimistic deliveries are never journaled until they reconcile to a native prompt id.

- [ ] **Step 4c: Queue only visible native upserts**

Queue journal upserts after native prompt reconciliation, final insertion, and `FinalPresentation`. The answer record uses its owning prompt's native stable id as `turn_id`. Order is monotonic within the pane/session and recovered as `max(order) + 1` on reopen.

- [ ] **Step 5: Run focused and regression tests and verify GREEN**

```bash
cargo test --test history_journal --test app_state --test toggle_state
cargo test --test transcript_follower --test codex_parser --test claude_parser --test ui_render
cargo clippy --all-targets --all-features -- -D warnings
```

- [ ] **Step 6: Request review, address findings, re-run Task 5 gates, and commit**

```bash
git add src/lib.rs src/model.rs src/history.rs src/app.rs src/state.rs src/ui/mod.rs tests/history_journal.rs tests/app_state.rs tests/toggle_state.rs
git commit -m "persist visible pane history"
```

### Task 6: Add native blocked interaction passthrough

**Files:**
- Create: `src/ui/interaction.rs`
- Modify: `src/herdr/client.rs`
- Modify: `src/transport.rs`
- Modify: `src/app.rs`
- Modify: `src/ui/runtime.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/render.rs`
- Create: `tests/blocked_interaction.rs`
- Modify: `tests/herdr_client.rs`
- Modify: `tests/transport_status.rs`
- Modify: `tests/ui_render.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before editing production code.
- Invoke `superpowers:requesting-code-review` after the coding batch.
- Invoke `superpowers:verification-before-completion` before marking the task complete.

- [ ] **Step 1: Write failing pure input-mapping tests**

Define:

```rust
pub enum InteractionInput {
    Text(String),
    Key(&'static str),
}

pub fn map_interaction_key(key: KeyEvent) -> Option<InteractionInput>;
```

Assert exact mappings: arrows to `up/down/left/right`, Tab to `tab`, BackTab to `shift+tab`, space to `space`, Enter to `enter`, Backspace to `backspace`, Delete to `delete`, Esc to `esc`, and other printable characters to `Text(character.to_string())`. Ignore PageUp/PageDown, function keys, unsupported control chords, and mouse input. Shifted printable characters remain text. A paste while blocked becomes one `Text(full_paste)` and never touches `Editor`.

- [ ] **Step 2: Write failing blocked-view/state tests**

Cover these transitions:

1. `Working -> Blocked` hides history, working row, and composer and shows `INTERACTION REQUIRED` plus sanitized native content.
2. Existing editor snapshot, attachments, prompt display metadata, turns, and scroll offset remain byte-for-byte unchanged.
3. Snapshot ANSI/OSC cannot color or rewrite outside the blocked body.
4. Snapshot failure shows `Unable to read native interaction` and `prefix+m`.
5. `Blocked -> Working/Done` restores the exact previous composer and ordinary history.
6. Interaction input is forwarded exactly once and is never journaled.

- [ ] **Step 3: Run blocked tests and verify RED**

```bash
cargo test --test blocked_interaction --test ui_render --test transport_status
```

Expected: no interaction mode, key mapper, ANSI snapshot, or passthrough transport exists.

- [ ] **Step 4: Add source-validated interaction transport**

Add:

```rust
pub fn forward_interaction_text(&self, text: &str) -> AppResult<()>;
pub fn forward_interaction_key(&self, key: &str) -> AppResult<()>;
```

Both call `validate_source` immediately before `pane.send_input`. Text uses `{"text": text, "keys": []}`; a key uses `{"keys": [key]}` with no text. Do not route blocked input through `agent.prompt`, because native permission/choice surfaces own their input semantics.

- [ ] **Step 5a: Extend observations with an ephemeral blocked surface**

Replace the observation tuple with:

```rust
pub struct SourceObservation {
    pub identity: AgentIdentity,
    pub status_text: String,
    pub blocked_surface: Option<Result<StyledText, String>>,
}
```

The observer always reads the short plain status screen. Only while status is `Blocked`, also read up to 200 visible ANSI lines and sanitize them. Poll at the existing 200 ms interval and coalesce observation events.

- [ ] **Step 5b: Store blocked state only in memory**

Store the surface only in `AppState`; do not put it in `DraftState`, `VisibleHistoryRecord`, or logs. Clear it immediately after leaving `Blocked`.

- [ ] **Step 5c: Render the blocked surface instead of normal history/composer**

In `render.rs`, branch on `AgentStatus::Blocked` before normal layout. Render a plugin-owned bold yellow `INTERACTION REQUIRED` header, then the sanitized visible rows, then a plugin-owned footer `Native Codex/Claude interaction · prefix+m to return`. Never set the editor cursor in blocked mode.

- [ ] **Step 6: Route keys and paste before composer handling**

At the top of key and paste handling, if status is `Blocked`, map and enqueue `ActionCommand::Interaction`; do not call any editor method, scroll method, submit, image paste, or interrupt path. A failed send updates only the blocked error line. When the status leaves blocked, clear the ephemeral surface/error and return to the unchanged normal view.

- [ ] **Step 7: Run focused and regression tests and verify GREEN**

```bash
cargo test --test blocked_interaction --test ui_render --test herdr_client --test transport_status
cargo test --test editor --test app_state --test history_journal
cargo clippy --all-targets --all-features -- -D warnings
```

- [ ] **Step 8: Request review, address findings, re-run Task 6 gates, and commit**

```bash
git add src/app.rs src/herdr/client.rs src/transport.rs src/ui/interaction.rs src/ui/mod.rs src/ui/render.rs src/ui/runtime.rs tests/blocked_interaction.rs tests/herdr_client.rs tests/transport_status.rs tests/ui_render.rs
git commit -m "pass through blocked agent interactions"
```

### Task 7: Tie private state cleanup to source-pane lifecycle

**Files:**
- Modify: `src/herdr/client.rs`
- Modify: `src/state.rs`
- Modify: `src/toggle.rs`
- Modify: `src/ui/runtime.rs`
- Modify: `src/ui/mod.rs`
- Modify: `tests/herdr_client.rs`
- Modify: `tests/toggle_state.rs`
- Modify: `tests/history_journal.rs`
- Modify: `tests/blocked_interaction.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before editing production code.
- Invoke `superpowers:requesting-code-review` after the coding batch.
- Invoke `superpowers:verification-before-completion` before marking the task complete.

- [ ] **Step 1: Write failing `events.wait` contract and cleanup tests**

Assert the exact request:

```json
{
  "method": "events.wait",
  "params": {
    "match_event": {"event": "pane_closed", "pane_id": "w1:p1"},
    "timeout_ms": 1000
  }
}
```

Accept only a `wait_matched` response whose envelope has `event: "pane_closed"` and matching `data.pane_id`. Treat timeout as `Ok(false)` and mismatched success as a protocol error.

Add lifecycle tests proving:

- closing only the overlay removes the registry mapping but retains draft and `history/<pane>/<session>.jsonl`;
- a live source `pane_closed` removes registry, draft, compact metadata, journal, pane namespace metadata, and disables the overlay;
- startup/toggle removes state for a proven `not_found` source pane;
- startup/toggle removes an old session namespace when `agent.get` proves a replacement session;
- socket/permission/temporary API failure does not delete state;
- an unverifiable namespace is stamped orphaned and removed only after 7 full days on a later invocation;
- cleanup never accepts `..`, slash, or another pane id as a filesystem target.

- [ ] **Step 2: Run lifecycle tests and verify RED**

```bash
cargo test --test herdr_client --test toggle_state --test history_journal --test blocked_interaction
```

Expected: `events.wait`, pane-state manifests, and complete source cleanup do not exist; current overlay close cannot distinguish registry cleanup from pane-data cleanup.

- [ ] **Step 3a: Add the typed `events.wait` client method**

Implement:

```rust
pub fn wait_for_pane_closed(&self, pane_id: &str, timeout: Duration) -> Result<bool, HerdrError>;
```

Validate the response envelope and distinguish API timeout from protocol errors.

- [ ] **Step 3b: Add the stoppable lifecycle worker**

Use a dedicated lifecycle worker with a 1-second `events.wait` timeout so `Drop` can stop within one poll. API timeout is not a connection failure. On exact match, emit `RuntimeEvent::SourcePaneClosed` once and exit. On socket errors, keep the normal bounded retry/backoff policy and let identity polling decide whether input is disabled; never delete state based only on a disconnected socket.

- [ ] **Step 4a: Separate registry removal from scoped pane-data removal**

Keep `remove_source` as registry-only and introduce an explicit destructive-but-scoped operation:

```rust
pub fn remove_pane_state(&self, source_pane: &str) -> AppResult<()>;
```

It resolves only sanitized paths owned by `StateStore`: `draft-<safe>.json`, the pane's `history/<safe>/` directory, and pane lifecycle metadata. Validate every resolved parent against `self.root` before deleting. Never follow symlinks; reject a symlinked pane namespace. Registry update is atomic, then exact pane files/directories are removed. Missing files are success.

- [ ] **Step 4b: Persist pane/session verification metadata**

Add `PaneNamespaceState { version, source_pane, session_id, last_verified_ms, orphaned_since_ms }` at `panes/<safe-source-pane-id>.json`; the `panes` directory is `0700` and each file is `0600`. Upgrade draft persistence to version 3 with `session_id: Option<String>`: a verified current overlay writes its exact native session, a version-2 draft is loaded once as unbound and bound only after the source pane/session is successfully validated, and a bound draft is deleted on a proven session mismatch. Change `DraftWriter::spawn`/`save_editor_draft` to carry that session id while keeping the existing version-2 and legacy-string migration tests.

`validate_saved_namespaces(client, now_ms)` follows this decision table:

| Live check | Action |
|---|---|
| same pane + same session | clear orphan marker; keep state |
| pane `not_found` | remove pane state |
| same pane + different proven session | remove old session journal/metadata; keep a current draft only if it was created for the new session |
| socket/API unavailable | set/retain orphan timestamp; keep state |
| still unverifiable and age >= 7 days | remove pane state on this invocation |

- [ ] **Step 4c: Implement the validation decision table exactly**

Cover each row independently, clear an orphan timestamp only after a successful same-session validation, and compute the seven-day threshold with saturating millisecond arithmetic.

- [ ] **Step 5: Integrate cleanup at both required entry points**

Run validation once in `toggle::run_from_env` before toggle routing and once in `ui::run_from_env` before loading draft/journal. On `RuntimeEvent::SourcePaneClosed`, call `remove_pane_state`, set `input_enabled = false`, clear the ephemeral blocked surface, and show `Source pane closed · prefix+m to return`. Do not create any process that outlives the overlay action/UI process.

- [ ] **Step 6: Run focused and regression tests and verify GREEN**

```bash
cargo test --test herdr_client --test toggle_state --test history_journal --test blocked_interaction
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

- [ ] **Step 7: Request review, address findings, re-run Task 7 gates, and commit**

```bash
git add src/herdr/client.rs src/state.rs src/toggle.rs src/ui/mod.rs src/ui/runtime.rs tests/herdr_client.rs tests/toggle_state.rs tests/history_journal.rs tests/blocked_interaction.rs
git commit -m "clean state with source pane lifecycle"
```

### Task 8: Document behavior and run full source-only verification

**Files:**
- Modify: `README.md`
- Modify if test evidence requires: `.github/workflows/ci.yml`
- Modify only for verified contract drift: `docs/superpowers/specs/2026-08-12-herdr-simple-prompts-design.md`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before any production-code correction discovered here.
- Invoke `superpowers:requesting-code-review` after any correction batch and for the complete diff.
- Invoke `superpowers:verification-before-completion` before any completion claim.

- [ ] **Step 1: Update README from implemented behavior**

Document:

- history now uses one explicit row engine, bottoms correctly, and supports PageUp/PageDown/mouse scroll;
- `YOU` uses a full-width band, `ANSWER` remains unboxed, and two wrapped prompt rows stick/push off;
- native ANSI style is sanitized and exact-match-only; fallback Markdown is identified as fallback behavior;
- the visible-history JSONL is an intentional private pane/session copy, exactly what it may and may not contain, `0700/0600` permissions, and deletion policy;
- blocked questions/approvals show the native surface, supported keys, no mouse mapping, and `prefix+m` fallback;
- source-only build/no binaries/no HTTP/telemetry remains true;
- close-overlay versus close-source retention semantics;
- current Codex and Claude smoke steps.

Correct the old statement that the state directory contains only registry/draft/attachments.

- [ ] **Step 2: Run formatting and static gates**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: both exit 0 with no warnings.

- [ ] **Step 3: Run the complete automated suite**

```bash
cargo test --all-targets --all-features
cargo build --locked --release
git diff --check
```

Expected: all tests pass, the locked release build succeeds, and no whitespace errors remain.

- [ ] **Step 4: Run a local Herdr/Codex manual smoke**

Rebuild/relink the worktree, reopen one Codex pane, and verify:

1. newest long multiword answer is visible at the bottom and remains reachable after PageUp/PageDown;
2. every wrapped `YOU` row fills the pane width;
3. first two prompt visual rows stick and the next prompt pushes them one row at a time;
4. headings/lists/code/color in a newly completed final answer resemble the native pane without allowing cursor/title changes;
5. a 1,000+ character paste remains a compact marker in composer/history and sends the complete source;
6. `Working` appears below the submitted prompt;
7. a Codex question/approval enters `INTERACTION REQUIRED`, accepts every supported key exactly once, and restores the unchanged draft;
8. closing/reopening only the overlay restores styled visible history;
9. deleting the source pane removes its state namespace.

Record commands and observations in the task handoff; do not commit real pane output or state.

- [ ] **Step 5: Run a Claude smoke or record the exact external limitation**

Repeat final-style capture, one blocked interaction, and overlay reopen against Claude Code. If a Claude session is unavailable, do not claim live Claude verification; report automated fixture coverage and the missing live prerequisite explicitly.

- [ ] **Step 6: Verify the source-only repository boundary**

```bash
git ls-files
find . -type f -size +5M -not -path './.git/*' -not -path './target/*'
rg -n "reqwest|ureq|hyper|tokio|telemetry|analytics|update.?check|release.*binary" Cargo.toml Cargo.lock src README.md herdr-plugin.toml .github
```

Expected: no committed binary/artifact, no runtime network client, no binary upload workflow, and no unexplained file above 5 MB. Inspect any textual match rather than assuming every match is a failure.

- [ ] **Step 7: Request final code review and resolve every Important finding**

Review the complete branch against the approved design, specifically: double wrapping, full-width bands, sticky duplication/push-off, ANSI control stripping, exact canonical matching, fallback provenance, journal privacy, blocked input isolation, and lifecycle deletion scope. Re-run the complete gates after any correction.

- [ ] **Step 8: Commit documentation/corrections and verify clean status**

```bash
git add README.md .github/workflows/ci.yml docs/superpowers/specs/2026-08-12-herdr-simple-prompts-design.md
git commit -m "document styled simple prompts history"
git status --short
```

Stage only files that actually changed; omit absent paths from `git add`. Expected final status: clean.

## Plan completion checks

- [ ] Search this plan and implementation for `TODO`, `TBD`, `placeholder`, `unimplemented!`, and `todo!`; resolve every production occurrence or document why a fixture intentionally contains it.
- [ ] Confirm every public serialized record has an explicit version and every style range validates against its canonical text fingerprint.
- [ ] Confirm `src/ui/render.rs` contains no history `.wrap(...)` and no independent history-height calculation.
- [ ] Confirm no history document index uses `u16`.
- [ ] Confirm the hidden body of a compact paste can exist only in an unsent draft or transient optimistic delivery, never in the visible journal.
- [ ] Confirm blocked snapshots are ephemeral and cannot reach draft/history persistence.
- [ ] Confirm source-pane deletion is the only immediate destructive lifecycle event; overlay close retains pane/session history.
- [ ] Confirm no daemon, detached process, HTTP client, telemetry, downloaded executable, or new dependency was introduced.
- [ ] Run `cargo fmt --check`, strict Clippy, all-target tests, locked release build, `git diff --check`, and the applicable live smoke matrix after the final correction.

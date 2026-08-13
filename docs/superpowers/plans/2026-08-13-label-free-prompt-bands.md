# Label-Free Prompt Bands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render every user prompt as a full-width gray block with one empty gray row above and below its text, while removing the plugin-owned `YOU` and `ANSWER` labels.

**Architecture:** Keep `HistoryDocument` as the single geometry/rendering source. Add explicit prompt padding rows and distinguish a prompt section's block start from its content start so sticky history can pin up to two content rows plus the top padding when the viewport has room. Answer styles and transcript data remain unchanged; only the visual-row projection changes.

**Tech Stack:** Rust 1.85+, Ratatui, Crossterm, existing `HistoryDocument`/`VisualRow` model and Rust integration tests.

---

### Task 1: Render label-free padded prompt blocks

**Files:**
- Modify: `tests/ui_render.rs`
- Modify: `src/ui/visual_rows.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before changing production code.
- Invoke `superpowers:requesting-code-review` after the batch is green.
- Invoke `superpowers:verification-before-completion` before marking the task done.

- [ ] **Step 1: Replace the role-label test with a failing padded-band contract**

In `tests/ui_render.rs`, replace `prompt_band_and_answer_label_distinguish_roles_without_color_only` with a test that:

```rust
#[test]
fn prompt_is_a_label_free_gray_block_and_answer_is_unboxed() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text("u1", "check dns", Some(1))));
    app.apply(AppEvent::NativeFinal(Message::text(
        "a1",
        "zone is pending",
        Some(2),
    )));

    let rendered = render_to_string(&app, &Editor::default(), 50, 14);
    let document = HistoryDocument::from_app(&app, 50);

    assert!(!rendered.contains("YOU"));
    assert!(!rendered.contains("ANSWER"));
    assert_eq!(document.rows[0].plain_text(), "");
    assert_eq!(document.rows[1].plain_text(), "check dns");
    assert_eq!(document.rows[2].plain_text(), "");
    assert_eq!(document.rows[3].plain_text(), "zone is pending");
    assert_eq!(document.rows[0].fill, document.rows[1].fill);
    assert_eq!(document.rows[1].fill, document.rows[2].fill);
    assert!(document.rows[0].fill.is_some());
    assert!(document.rows[3].fill.is_none());
}
```

Replace `wrapped_prompt_continuations_keep_the_prefix_width_indent` with:

```rust
#[test]
fn wrapped_prompt_uses_the_full_width_without_a_role_indent() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "abcdefghijklmnopqrst",
        Some(1),
    )));

    let document = HistoryDocument::from_app(&app, 14);
    assert_eq!(document.rows[0].plain_text(), "");
    assert_eq!(document.rows[1].plain_text(), "abcdefghijklmn");
    assert_eq!(document.rows[2].plain_text(), "opqrst");
    assert_eq!(document.rows[3].plain_text(), "");
}
```

Update `wrapped_prompt_rows_fill_the_full_band_background` so it locates the row whose first cell is `a` rather than `Y`, then also asserts that all cells in `first - 1`, `first`, and the prompt's final padding row have `Color::DarkGray` backgrounds.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test --test ui_render prompt_is_a_label_free_gray_block_and_answer_is_unboxed -- --exact
cargo test --test ui_render wrapped_prompt_uses_the_full_width_without_a_role_indent -- --exact
```

Expected: both fail because history still emits `YOU`/`ANSWER`, indents prompt continuations, and has no gray padding rows.

- [ ] **Step 3: Implement the minimal visual-row projection**

In `src/ui/visual_rows.rs`:

1. Remove `PROMPT_PREFIX`, `ANSWER_PREFIX`, `prompt_prefix_style`, and `answer_prefix_style`.
2. Replace `push_labeled_rows` with a helper that wraps the full width and only applies an optional fill:

```rust
fn push_styled_rows(
    rows: &mut Vec<VisualRow>,
    source: &StyledText,
    fill: Option<CellStyle>,
    width: usize,
) {
    for mut row in wrap_styled(source, width) {
        row.fill = fill;
        rows.push(row);
    }
}

fn filled_empty_row(fill: Option<CellStyle>) -> VisualRow {
    VisualRow {
        spans: Vec::new(),
        fill,
    }
}
```

3. In `HistoryDocument::from_app`, append one `filled_empty_row(prompt_fill())` before prompt content and one after it. Render prompt content with `push_styled_rows(..., prompt_fill(), width)` and answers with `push_styled_rows(..., None, width)`.
4. Preserve the existing unfilled inter-turn row after the answer or pending prompt. Do not modify message text, ANSI runs, Markdown fallback, delivery state, or persistence.

- [ ] **Step 4: Run the focused render tests and verify GREEN**

Run:

```bash
cargo test --test ui_render prompt_is_a_label_free_gray_block_and_answer_is_unboxed -- --exact
cargo test --test ui_render wrapped_prompt_uses_the_full_width_without_a_role_indent -- --exact
cargo test --test ui_render wrapped_prompt_rows_fill_the_full_band_background -- --exact
```

Expected: all three pass.

### Task 2: Preserve two-line sticky prompt context with padding

**Files:**
- Modify: `tests/ui_render.rs`
- Modify: `src/ui/visual_rows.rs`

**Required skill checkpoints:**
- Continue the active `superpowers:test-driven-development` cycle before production changes.
- Invoke `superpowers:requesting-code-review` after the batch is green.
- Invoke `superpowers:verification-before-completion` before marking the task done.

- [ ] **Step 1: Add failing sticky geometry tests**

Extend `PromptSection` literals in `tests/ui_render.rs` with `content_start_row`. Update the principal sticky contract to:

```rust
#[test]
fn sticky_prompt_keeps_top_padding_and_two_content_rows_when_space_allows() {
    let sections = [PromptSection {
        start_row: 0,
        content_start_row: 1,
        prompt_rows: 2,
        end_row: 9,
    }];

    assert_eq!(sticky_overlay(&sections, 1, 5), None);
    assert_eq!(
        sticky_overlay(&sections, 2, 5),
        Some(StickyRows {
            source_start: 0,
            screen_start: 0,
            count: 3,
        })
    );
}
```

Update the constrained-height test to prove that height `2` pins one content row without padding, while height `4` pins top padding plus two content rows and always leaves at least one natural-history row. Update the next-prompt push-off test so the next section's `start_row` (its gray top padding) pushes the old sticky slice upward one row at a time.

- [ ] **Step 2: Run focused sticky tests and verify RED**

Run:

```bash
cargo test --test ui_render sticky_prompt_keeps_top_padding_and_two_content_rows_when_space_allows -- --exact
cargo test --test ui_render later_prompt_pushes_sticky_copy_off_one_row_at_a_time -- --exact
cargo test --test ui_render sticky_one_row_prompt_pins_one_row_and_short_viewports_keep_natural_content -- --exact
```

Expected: compile/test failure because `PromptSection` has no `content_start_row` and sticky geometry still treats the first prompt row as content.

- [ ] **Step 3: Implement padding-aware sticky geometry**

Change the section model to:

```rust
pub struct PromptSection {
    pub start_row: usize,
    pub content_start_row: usize,
    pub prompt_rows: usize,
    pub end_row: usize,
}
```

Set `start_row` before the top padding and `content_start_row` immediately after it in `HistoryDocument::from_app`. In `sticky_overlay`:

1. Select a section only after `content_start_row` has left the natural viewport.
2. Reserve at least one viewport row for natural history.
3. Select at most two prompt-content rows.
4. Prepend the top padding when one additional row is available.
5. Continue using the next section's `start_row` as the push-off boundary, shifting `source_start` forward as rows are displaced.

Use this shape:

```rust
let content_count = 2
    .min(section.prompt_rows)
    .min(height.saturating_sub(1));
let include_padding = content_count > 0 && height >= content_count + 2;
let desired_count = content_count + usize::from(include_padding);
let base_source_start = if include_padding {
    section.start_row
} else {
    section.content_start_row
};
```

Complete the function with the existing next-section push-off rule expressed
against the padded slice:

```rust
pub fn sticky_overlay(
    sections: &[PromptSection],
    top: usize,
    height: usize,
) -> Option<StickyRows> {
    let section_index = sections.iter().rposition(|section| {
        section.content_start_row < top && top < section.end_row
    })?;
    let section = sections[section_index];
    let content_count = 2
        .min(section.prompt_rows)
        .min(height.saturating_sub(1));
    if content_count == 0 {
        return None;
    }
    let include_padding = height >= content_count + 2;
    let desired_count = content_count + usize::from(include_padding);
    let base_source_start = if include_padding {
        section.start_row
    } else {
        section.content_start_row
    };
    let mut count = desired_count;
    if let Some(next) = sections.get(section_index + 1) {
        count = count.min(next.start_row.saturating_sub(top));
    }
    (count > 0).then_some(StickyRows {
        source_start: base_source_start + (desired_count - count),
        screen_start: 0,
        count,
    })
}
```

- [ ] **Step 4: Run all UI render tests and verify GREEN**

Run:

```bash
cargo test --test ui_render
```

Expected: all UI render tests pass, including generated documents above `u16::MAX`, image-only prompts, wrapping, scrolling, and blocked mode.

- [ ] **Step 5: Request code review and commit the implementation**

Run `superpowers:requesting-code-review`, resolve any Critical/Important findings, then:

```bash
git add src/ui/visual_rows.rs tests/ui_render.rs
git commit -m "simplify prompt and answer presentation"
```

### Task 3: Align public documentation

**Files:**
- Modify: `README.md`

**Required skill checkpoints:**
- Tester-oriented checkpoint does not apply because this task changes documentation only.
- Review the README diff against the approved design before committing.
- Invoke `superpowers:verification-before-completion` before claiming documentation alignment.

- [ ] **Step 1: Update the history appearance contract**

Replace README claims that prompts start with `YOU` and answers with `ANSWER`. State instead that each prompt is a full-width neutral-gray block with one gray blank row above and below its text, and each answer begins directly with styled answer text on the normal terminal surface.

- [ ] **Step 2: Check for stale role-label claims**

Run:

```bash
rg -n 'YOU|ANSWER|role label|prompt label|answer label' README.md docs/superpowers/specs/2026-08-12-herdr-simple-prompts-design.md
```

Expected: matches exist only where the documents explicitly state that these labels are absent.

- [ ] **Step 3: Commit documentation**

```bash
git add README.md
git commit -m "document label-free prompt bands"
```

### Task 4: Verify, merge, and activate in the current Herdr session

**Files:**
- No source edits expected.

**Required skill checkpoints:**
- Invoke `superpowers:verification-before-completion` before reporting success.
- Use `superpowers:finishing-a-development-branch` for the already selected local-merge workflow.

- [ ] **Step 1: Run all quality gates in the feature worktree**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
git diff --check main...HEAD
```

Expected: every command exits `0`; the full test output reports no failures.

- [ ] **Step 2: Fast-forward `main` and verify the merged tree**

From the main worktree, fast-forward merge `feature/compact-sticky-prompts`, then run:

```bash
cargo test --all-targets --all-features
cargo build --locked --release
```

Expected: both commands exit `0` on merged `main`.

- [ ] **Step 3: Relink the installed plugin to main**

From the main repository root run:

```bash
herdr plugin link .
herdr plugin list
herdr integration status
```

Expected: `herdr.simple-prompts` is enabled and its local source is the main repository root; Codex v6 and Claude v7 integrations are current.

- [ ] **Step 4: Smoke-test and clean up**

Invoke the toggle action against a supported current Codex pane, verify the rendered prompt has gray padding and no `YOU`/`ANSWER`, close only the overlay, then remove the merged feature worktree and delete the feature branch. Never close the user's source pane.

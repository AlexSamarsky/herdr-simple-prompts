# Prompt Background and Date Alignment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let user-prompt gray backgrounds touch both terminal edges while keeping prompt text at column 1 and moving user and answer timestamps to column 3.

**Architecture:** Keep all text rendering inside the existing one-cell content rectangle. Paint only the two outer history cells for visible rows that carry a prompt fill, and add a two-cell content prefix when either timestamp exists.

**Tech Stack:** Rust, Ratatui 0.29, Cargo integration tests.

---

### Task 1: Lock the corrected geometry with failing renderer tests

**Files:**
- Modify: `tests/ui_render.rs:27-75`
- Modify: `tests/ui_render.rs:116-145`
- Modify: `tests/ui_render.rs:521-554`
- Modify: `tests/ui_render.rs:1287-1299`

**Required skill checkpoints:**
- Use `superpowers:test-driven-development` before changing production code.
- Use `superpowers:requesting-code-review` after the complete RED/GREEN batch.
- Use `superpowers:verification-before-completion` before completion claims.

- [ ] **Step 1: Replace the all-rows-clear ordinary-view assertion with semantic edge assertions**

Add helpers that assert a blank cell has the expected prompt background and that
non-prompt cells remain clear:

```rust
fn assert_prompt_fill_cell(buffer: &Buffer, x: u16, y: u16) {
    let cell = &buffer[(x, y)];
    assert_eq!(cell.symbol(), " ");
    assert_eq!(cell.style().bg, Some(Color::Rgb(52, 53, 54)));
}
```

In `ordinary_view_uses_one_clear_cell_on_both_horizontal_edges`, assert both
edge cells are prompt gray on the prompt rows, while answer, Working, composer,
and footer rows still satisfy `assert_clear_cell`. Keep the existing assertions
that prompt text and composer cursor begin at column `1`.

- [ ] **Step 2: Require the user timestamp at terminal column 3**

Update `timestamp_uses_the_existing_top_prompt_row_at_a_fixed_offset` to render a
known timestamp and assert:

```rust
assert_eq!(buffer[(3, timestamp_row)].symbol(), "1");
assert_eq!(buffer[(1, timestamp_row)].symbol(), " ");
assert_eq!(buffer[(2, timestamp_row)].symbol(), " ");
```

Also assert that both terminal-edge cells on this row carry the prompt gray
background. Require the answer timestamp to use the same column `3` alignment.

- [ ] **Step 3: Require wrapped and sticky prompt backgrounds to reach both edges**

Rename `wrapped_prompt_rows_fill_only_the_content_band_between_gutters` to
`wrapped_prompt_rows_fill_the_full_terminal_width` and require every cell from
`0..width` on each prompt-filled row to have `Color::Rgb(52, 53, 54)`.

Add a sticky-history regression that scrolls within a long answer, verifies the
sticky prompt copy occupies the first visible rows, and requires its edge cells
to carry the same prompt background.

- [ ] **Step 4: Preserve narrow-terminal behavior**

Keep `sub_three_cell_widths_render_without_painting_or_panicking` unchanged so
widths `1` and `2` must remain entirely clear.

- [ ] **Step 5: Run the focused renderer tests and verify RED**

Run:

```bash
cargo test --locked --test ui_render ordinary_view_uses_one_clear_cell_on_both_horizontal_edges
cargo test --locked --test ui_render timestamp_uses_the_existing_top_prompt_row_at_a_fixed_offset
cargo test --locked --test ui_render wrapped_prompt_rows_fill_the_full_terminal_width
cargo test --locked --test ui_render sticky_prompt_background_reaches_both_terminal_edges
```

Expected: failures show clear outer prompt cells and a timestamp beginning at
column `1`; the narrow-width regression remains green.

### Task 2: Paint prompt backgrounds independently from text gutters

**Files:**
- Modify: `src/ui/render.rs:141-204`
- Test: `tests/ui_render.rs`

- [ ] **Step 1: Add a renderer helper for prompt edge fills**

Add a focused helper that receives the full frame area, visible history area,
and visible rows. Return immediately when the full width is below `3`. For each
visible row with `Some(fill)`, style the blank cell at `frame_area.x` and the
blank cell at `frame_area.right() - 1` using `ratatui_style(fill)`.

```rust
fn render_prompt_edge_fills(
    frame: &mut Frame<'_>,
    frame_area: Rect,
    history_area: Rect,
    rows: &[VisualRow],
) {
    if frame_area.width < 3 {
        return;
    }
    for (offset, row) in rows.iter().enumerate() {
        let Some(fill) = row.fill else { continue };
        let Ok(offset) = u16::try_from(offset) else { break };
        let Some(y) = history_area.y.checked_add(offset) else { break };
        if y >= history_area.bottom() { break; }
        let style = ratatui_style(fill);
        frame.buffer_mut()[(frame_area.x, y)].set_style(style);
        frame.buffer_mut()[(frame_area.right() - 1, y)].set_style(style);
    }
}
```

- [ ] **Step 2: Call the helper after rendering history text**

Retain `horizontal_content_area(frame.area())` for layout and wrapping. Save the
outer `frame.area()` before creating the content area, render the history
paragraph normally, then call `render_prompt_edge_fills` with the visible rows.
Do not paint edges for answer, Working, composer, footer, error, or blocked rows.

- [ ] **Step 3: Run the background-focused tests and verify GREEN**

Run:

```bash
cargo test --locked --test ui_render ordinary_view_uses_one_clear_cell_on_both_horizontal_edges
cargo test --locked --test ui_render wrapped_prompt_rows_fill_the_full_terminal_width
cargo test --locked --test ui_render sticky_prompt_background_reaches_both_terminal_edges
cargo test --locked --test ui_render sub_three_cell_widths_render_without_painting_or_panicking
```

Expected: all selected tests pass.

### Task 3: Move both timestamps two cells right

**Files:**
- Modify: `src/ui/visual_rows.rs:118-153`
- Modify: `src/ui/visual_rows.rs:597-621`
- Test: `tests/ui_render.rs`

- [ ] **Step 1: Give timestamp rows an explicit left padding parameter**

Extend `filled_timestamp_row` with `left_padding: usize`. Clip the timestamp to
`width.saturating_sub(left_padding)`, and only for a non-empty timestamp prepend
`" ".repeat(left_padding)` to its span text. Missing or invalid timestamps must
still produce an empty span list.

- [ ] **Step 2: Apply padding to user and answer timestamps**

Pass `2` for both the prompt timestamp row and the answer timestamp row.
Keep the row fill and timestamp color unchanged.

- [ ] **Step 3: Run timestamp and document regressions**

Run:

```bash
cargo test --locked --test ui_render timestamp_uses_the_existing_top_prompt_row_at_a_fixed_offset
cargo test --locked --test ui_render narrow_timestamp_is_clipped_to_one_gray_row_without_growing_the_prompt
cargo test --locked --test ui_render answer_timestamp_is_an_undimmed_gray_row_above_the_styled_body
cargo test --locked --test ui_render missing_or_invalid_timestamp_leaves_the_existing_top_gray_row_blank
```

Expected: all selected tests pass and both timestamp types start at column `3`.

### Task 4: Review, verify, integrate, and reload

**Files:**
- Review: `src/ui/render.rs`
- Review: `src/ui/visual_rows.rs`
- Review: `tests/ui_render.rs`
- Review: `docs/superpowers/specs/2026-08-14-prompt-background-and-date-alignment-design.md`

- [ ] **Step 1: Run the review checkpoint**

Invoke `superpowers:requesting-code-review` against the complete branch diff and
resolve every Critical or Important finding before continuing.

- [ ] **Step 2: Run fresh full verification**

```bash
cargo fmt --check
git diff --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

Expected: every command exits `0`, with zero test failures and zero Clippy
warnings.

- [ ] **Step 3: Commit the implementation**

```bash
git add src/ui/render.rs src/ui/visual_rows.rs tests/ui_render.rs \
  docs/superpowers/plans/2026-08-14-prompt-background-and-date-alignment.md
git commit -m "fix prompt background alignment"
```

- [ ] **Step 4: Merge and reload the local plugin**

Fast-forward `fix/prompt-background-gutter` into local `main`, repeat the full
verification from `main`, run:

```bash
herdr plugin link /Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts
herdr plugin list --plugin herdr.simple-prompts
```

Then remove the clean merged worktree and local task branch. Do not toggle any
currently active pane during verification.

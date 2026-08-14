# Global Horizontal Gutter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every ordinary and blocked Simple Prompts surface one empty terminal cell on each horizontal edge.

**Architecture:** Derive one safe horizontally inset `Rect` at the root of each render path, then let the existing child layouts, wrapping, hyperlink projection, and cursor calculations consume that rectangle. Keep the gutter out of message strings and widget-local padding so every surface shares one coordinate system.

**Tech Stack:** Rust 1.85+, Ratatui 0.29, Crossterm 0.28, Cargo test harness

---

## File map

- Modify `src/ui/render.rs`: own the shared root content rectangle and suppress a cursor when that rectangle has zero width.
- Modify `tests/ui_render.rs`: lock down ordinary, blocked, prompt-band, wrapping, link/cursor, and narrow-width behavior.
- No manifest, dependency, state, transcript, plugin configuration, or README changes.

### Task 1: Specify the global gutter in renderer tests

**Files:**
- Modify: `tests/ui_render.rs:18-24`
- Modify: `tests/ui_render.rs:470-535`
- Modify: `tests/ui_render.rs:1108-1134`
- Modify: `tests/ui_render.rs:1214-1235`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before editing tests.
- Invoke `superpowers:requesting-code-review` after the renderer batch is green.
- Invoke `superpowers:verification-before-completion` before marking the task complete.

- [ ] **Step 1: Read both target files completely**

Run:

```bash
sed -n '1,940p' src/ui/render.rs
sed -n '1,1320p' tests/ui_render.rs
```

Expected: the entire current renderer and public render-test file are read before either is modified.

- [ ] **Step 2: Add a reusable gutter assertion to the test file**

Add below `rendered_buffer`:

```rust
fn assert_clear_horizontal_gutters(buffer: &Buffer, width: u16, height: u16) {
    assert!(width >= 2);
    for y in 0..height {
        for x in [0, width - 1] {
            assert_clear_cell(buffer, x, y);
        }
    }
}

fn assert_clear_cell(buffer: &Buffer, x: u16, y: u16) {
    let cell = &buffer[(x, y)];
    let style = cell.style();
    assert_eq!(cell.symbol(), " ", "painted gutter at ({x}, {y})");
    assert!(matches!(style.fg, None | Some(Color::Reset)));
    assert!(matches!(style.bg, None | Some(Color::Reset)));
    assert!(style.add_modifier.is_empty() && style.sub_modifier.is_empty());
}
```

- [ ] **Step 3: Add an ordinary-surface regression**

Add a test that renders history, a working row, composer text, footer, and cursor together:

```rust
#[test]
fn ordinary_view_uses_one_clear_cell_on_both_horizontal_edges() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text("u1", "abcdefghijklmnop", None)));
    app.apply(AppEvent::NativeFinal(Message::final_text("a1", "answer", None)));
    app.agent_status = AgentStatus::Working;
    app.working_since = Some(Instant::now());
    let editor = Editor::default();

    let (buffer, cursor) = render_terminal_to_buffer(&app, &editor, 16, 12);

    assert_clear_horizontal_gutters(&buffer, 16, 12);
    assert_eq!(buffer[(1, 1)].symbol(), "a");
    assert_eq!(buffer[(14, 1)].symbol(), "n");
    assert_eq!(buffer[(1, 2)].symbol(), "o");
    assert_eq!(cursor.0, 1);
}
```

This proves the 16-cell frame wraps the prompt at the 14-cell content width while all ordinary rows share the gutter.

- [ ] **Step 4: Update existing coordinate-sensitive expectations**

In `wrapped_prompt_rows_fill_the_full_band_background`, rename the test to `wrapped_prompt_rows_fill_only_the_content_band_between_gutters`, locate the prompt at column `1`, assert both gutter cells have default style, and restrict the gray-background loop to `1..width - 1`:

```rust
let first = (0..height)
    .find(|&row| buffer[(1, row)].symbol() == "a")
    .expect("prompt row should be visible");
for row in block_start..block_start + gray_rows {
    assert_clear_cell(&buffer, 0, row);
    assert_clear_cell(&buffer, width - 1, row);
    for column in 1..width - 1 {
        assert_eq!(
            buffer[(column, row)].style().bg,
            Some(Color::Rgb(52, 53, 54)),
        );
    }
}
```

Change the OSC/cursor regression from `assert_eq!(cursor, (0, 11));` to:

```rust
assert_eq!(cursor, (1, 11));
```

Update the internal Crossterm byte assertion from ANSI column `1` to ANSI
column `2`, preserving its requirement that cursor restoration occurs only
after the balanced OSC 8 close sequence.

In `blocked_snapshot_styles_are_sanitized_and_confined_to_body`, move the owned body/footer coordinates to the shared content origin:

```rust
let danger = (1, 1);
let footer = (1, 7);
```

- [ ] **Step 5: Add blocked and narrow-width regressions**

Add:

```rust
#[test]
fn blocked_view_uses_the_same_clear_horizontal_gutters() {
    let mut app = AppState {
        agent_status: AgentStatus::Blocked,
        ..AppState::default()
    };
    app.blocked_surface = Some(Ok(StyledText {
        text: "Allow command?\n  Yes\n  No".into(),
        runs: Vec::new(),
    }));

    let buffer = rendered_buffer(&app, 32, 8);

    assert_clear_horizontal_gutters(&buffer, 32, 8);
    assert_eq!(buffer[(1, 0)].symbol(), "I");
    assert_eq!(buffer[(1, 1)].symbol(), "A");
}

#[test]
fn sub_three_cell_widths_render_without_painting_or_panicking() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text("u1", "prompt", None)));

    for width in 1..3 {
        let buffer = rendered_buffer(&app, width, 8);
        for y in 0..8 {
            for x in 0..width {
                assert_clear_cell(&buffer, x, y);
            }
        }
    }
}
```

- [ ] **Step 6: Run the focused tests and observe RED**

Run:

```bash
cargo test --locked --test ui_render ordinary_view_uses_one_clear_cell_on_both_horizontal_edges -- --exact
cargo test --locked --test ui_render blocked_view_uses_the_same_clear_horizontal_gutters -- --exact
cargo test --locked --test ui_render sub_three_cell_widths_render_without_painting_or_panicking -- --exact
```

Expected before implementation: ordinary and blocked gutter assertions fail because column zero is painted; the narrow-width regression may additionally expose unsafe cursor/layout behavior. Do not change the tests to fit existing output.

### Task 2: Apply one shared root content rectangle

**Files:**
- Modify: `src/ui/render.rs:130-280`
- Modify: `src/ui/render.rs:390-450`
- Test: `tests/ui_render.rs`

**Required skill checkpoints:**
- Continue the active `superpowers:test-driven-development` RED/GREEN cycle.
- Invoke `superpowers:requesting-code-review` after focused and full renderer tests pass.
- Invoke `superpowers:verification-before-completion` before the implementation commit.

- [ ] **Step 1: Add the safe content-area helper**

Add near the render effects definitions:

```rust
fn horizontal_content_area(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(u16::from(area.width > 0)),
        y: area.y,
        width: area.width.saturating_sub(2),
        height: area.height,
    }
}
```

This yields the exact one-cell inset for widths of three or more and a zero-width content rectangle for widths below three.

- [ ] **Step 2: Route both root render paths through the helper**

In ordinary `render`, replace:

```rust
let area = frame.area();
```

with:

```rust
let area = horizontal_content_area(frame.area());
```

Make the same replacement at the start of `render_blocked`. Do not add spaces to strings or padding to individual paragraphs.

- [ ] **Step 3: Avoid placing a cursor into an empty content rectangle**

Change the ordinary cursor guard to:

```rust
if app.input_enabled && composer_guard.is_none() && areas[3].width > 0 {
```

The existing `editor_cursor` behavior remains unchanged for non-empty content areas.

- [ ] **Step 4: Run the focused renderer tests and observe GREEN**

Run:

```bash
cargo test --locked --test ui_render ordinary_view_uses_one_clear_cell_on_both_horizontal_edges -- --exact
cargo test --locked --test ui_render blocked_view_uses_the_same_clear_horizontal_gutters -- --exact
cargo test --locked --test ui_render sub_three_cell_widths_render_without_painting_or_panicking -- --exact
cargo test --locked --test ui_render wrapped_prompt_rows_fill_only_the_content_band_between_gutters -- --exact
cargo test --locked --test ui_render terminal_draw_emits_balanced_osc_8_and_restores_the_composer_cursor -- --exact
```

Expected: all five focused tests pass.

- [ ] **Step 5: Run the complete renderer suite**

Run:

```bash
cargo test --locked --test ui_render
```

Expected: all renderer tests pass with no warnings or failures.

- [ ] **Step 6: Request code review and resolve findings**

Review `git diff -- src/ui/render.rs tests/ui_render.rs` for:

- both ordinary and blocked paths using the same helper;
- hyperlinks and cursor deriving coordinates only from inset child rectangles;
- no message-string padding;
- exact one-cell gutters at normal widths;
- safe behavior below three columns.

If review changes production behavior, add or adjust a failing regression before editing production code, then repeat focused and renderer tests.

- [ ] **Step 7: Format, check, and commit the renderer change**

Run:

```bash
cargo fmt --check
git diff --check
cargo test --locked --test ui_render
```

Expected: all commands exit zero.

Commit:

```bash
git add src/ui/render.rs tests/ui_render.rs
git commit -m "add global horizontal gutter"
```

### Task 3: Verify, merge, and reload the source-only plugin

**Files:**
- Verify only: complete branch
- No additional source files expected

**Required skill checkpoints:**
- Invoke `superpowers:requesting-code-review` for the complete `main...HEAD` range.
- Invoke `superpowers:verification-before-completion` before any completion claim.
- Invoke `superpowers:finishing-a-development-branch` for merge and worktree cleanup.

- [ ] **Step 1: Run the full branch verification matrix**

Run:

```bash
cargo fmt --check
git diff main...HEAD --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

Expected: formatting and diff checks are clean, all tests pass, Clippy emits no warnings, and the release build succeeds.

- [ ] **Step 2: Review the complete committed branch**

Review `git diff main...HEAD` for spec coverage, scope, terminal-coordinate correctness, and test adequacy. Resolve every actionable finding through RED/GREEN before proceeding.

- [ ] **Step 3: Fast-forward the clean branch into `main`**

From the repository root:

```bash
git status --short --branch
git merge --ff-only feature/global-horizontal-gutter
```

Expected: clean `main` fast-forwards to the renderer commit.

- [ ] **Step 4: Reverify the merged checkout**

Run on `main`:

```bash
cargo fmt --check
git diff --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

Expected: the merged checkout passes the same release gate.

- [ ] **Step 5: Relink without changing active panes**

Run:

```bash
herdr plugin link /Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts
herdr plugin list --plugin herdr.simple-prompts
```

Expected: `herdr.simple-prompts` is enabled and linked to the main repository root. Do not invoke the toggle command globally; the user will close and reopen the desired overlay with `prefix+m`.

- [ ] **Step 6: Clean up the merged worktree and branch**

After confirming the worktree is clean and merged:

```bash
git worktree remove /Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts/.worktrees/global-horizontal-gutter
git branch -d feature/global-horizontal-gutter
```

Expected: only the main worktree remains for this task and the merged feature branch is removed.

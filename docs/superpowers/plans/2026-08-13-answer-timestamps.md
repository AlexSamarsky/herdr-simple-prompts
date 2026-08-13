# Final Answer Timestamps Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to execute this plan task by task. Keep the TDD, review, verification, and branch-finishing checkpoints below.

**Goal:** Show a local `DD.MM.YYYY HH:MM` timestamp on its own dim, unboxed row immediately above every final answer that has a valid persisted timestamp.

**Architecture:** Extend the existing `HistoryDocument` visual-row projection only. Reuse the same injected timestamp formatter and `filled_timestamp_row` clipping/style helper already used by prompts, but append the answer metadata row only after successful formatting and with `fill: None`. This keeps the answer's native ANSI or Markdown-fallback body untouched and requires no persistence migration.

**Tech Stack:** Rust 2024, Ratatui, Chrono, Cargo integration tests.

---

### Task 1: Specify final-answer timestamp rows with failing UI tests

**Files:**

- Modify: `tests/ui_render.rs:20-170`
- Verify: `tests/ui_render.rs:755-835`

- [ ] **Step 1: Preserve the existing prompt-only structural tests**

Change the answer timestamp in `prompt_is_a_label_free_gray_block_and_answer_is_unboxed` from `Some(2)` to `None`, and change the answer timestamp in `timestamp_uses_the_existing_top_prompt_row_at_a_fixed_offset` to `None`. Keep their current row indexes and assertions so the first test continues to isolate the pre-existing answer-body contract and the second continues to isolate prompt timestamp placement. The dedicated tests below own the new answer timestamp expectations.

- [ ] **Step 2: Add a deterministic answer timestamp placement/style test**

Add a test named `answer_timestamp_is_a_dim_unboxed_row_above_the_styled_body`. Build a prompt with no timestamp and a `MessagePresentation::NativeAnsi` answer whose timestamp is `Some(1_786_638_720_000)`, then render with `HistoryDocument::from_app_at_offset(..., FixedOffset::east_opt(3 * 60 * 60).unwrap())`.

Assert all of the following:

```rust
assert_eq!(document.rows[3].plain_text(), "13.08.2026 19:32");
assert!(document.rows[3].fill.is_none());
assert_eq!(
    document.rows[3].spans[0].style.foreground,
    Some(AnsiColor::BrightBlack)
);
assert!(document.rows[3].spans[0].style.modifiers.dim);
assert_eq!(document.rows[4].plain_text(), "Native answer");
assert_eq!(document.rows[4].spans[0].style.foreground, Some(AnsiColor::Green));
assert!(document.rows[4].spans[0].style.modifiers.bold);
```

This proves the metadata row has the agreed style while the answer retains its owned native presentation.

- [ ] **Step 3: Add narrow-width, absent/invalid, and hydrated-history cases**

Add tests with the common prefix `answer_timestamp_`:

1. `answer_timestamp_is_clipped_to_one_visual_row` renders at width `8` and asserts the metadata row is exactly `13.08.20`, has cell width `8`, and is followed by the answer body.
2. `answer_timestamp_is_omitted_without_a_valid_value` loops over `None` and `Some(u64::MAX)`, asserts the row count remains the existing prompt-top/prompt/prompt-bottom/answer/gap shape, and asserts the answer body immediately follows the prompt block with no blank metadata row.
3. `answer_timestamp_survives_visible_history_hydration` hydrates matching `VisibleRole::Prompt` and `VisibleRole::Final` version-2 records, gives the final record the fixed timestamp above, and asserts the timestamp row and saved answer text appear consecutively.

Use `PersistedPresentation::Fallback` for the hydrated final record and the existing `fingerprint` helper for both records. Do not add a history schema or migration fixture because `VisibleHistoryRecord::timestamp_ms` is already persisted.

- [ ] **Step 4: Run the focused tests and confirm the red state**

Run:

```bash
cargo test --test ui_render answer_timestamp -- --nocapture
```

Expected: the new tests fail because `HistoryDocument` currently begins directly with the final-answer body. If they pass before production code changes, stop and correct the assertions so they exercise the missing row.

- [ ] **Step 5: Commit only after the production change in Task 2**

Keep the failing tests uncommitted while performing the minimal implementation; the green test and implementation form one logical commit.

### Task 2: Add the answer timestamp row in the visual-row projection

**Files:**

- Modify: `src/ui/visual_rows.rs:105-142`
- Test: `tests/ui_render.rs`

- [ ] **Step 1: Implement the smallest conditional row insertion**

Immediately before `answer_lines(answer)` is projected, format the final answer's owned timestamp and append a normal-surface metadata row only when formatting succeeds:

```rust
if let Some(answer) = &turn.final_answer {
    if let Some(timestamp) = format_timestamp(answer.timestamp_ms) {
        document.rows.push(filled_timestamp_row(
            Some(timestamp.as_str()),
            None,
            width,
        ));
    }
    for line in &answer_lines(answer) {
        push_styled_rows(&mut document.rows, line, None, width);
    }
}
```

Do not push `filled_timestamp_row(None, None, width)`: that would create the forbidden empty gap for legacy/invalid timestamps. Do not change `answer_lines`, Markdown styling, ANSI style runs, sticky-section boundaries, or scroll math.

- [ ] **Step 2: Run the focused tests and confirm the green state**

Run:

```bash
cargo test --test ui_render answer_timestamp -- --nocapture
```

Expected: all answer-timestamp tests pass.

- [ ] **Step 3: Run the complete renderer test target**

Run:

```bash
cargo test --test ui_render
```

Confirm the existing prompt timestamp, sticky prompt, bottom-following, narrow long-answer, Markdown fallback, and native ANSI tests remain green.

- [ ] **Step 4: Request code review and resolve findings**

Use `superpowers:requesting-code-review` against the branch diff. Review specifically for accidental blank rows, changed answer presentation, width overflow, and changes to prompt/sticky behavior. Apply any valid findings through the same red-green loop.

- [ ] **Step 5: Commit the tested code change**

Run:

```bash
git add src/ui/visual_rows.rs tests/ui_render.rs
git commit -m "show final answer timestamps"
```

### Task 3: Document the visible contract and verify the source-only release

**Files:**

- Modify: `README.md:78-99`
- Modify: `README.md:286-309`

- [ ] **Step 1: Update the conversation-view contract**

Replace the final-answer bullet with wording equivalent to:

```markdown
- Each final answer begins with its local `DD.MM.YYYY HH:MM` timestamp on one
  dim, unboxed row on the normal terminal surface, followed immediately by the
  styled answer text. There is no `ANSWER` label or answer box. Legacy records
  without a valid timestamp add no metadata row or empty gap.
```

- [ ] **Step 2: Update the manual smoke test**

In step 2, add an explicit check that the dim local timestamp appears immediately above the final answer, that it has no gray fill/box, and that the answer formatting still matches the native pane.

- [ ] **Step 3: Run formatting and all automated quality gates**

Run from the worktree root:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
git diff --check
```

Expected: formatting and Clippy are clean, all tests pass, the release binary builds from the locked graph, and the diff has no whitespace errors.

- [ ] **Step 4: Commit the documentation update**

Run:

```bash
git add README.md
git commit -m "document final answer timestamps"
```

### Task 4: Finish the branch, merge, and refresh the local plugin

**Files:**

- Verify: branch diff and local Herdr plugin installation

- [ ] **Step 1: Perform the completion audit**

Use `superpowers:verification-before-completion`, then inspect:

```bash
git status --short
git log --oneline main..HEAD
git diff --stat main...HEAD
git diff --check main...HEAD
```

Require a clean worktree and only the design, plan, renderer tests/code, and README changes described here.

- [ ] **Step 2: Merge with the repository's established flow**

Use `superpowers:finishing-a-development-branch`. Because the user already requested merge and live plugin refresh for this plugin, merge `fix/answer-timestamps` into local `main` with a non-fast-forward merge after all checks pass. Do not rewrite or discard unrelated user changes.

- [ ] **Step 3: Reverify and rebuild from merged `main`**

From `/Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts` run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
herdr plugin link . --enabled
herdr server reload-config
```

Expected: the merged source passes the same gates, Herdr links the rebuilt source-only plugin, and configuration reload succeeds.

- [ ] **Step 4: Refresh and smoke-test the current overlay**

Close and reopen the current Simple Prompts overlay with `prefix+m` so the running pane uses the rebuilt binary. In a synthetic turn, confirm one dim `DD.MM.YYYY HH:MM` row appears immediately above the final answer, the timestamp is absent rather than blank for a legacy fixture, the answer's Markdown/ANSI styling is unchanged, and scrolling still reaches the final row.

- [ ] **Step 5: Report the result**

Report the merge commit, exact automated checks, whether the live Codex smoke test was performed, and that no history migration was needed. Do not expose native session IDs, transcript content, or pane-private state.

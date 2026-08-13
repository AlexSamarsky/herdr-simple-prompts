# Clickable Markdown Links Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan inline. Before coding, invoke `superpowers:test-driven-development`. After the meaningful coding batch, invoke `superpowers:requesting-code-review`. Before any completion claim, invoke `superpowers:verification-before-completion`. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make safe Markdown HTTP(S) labels clickable in supporting terminals without exposing destinations or changing persisted history.

**Architecture:** Extend Markdown projection with ephemeral safe hyperlink ranges, carry them through answer wrapping as visual-span metadata, and redraw only those spans through the terminal backend with balanced OSC 8 sequences after Ratatui renders the plain frame. Reuse exact Markdown/native visible-text matching for captured native answers.

**Tech Stack:** Rust 2024, Ratatui 0.29, Crossterm 0.28, existing `unicode-width` support, Cargo integration tests.

---

### Task 1: Specify safe hyperlink projection

**Files:**

- Modify: `tests/ansi_style.rs`
- Modify: `src/markdown.rs`

**Required skill checkpoints:** TDD before production changes; review after the batch; verification before completion.

- [ ] Add failing tests proving HTTP(S) targets produce projected hyperlink ranges, unsupported schemes become ordinary labels, and terminal controls never become hyperlink targets.
- [ ] Run `cargo test --test ansi_style markdown_hyperlink -- --nocapture` and confirm failure because hyperlink metadata does not exist.
- [ ] Add `MarkdownProjection` and `HyperlinkRange`, retain projected byte offsets while delimiters are removed, and validate only control-free HTTP(S) destinations.
- [ ] Keep `style_markdown` as the compatibility wrapper returning only `StyledText`.
- [ ] Re-run the focused tests and confirm green.

### Task 2: Preserve hyperlink metadata through visual rows

**Files:**

- Modify: `tests/ui_render.rs`
- Modify: `src/ui/visual_rows.rs`

**Required skill checkpoints:** TDD before production changes; review after the batch; verification before completion.

- [ ] Add failing tests proving safe URLs remain attached to Unicode/wrapped label spans and unsupported schemes have no hyperlink metadata.
- [ ] Add a native-ANSI test proving an exact Markdown projection adds the URL without replacing native style.
- [ ] Run the focused renderer tests and confirm the missing metadata failures.
- [ ] Add ephemeral `VisualSpan.hyperlink`, hyperlink-aware wrapping, and exact native-projection reuse.
- [ ] Re-run the focused tests and confirm green.

### Task 3: Emit balanced OSC 8 on the real terminal path

**Files:**

- Modify: `tests/ui_render.rs`
- Modify: `src/ui/render.rs`
- Modify: `src/ui/mod.rs`

**Required skill checkpoints:** TDD before production changes; review after the batch; verification before completion.

- [ ] Add a failing TestBackend test for exact `ESC ] 8 ;; URL BEL label ESC ] 8 ;; BEL` output and restored composer cursor.
- [ ] Run the focused test and confirm failure because the real terminal path currently emits ordinary labels only.
- [ ] Return hyperlink redraw effects from the renderer, add a backend-generic terminal draw helper, and switch the live UI loop to it.
- [ ] Keep ordinary test-buffer rendering free of OSC so existing layout assertions remain readable.
- [ ] Re-run the focused test and renderer suite.

### Task 4: Document, review, verify, merge, and refresh

**Files:**

- Modify: `README.md`

**Required skill checkpoints:** `requesting-code-review`, then `verification-before-completion`, then `finishing-a-development-branch`.

- [ ] Document terminal OSC 8 support, the HTTP(S)-only safety boundary, and graceful fallback for unsupported terminals/schemes.
- [ ] Review the branch diff for injection, width/wrap regressions, native-style loss, and persistence changes; fix valid findings through red-green tests.
- [ ] Run `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features`, `cargo build --locked --release`, and `git diff --check`.
- [ ] Commit the feature, merge it into local `main`, rebuild/relink the source-only plugin, reload Herdr configuration, and refresh only the current Simple Prompts overlay.

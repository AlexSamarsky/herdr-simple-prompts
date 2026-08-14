# Local File Links Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Match Herdr's local-file link presentation by showing safe absolute paths, retaining native capture styling, and emitting guarded `file:///...` OSC 8 targets.

**Architecture:** The Markdown layer classifies link destinations and owns visible-text projection plus target construction. Native capture consumes that projected path unchanged, while the renderer independently allow-lists HTTP and local file URI targets before emitting OSC 8.

**Tech Stack:** Rust, Ratatui 0.29, Crossterm OSC 8, Cargo integration tests.

---

### Task 1: Project safe local paths and reject unsafe targets

**Files:**
- Modify: `src/markdown.rs:36-60`
- Modify: `src/markdown.rs:188-232`
- Modify: `src/markdown.rs:367-423`
- Test: `tests/ansi_style.rs:311-354`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before production edits.
- Invoke `superpowers:requesting-code-review` after the complete coding batch.
- Invoke `superpowers:verification-before-completion` before completion claims.

- [ ] **Step 1: Add RED projection coverage for a local path**

Add a test using:

```rust
let text = "Read [TDD](/Users/example/skills/тест/SKILL.md).";
let projected = style_markdown_with_links(text);

assert_eq!(
    projected.styled.text,
    "Read /Users/example/skills/тест/SKILL.md."
);
assert_eq!(projected.hyperlinks.len(), 1);
assert_eq!(
    projected.hyperlinks[0].url,
    "file:///Users/example/skills/тест/SKILL.md"
);
```

Assert that the full projected path range is cyan and underlined.

- [ ] **Step 2: Add RED rejection coverage**

Project candidates for `relative/file.md`, `//host/share.md`,
`file://host/share.md`, `/tmp/has space.md`, and a control-bearing absolute
path. Assert that none creates `HyperlinkRange` metadata and that control bytes
never reach projected text.

- [ ] **Step 3: Run projection tests and verify RED**

```bash
cargo test --locked --test ansi_style markdown_local_file_links
```

Expected: the safe-path test fails because the label `TDD` remains visible and
no hyperlink is created; rejection tests remain safe.

- [ ] **Step 4: Implement explicit link classification**

Replace the borrowed label-only `SourceLink` fields with owned visible-range
metadata:

```rust
struct SourceLink {
    visible_start: usize,
    visible_end: usize,
    clickable_url: Option<String>,
}
```

Add a small classifier:

```rust
enum LinkTarget<'a> {
    Http(&'a str),
    AbsolutePath(&'a str),
}

fn classify_link_target(target: &str) -> Option<LinkTarget<'_>>;
```

HTTP accepts the existing `is_safe_http_url` contract. `AbsolutePath` requires
exactly one leading `/`, at least one following character, and no whitespace,
control bytes, `#`, or `?`. It performs no filesystem access.

- [ ] **Step 5: Project the destination for local paths**

For HTTP, keep the label as the visible range and the original URL as target.
For `AbsolutePath`, keep destination bytes visible, discard the label and
Markdown delimiters, style the destination with `link_style()`, and store
`format!("file://{path}")` as target. Non-clickable candidates retain the label
and no target.

- [ ] **Step 6: Run projection tests and verify GREEN**

```bash
cargo test --locked --test ansi_style markdown_local_file_links
cargo test --locked --test ansi_style markdown_hyperlink_projection_accepts_only_safe_http_targets
```

Expected: both new local-path and existing HTTP suites pass.

### Task 2: Restore exact native capture for Codex file links

**Files:**
- Test: `src/ui/runtime.rs:840-884`
- Production behavior reused from: `src/ui/runtime.rs:503-526`

**Required skill checkpoints:**
- Continue the active `superpowers:test-driven-development` cycle.
- Include this checkpoint in the batch review.
- Include the test in the final full verification.

- [ ] **Step 1: Add a capture regression**

Add an internal runtime test with canonical Markdown:

```rust
let canonical = "Basis: [TDD](/Users/example/SKILL.md).";
```

Return a Codex ANSI surface whose visible final is:

```text
Basis: /Users/example/SKILL.md.
```

and whose path has blue/underlined ANSI styling. Require
`MessagePresentation::NativeAnsi`, exact projected path text, and retained link
style.

- [ ] **Step 2: Run the capture regression**

```bash
cargo test --locked ui::runtime::tests::capture_resolution_projects_local_file_path_before_exact_match
```

Expected: PASS after Task 1 because the Markdown projection now matches Codex's
visible path. No runtime production change should be necessary.

### Task 3: Emit only safe local OSC 8 targets

**Files:**
- Modify: `src/markdown.rs:406-423`
- Modify: `src/ui/render.rs:5`
- Modify: `src/ui/render.rs:320-354`
- Test: `tests/ui_render.rs:1058-1100`
- Test: `src/ui/render.rs:690-903`

**Required skill checkpoints:**
- Continue the active `superpowers:test-driven-development` cycle.
- Invoke `superpowers:requesting-code-review` after GREEN.
- Invoke `superpowers:verification-before-completion` before completion claims.

- [ ] **Step 1: Add RED renderer coverage for a balanced local-file OSC link**

Render a final answer containing
`[TDD](/Users/example/skills/тест/SKILL.md)`. Assert that the visible buffer uses
the full path and exactly one cell contains:

```text
ESC ] 8 ; ; file:///Users/example/skills/тест/SKILL.md BEL
```

followed by visible path text and a balanced OSC close.

- [ ] **Step 2: Add RED defense-in-depth coverage**

In the renderer unit-test module, construct `VisualRow` values with hyperlink
metadata for `file://host/share.md`, `file:////host/share.md`, and a
control-bearing file URI. Pass them to `hyperlink_patches` and require an empty
result. Also require `file:///Users/example/SKILL.md` to produce one patch.

- [ ] **Step 3: Run renderer tests and verify RED**

```bash
cargo test --locked --test ui_render local_file
cargo test --locked ui::render::tests::hyperlink_patches_allow_only_local_file_urls
```

Expected: safe file metadata is rejected by the existing HTTP-only render gate.

- [ ] **Step 4: Add the independent file-URI allow-list**

Keep `is_safe_http_url` and add:

```rust
pub(crate) fn is_safe_local_file_url(url: &str) -> bool;
pub(crate) fn is_safe_hyperlink_url(url: &str) -> bool;
```

`is_safe_local_file_url` accepts only `file://` followed by a safe absolute
path with exactly one leading slash in the remainder. `is_safe_hyperlink_url`
combines the HTTP and local-file rules. Update `hyperlink_patches` to use only
the combined validator.

- [ ] **Step 5: Run all link-focused tests and verify GREEN**

```bash
cargo test --locked --test ansi_style markdown_local_file_links
cargo test --locked --test ui_render local_file
cargo test --locked ui::runtime::tests::capture_resolution_projects_local_file_path_before_exact_match
cargo test --locked ui::render::tests::hyperlink_patches_allow_only_local_file_urls
```

Expected: all selected tests pass with balanced OSC output and rejected unsafe
targets.

### Task 4: Review, verify, integrate, and reload

**Files:**
- Review: `src/markdown.rs`
- Review: `src/ui/runtime.rs`
- Review: `src/ui/render.rs`
- Review: `tests/ansi_style.rs`
- Review: `tests/ui_render.rs`
- Review: `docs/superpowers/specs/2026-08-14-local-file-links-design.md`

- [ ] **Step 1: Run the review checkpoint**

Invoke `superpowers:requesting-code-review` against the full branch diff. Resolve
every Critical or Important finding, with particular attention to OSC injection,
remote file hosts, projected byte offsets, Unicode wrapping, and stale native
capture behavior.

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
git add src/markdown.rs src/ui/runtime.rs src/ui/render.rs \
  tests/ansi_style.rs tests/ui_render.rs \
  docs/superpowers/plans/2026-08-14-local-file-links.md
git commit -m "support local file links"
```

- [ ] **Step 4: Merge and reload the local plugin**

Fast-forward `fix/local-file-links` into local `main`, repeat the full gate from
`main`, and run:

```bash
herdr plugin link /Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts
herdr plugin list --plugin herdr.simple-prompts
```

Remove the clean merged worktree and local task branch. Do not toggle any active
pane during verification.

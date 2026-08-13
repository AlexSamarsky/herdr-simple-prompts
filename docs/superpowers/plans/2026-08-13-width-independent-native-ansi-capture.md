# Width-Independent Native ANSI Capture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve native Codex/Claude final-answer ANSI styles across source-pane wrapping and match the native Codex prompt-band tone.

**Architecture:** Keep the current exact line matcher as the preferred path. If it finds no candidate, scan provider-bounded final blocks, compare their non-whitespace Unicode scalar sequence with the projected transcript, and map styles from equal native scalars onto canonical byte ranges. Reject non-whitespace differences, duplicate candidates, unknown chrome, and unsafe footers. Independently replace only the prompt fill background with the measured native Codex RGB tone; prompt geometry remains unchanged.

**Tech Stack:** Rust 2024, existing dependency-free ANSI/Markdown projector, Cargo tests and Clippy.

---

### Task 1: Match the native Codex prompt-band tone

**Files:**
- Modify: `tests/ui_render.rs:209-238`
- Modify: `src/ui/visual_rows.rs:402-410`

**Required skill checkpoints:**
- Use `superpowers:test-driven-development` before editing `src/ui/visual_rows.rs`.
- Include this presentation change in the `superpowers:requesting-code-review` batch.
- Use `superpowers:verification-before-completion` before any parity claim.

- [ ] **Step 1: Change the rendered-buffer expectation and verify RED**

In `wrapped_prompt_rows_fill_the_full_band_background`, replace the terminal
palette expectation:

```rust
assert_eq!(buffer[(column, row)].style().bg, Some(Color::DarkGray));
```

with the sampled Codex reference tone:

```rust
assert_eq!(
    buffer[(column, row)].style().bg,
    Some(Color::Rgb(52, 53, 54)),
);
```

Run:

```bash
cargo test --test ui_render wrapped_prompt_rows_fill_the_full_band_background -- --exact
```

Expected: FAIL because the current `AnsiColor::BrightBlack` maps to
`Color::DarkGray` rather than the measured RGB color.

- [ ] **Step 2: Apply the exact prompt fill and verify GREEN**

Change only the background in `prompt_fill`:

```rust
fn prompt_fill() -> Option<CellStyle> {
    Some(CellStyle {
        foreground: Some(AnsiColor::BrightWhite),
        background: Some(AnsiColor::Rgb(52, 53, 54)),
        modifiers: StyleModifiers::default(),
    })
}
```

Run the same focused test. Expected: PASS with all prompt rows, padding, and
right-edge cells using `Color::Rgb(52, 53, 54)`.

### Task 2: Lock the width-independent capture contract with failing tests

**Files:**
- Modify: `tests/ansi_style.rs:552-582`
- Modify: `tests/ansi_style.rs:584-642`

**Required skill checkpoints:**
- Use `superpowers:test-driven-development` before editing production code.
- Use `superpowers:requesting-code-review` after the coding batch in Task 3.
- Use `superpowers:verification-before-completion` before any success claim.

- [ ] **Step 1: Add a failing soft-wrap/no-leading-separator regression**

Add this test after the current projected-visible-text capture test:

```rust
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
```

- [ ] **Step 2: Add failing fenced-shell style and safety regressions**

Add tests proving canonical logical lines survive native wrapping and safety is
not relaxed beyond whitespace:

```rust
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
fn width_independent_native_capture_rejects_content_changes_and_duplicates() {
    let changed = "• same answer\n  changed token\n────────\n› Write a prompt";
    assert!(
        extract_native_final(changed, "same answer\nexpected token", AgentKind::Codex).is_none()
    );

    let duplicate = concat!(
        "• same answer\n────────\n› Write a prompt\n",
        "• same answer\n────────\n› Write a prompt",
    );
    assert!(extract_native_final(duplicate, "same answer", AgentKind::Codex).is_none());
}
```

Remove the old `// Partial scrollback misses the leading boundary.` case from
`native_final_capture_rejects_unsafe_or_non_exact_candidates`; the approved
contract now accepts a complete provider-bounded answer without that separator.

- [ ] **Step 3: Run the focused tests and verify RED**

Run:

```bash
cargo test --test ansi_style native_final_capture_ignores_physical_wraps_without_a_leading_separator -- --exact
cargo test --test ansi_style native_final_capture_maps_wrapped_shell_styles_to_projected_markdown -- --exact
```

Expected: both fail because `extract_native_final` returns `None`; this proves
the regressions exercise missing behavior rather than existing strict capture.

### Task 3: Add the width-independent candidate matcher

**Files:**
- Modify: `src/ansi.rs:69-167`
- Modify: `src/ansi.rs:169-267`
- Test: `tests/ansi_style.rs`

**Required skill checkpoints:**
- Continue the active `superpowers:test-driven-development` RED/GREEN cycle.
- Invoke `superpowers:requesting-code-review` after focused tests are green.
- Invoke `superpowers:verification-before-completion` before marking the task complete.

- [ ] **Step 1: Introduce scalar and candidate helpers**

Add a byte-safe scalar mapping type beside `LineRange`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ScalarRange {
    character: char,
    start: usize,
    end: usize,
}

fn non_whitespace_scalars(text: &str, start: usize, end: usize) -> Vec<ScalarRange> {
    text[start..end]
        .char_indices()
        .filter_map(|(offset, character)| {
            (!character.is_whitespace()).then_some(ScalarRange {
                character,
                start: start + offset,
                end: start + offset + character.len_utf8(),
            })
        })
        .collect()
}
```

Add `reflow_candidate_runs`, which strips only reviewed provider prefixes,
requires every continuation row to use known continuation chrome, compares
non-whitespace scalar values exactly, and reuses `slice_mapped_runs`:

```rust
fn reflow_candidate_runs(
    sanitized: &StyledText,
    lines: &[LineRange],
    first: usize,
    trailing: usize,
    expected_visible: &str,
    chrome: &NativeChrome,
) -> Option<Vec<StyleRun>> {
    let mut source = Vec::new();
    for line_index in first..trailing {
        let range = lines[line_index];
        let line = line_text(&sanitized.text, range);
        let prefixes = if line_index == first {
            chrome.role_prefixes
        } else {
            chrome.continuation_prefixes
        };
        let content_start = if line.is_empty() {
            range.start
        } else {
            let prefix = prefixes.iter().find(|prefix| line.starts_with(**prefix))?;
            range.start + prefix.len()
        };
        source.extend(non_whitespace_scalars(
            &sanitized.text,
            content_start,
            range.end,
        ));
    }

    let destination = non_whitespace_scalars(expected_visible, 0, expected_visible.len());
    if source.len() != destination.len()
        || source
            .iter()
            .zip(&destination)
            .any(|(left, right)| left.character != right.character)
    {
        return None;
    }

    let mappings: Vec<_> = source
        .iter()
        .zip(destination)
        .map(|(source, destination)| (source.start, source.end, destination.start))
        .collect();
    Some(slice_mapped_runs(&sanitized.runs, &mappings))
}
```

- [ ] **Step 2: Preserve strict matching and add the fallback scan**

Refactor the existing strict loop into a helper without changing its matching
rules. In `extract_native_final`, use this decision order:

```rust
let strict_candidates = strict_final_candidates(
    &sanitized,
    &lines,
    &expected_visible_lines,
    chrome,
);
let candidate = match strict_candidates.len() {
    1 => strict_candidates.into_iter().next(),
    0 => {
        let mut reflow_candidates = Vec::new();
        for trailing in 0..lines.len() {
            if !is_trailing_boundary(line_text(&sanitized.text, lines[trailing]), chrome) {
                continue;
            }
            let composer = trailing + 1;
            if composer >= lines.len()
                || !starts_with_any(
                    line_text(&sanitized.text, lines[composer]),
                    chrome.composer_prefixes,
                )
            {
                continue;
            }
            for first in 0..trailing {
                if !starts_with_any(
                    line_text(&sanitized.text, lines[first]),
                    chrome.role_prefixes,
                ) {
                    continue;
                }
                if let Some(runs) = reflow_candidate_runs(
                    &sanitized,
                    &lines,
                    first,
                    trailing,
                    expected_visible,
                    chrome,
                ) {
                    reflow_candidates.push((runs, composer));
                }
            }
        }
        (reflow_candidates.len() == 1)
            .then(|| reflow_candidates.pop().expect("one reflow candidate"))
    }
    _ => None,
}?;
```

After candidate selection, validate every non-empty footer row with its complete
reviewed structure: a provider-appropriate model label, `·`, and an absolute or
home-relative working-directory field. Reject prefix-only lookalikes such as
`gpt-unreviewed payload` and `ClaudeInjected · /repo`. Return
`StyledText { text: expected_visible.to_owned(), runs }`. Do not modify runtime,
history, or rendering schemas.

- [ ] **Step 3: Run focused tests and verify GREEN**

Run:

```bash
cargo test --test ansi_style
```

Expected: all ANSI sanitizer, strict capture, soft-wrap capture, fenced-shell,
ambiguity, structured-footer, multibyte Unicode, Codex, and Claude tests pass.

- [ ] **Step 4: Confirm no additional refactor is required**

Read the diff and keep the strict and reflow matchers as separate private
helpers. Do not restructure the sanitizer, runtime, history, or renderer. Run:

```bash
cargo test --test ansi_style
```

Expected: all focused tests remain green.

- [ ] **Step 5: Review the coding batch**

Invoke `superpowers:requesting-code-review` against the diff from commit
`efd6145`, address only correctness or maintainability findings inside the
approved capture scope, and rerun `cargo test --test ansi_style` after any edit.

### Task 4: Verify, build, install, and persist the fix

**Files:**
- Modify only if verification exposes a scoped defect: `src/ansi.rs`, `tests/ansi_style.rs`
- Build output: `target/release/herdr-simple-prompts`

**Required skill checkpoints:**
- Use `superpowers:test-driven-development` for any verification-discovered code fix.
- Use `superpowers:requesting-code-review` after any additional coding batch.
- Use `superpowers:verification-before-completion` for the final commands and report.

- [ ] **Step 1: Run complete Rust verification**

Run fresh commands and inspect their exit codes:

```bash
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Expected: every target passes, Clippy reports no warnings, and rustfmt reports no
diff.

- [ ] **Step 2: Build the source-only release artifact**

Run:

```bash
cargo build --release
```

Expected: exit code 0 and an updated
`target/release/herdr-simple-prompts` executable.

- [ ] **Step 3: Commit the implementation**

Review `git diff --check` and `git status --short`, then commit only the approved
capture change, regressions, and this plan:

```bash
git add src/ansi.rs src/ui/visual_rows.rs tests/ansi_style.rs tests/ui_render.rs docs/superpowers/specs/2026-08-13-width-independent-native-ansi-capture-design.md docs/superpowers/plans/2026-08-13-width-independent-native-ansi-capture.md
git commit -m "preserve native styles across terminal wraps"
```

- [ ] **Step 4: Merge into `main` and rebuild the installed source-only plugin**

Verify `/Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts` is a
clean `main` worktree. Merge the verified branch and rebuild the exact release
path used by the plugin:

```bash
git -C /Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts merge --no-ff fix/native-rendered-presentation
cargo build --release --manifest-path /Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts/Cargo.toml
```

Expected: the merge exits successfully and the main-worktree release binary is
newer than any already-running overlay process.

- [ ] **Step 5: Reload the active overlay and report**

Confirm the Herdr plugin config still points to
`/Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts/target/release/herdr-simple-prompts`.
Close the currently open Simple Prompts overlay and reopen it with `prefix+m`; a
running overlay process cannot load the rebuilt executable in place. Do not
inject a synthetic prompt into the user's Codex session. Ask the user to check
the next naturally produced fenced shell-code final answer, then inspect only
its `presentation` field if further diagnosis is needed.

Verify both worktrees are clean and report:

- exact verification commands and counts;
- the implementation and merge commit hashes;
- that existing fallback history is not retroactively restyled;
- that newly captured answers use native ANSI when the normalized candidate is
  unique;
- that no CoachTM Obsidian note was changed because the implementation is
  isolated to the Herdr plugin repository.

# Public README, Session Recovery, Working Composer, and Native Selection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish a clear English project entrypoint and safe existing-session helper while fixing Codex Working-state composer classification and restoring Herdr-native drag selection with automatic copy-on-release.

**Architecture:** Keep the changes in four bounded units: extend only the Codex composer-boundary predicate; stop the nested Crossterm UI from capturing mouse input; provide a fail-closed Bash recovery helper with an isolated command-fixture harness; and reorganize the existing README around one authentic English screenshot and the public GitHub install path. Preserve the source-only manifest, transcript/history model, native-draft preflight, OSC 8 link path, and pane targeting.

**Tech Stack:** Rust 1.85 edition 2024, Crossterm 0.28.1, Ratatui 0.29.0, Bash, Herdr 0.7.5 CLI, `jq`, `rg`, GitHub Actions, Markdown.

**Design source:** `docs/superpowers/specs/2026-08-14-public-readme-session-recovery-working-composer-design.md`

---

## File map

- Modify `src/composer.rs`: accept one exact native Codex Working boundary.
- Modify `tests/composer.rs`: pin clear, occupied, valid elapsed, and malformed Working layouts.
- Modify `src/ui/terminal.rs`: enter/restore the nested TUI without mouse capture.
- Modify `src/ui/mod.rs`: remove plugin-owned mouse-event consumption while retaining keyboard scrolling.
- Create `scripts/register-existing-sessions.sh`: recover only unambiguous, transcript-backed Codex session identities.
- Create `tests/register-existing-sessions.sh`: exercise the helper through fake Herdr responses and filesystem fixtures.
- Modify `.github/workflows/ci.yml`: execute the helper contract test on every supported CI job.
- Create `assets/simple-prompts.png`: one cropped, synthetic English terminal capture.
- Modify `README.md`: marketplace-style introduction, exact installation, existing-session recovery, native selection, and corrected controls.

## Task 1: Recognize the live Codex Working composer boundary

**Files:**
- Modify: `tests/composer.rs`
- Modify: `src/composer.rs:170-202`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before changing the tests.
- Invoke `superpowers:requesting-code-review` after the composer batch is green.
- Invoke `superpowers:verification-before-completion` before marking this task done.

- [ ] **Step 1: Add failing regression tests for the exact live layout**

Append these tests to `tests/composer.rs`:

```rust
fn codex_working_surface(prompt: &str, elapsed: &str, separator: char, suffix: &str) -> String {
    format!(
        "• Working ({elapsed} {separator} {suffix})\n› {prompt}\ngpt-5.6-sol xhigh · /repo · weekly 47% left"
    )
}

#[test]
fn codex_working_boundary_accepts_a_dim_placeholder() {
    let text = codex_working_surface("Write a prompt", "10m 20s", '•', "esc to interrupt");
    let surface = styled_range(
        &text,
        "Write a prompt",
        AnsiColor::BrightBlack,
        true,
    );

    assert_eq!(
        classify_native_composer(AgentKind::Codex, &surface),
        NativeComposerState::Clear
    );
}

#[test]
fn codex_working_boundary_still_detects_unsent_text() {
    let surface = plain(&codex_working_surface(
        "unsent native text",
        "2s",
        '•',
        "esc to interrupt",
    ));

    assert_eq!(
        classify_native_composer(AgentKind::Codex, &surface),
        NativeComposerState::Occupied
    );
}

#[test]
fn codex_working_boundary_requires_the_exact_native_shape() {
    for surface in [
        codex_working_surface("Write a prompt", "eventually", '•', "esc to interrupt"),
        codex_working_surface("Write a prompt", "2m 3s", '·', "esc to interrupt"),
        codex_working_surface("Write a prompt", "2m 3s", '•', "press esc"),
        "• Working (2s • esc to interrupt)\n› Write a prompt".to_owned(),
    ] {
        assert_eq!(
            classify_native_composer(AgentKind::Codex, &plain(&surface)),
            NativeComposerState::Unknown,
            "surface must fail closed: {surface:?}"
        );
    }
}
```

- [ ] **Step 2: Run the focused tests and observe RED**

Run:

```bash
cargo test --test composer codex_working_boundary -- --nocapture
```

Expected: the dim-placeholder and unsent-text tests fail with `Unknown`, proving
the live Working boundary is not recognized. The malformed-shape cases already
remain fail-closed.

- [ ] **Step 3: Implement the narrow Working boundary predicate**

Change the boundary code in `src/composer.rs` to:

```rust
fn is_codex_boundary(line: &str) -> bool {
    is_pure_separator(line, 8) || is_worked_boundary(line) || is_working_boundary(line)
}

fn is_working_boundary(line: &str) -> bool {
    const PREFIX: &str = "• Working (";
    const SUFFIX: &str = " • esc to interrupt)";
    line.strip_prefix(PREFIX)
        .and_then(|value| value.strip_suffix(SUFFIX))
        .is_some_and(valid_elapsed_label)
}
```

Do not change `classify_content`, placeholder style rules, attachment counting,
footer recognition, or submission preflight.

- [ ] **Step 4: Run focused and adjacent composer tests and observe GREEN**

Run:

```bash
cargo test --test composer
cargo test --test transport_status
```

Expected: all composer and transport preflight tests pass; Working plus real
text remains `Occupied`, and malformed layouts remain `Unknown`.

- [ ] **Step 5: Request code review and verify the batch**

Invoke `superpowers:requesting-code-review`, address only actionable findings,
then run:

```bash
cargo fmt --check
cargo clippy --test composer -- -D warnings
```

Expected: formatting and Clippy pass with no warnings.

- [ ] **Step 6: Commit the composer fix**

```bash
git add src/composer.rs tests/composer.rs
git commit -m "fix working composer classification"
```

## Task 2: Restore Herdr-native drag selection and auto-copy

**Files:**
- Modify: `src/ui/terminal.rs`
- Modify: `src/ui/mod.rs:18-23, 214-232`
- Test: inline tests in `src/ui/terminal.rs`
- Test: inline tests in `src/ui/mod.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before changing terminal behavior.
- Invoke `superpowers:requesting-code-review` after the selection batch is green.
- Invoke `superpowers:verification-before-completion` before marking this task done.

- [ ] **Step 1: Make the PTY lifecycle test fail on mouse-capture sequences**

In `src/ui/terminal.rs`, strengthen the existing PTY test after assembling the
transcript:

```rust
assert!(transcript.contains("\u{1b}[?2004h"), "{transcript}");
assert!(transcript.contains("\u{1b}[?1049h"), "{transcript}");
assert!(!transcript.contains("\u{1b}[?1000h"), "{transcript}");
assert!(!transcript.contains("\u{1b}[?1002h"), "{transcript}");
assert!(!transcript.contains("\u{1b}[?1003h"), "{transcript}");
assert!(!transcript.contains("\u{1b}[?1006h"), "{transcript}");
```

Change the restoration-sequence test to require cursor, bracketed-paste, and
alternate-screen restoration while rejecting a mouse-disable sequence:

```rust
assert!(output.contains("\u{1b}[?25h"));
assert!(!output.contains("\u{1b}[?1000l"));
assert!(output.contains("\u{1b}[?2004l"));
assert!(output.contains("\u{1b}[?1049l"));
```

- [ ] **Step 2: Run the terminal tests and observe RED**

Run:

```bash
cargo test ui::terminal::tests -- --nocapture
```

Expected: current `EnableMouseCapture` and `DisableMouseCapture` sequences make
the new negative assertions fail.

- [ ] **Step 3: Remove mouse capture from the terminal lifecycle**

In `src/ui/terminal.rs`:

- import only `DisableBracketedPaste` and `EnableBracketedPaste` from
  `crossterm::event`;
- remove `EnableMouseCapture` from `TerminalGuard::enter`;
- remove `DisableMouseCapture` from `write_restore_sequences`;
- keep raw mode, alternate screen, bracketed paste, cursor hide/show, panic
  restoration, and normal drop restoration unchanged.

The resulting `execute!` payloads must be:

```rust
execute!(stdout, EnterAlternateScreen, EnableBracketedPaste, Hide)
```

and:

```rust
execute!(writer, Show, DisableBracketedPaste, LeaveAlternateScreen)
```

- [ ] **Step 4: Remove nested UI mouse-event consumption**

In `src/ui/mod.rs`, remove `MouseEventKind` from the Crossterm import and delete
the complete `Event::Mouse(mouse) => ...` match arm. Preserve the wildcard arm,
the `PageUp`/`PageDown` key paths, blocked keyboard routing, and all editor
behavior.

- [ ] **Step 5: Run terminal, keyboard-navigation, and hyperlink tests**

Run:

```bash
cargo test ui::terminal::tests -- --nocapture
cargo test ui::tests::page_navigation_remains_available_while_composer_is_guarded
cargo test --test ui_render markdown_hyperlink
```

Expected: no mouse-capture enable/disable sequence is emitted; keyboard history
scrolling and OSC 8 hyperlink rendering remain green.

- [ ] **Step 6: Request code review and verify the batch**

Invoke `superpowers:requesting-code-review`, address actionable findings, then
run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

- [ ] **Step 7: Commit the native-selection fix**

```bash
git add src/ui/terminal.rs src/ui/mod.rs
git commit -m "restore native terminal selection"
```

## Task 3: Add the safe existing-session registration helper

**Files:**
- Create: `scripts/register-existing-sessions.sh`
- Create: `tests/register-existing-sessions.sh`
- Modify: `.github/workflows/ci.yml`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before creating the helper.
- Invoke `superpowers:requesting-code-review` after helper tests are green.
- Invoke `superpowers:verification-before-completion` before marking this task done.

- [ ] **Step 1: Write a failing black-box helper test harness**

Create `tests/register-existing-sessions.sh` as an executable Bash test. It must:

```bash
#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/simple-prompts-session-test.XXXXXX")"
trap 'rm -rf "$fixture_root"' EXIT

fake_bin="$fixture_root/bin"
mkdir -p "$fake_bin" "$fixture_root/surfaces" "$fixture_root/codex/sessions/2026/08/14"

cat >"$fake_bin/herdr" <<'FAKE_HERDR'
#!/usr/bin/env bash
set -euo pipefail
case "$1 $2" in
  "agent list")
    cat "$FAKE_AGENT_LIST"
    ;;
  "agent read")
    cat "$FAKE_SURFACE_ROOT/$3.txt"
    ;;
  "pane report-agent-session")
    printf '%s\n' "$*" >>"$FAKE_REPORT_LOG"
    ;;
  *)
    printf 'unexpected fake herdr command\n' >&2
    exit 64
    ;;
esac
FAKE_HERDR
chmod +x "$fake_bin/herdr"

run_helper() {
  : >"$fixture_root/report.log"
  PATH="$fake_bin:$PATH" \
    FAKE_AGENT_LIST="$fixture_root/agents.json" \
    FAKE_SURFACE_ROOT="$fixture_root/surfaces" \
    FAKE_REPORT_LOG="$fixture_root/report.log" \
    CODEX_SESSIONS_ROOT="$fixture_root/codex/sessions" \
    bash "$repo_root/scripts/register-existing-sessions.sh"
}
```

Use fixed synthetic IDs such as
`11111111-1111-4111-8111-111111111111` and
`22222222-2222-4222-8222-222222222222`. Add isolated cases that rewrite
`agents.json` and pane surfaces before each `run_helper` call:

1. one unregistered Codex pane with one recognized footer and exactly one
   matching transcript filename reports once;
2. an already ID-registered pane produces no report and exits zero;
3. a UUID in conversation text with no recognized footer produces no report and
   exits non-zero;
4. two recognized footer candidates produce no report and exit non-zero;
5. two transcript filename matches produce no report and exit non-zero;
6. an unregistered Claude pane produces no report, exits non-zero, and mentions
   restart/resume;
7. captured stdout/stderr from every failure case contains neither synthetic ID.

Assertions must inspect `report.log` for the private command argument while
checking that user-visible output never includes it:

```bash
rg -q -- '--agent codex' "$fixture_root/report.log"
rg -q -- '--session-start-source resume' "$fixture_root/report.log"
rg -q -- '--agent-session-id 11111111-1111-4111-8111-111111111111' \
  "$fixture_root/report.log"
if rg -q '11111111-1111-4111-8111-111111111111' "$fixture_root/output.log"; then
  printf 'helper leaked a session id\n' >&2
  exit 1
fi
```

- [ ] **Step 2: Run the helper harness and observe RED**

Run:

```bash
bash tests/register-existing-sessions.sh
```

Expected: failure because `scripts/register-existing-sessions.sh` does not yet
exist.

- [ ] **Step 3: Implement the fail-closed helper**

Create executable `scripts/register-existing-sessions.sh` with this structure:

```bash
#!/usr/bin/env bash
set -u

herdr_bin="${HERDR_BIN:-herdr}"
jq_bin="${JQ_BIN:-jq}"
rg_bin="${RG_BIN:-rg}"
sessions_root="${CODEX_SESSIONS_ROOT:-${CODEX_HOME:-$HOME/.codex}/sessions}"
uuid_pattern='[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}'
footer_pattern="^[[:space:]]*gpt-[A-Za-z0-9._ -]+[[:space:]]*·[[:space:]]*(~|/)[^·]*[[:space:]]*·.*[[:space:]]·[[:space:]]*${uuid_pattern}([[:space:]]*·[^·]+)*[[:space:]]*$"

for command in "$herdr_bin" "$jq_bin" "$rg_bin"; do
  if ! command -v "$command" >/dev/null 2>&1; then
    printf 'Missing required command: %s\n' "$command" >&2
    exit 2
  fi
done

if ! agents_json="$($herdr_bin agent list)"; then
  printf 'Unable to read Herdr agents.\n' >&2
  exit 1
fi

failed=0
seen=0
while IFS=$'\t' read -r pane agent; do
  [ -n "$pane" ] || continue
  seen=1
  if [ "$agent" = "claude" ]; then
    printf 'Skipped %s: restart or resume Claude after installing the Herdr integration.\n' "$pane" >&2
    failed=1
    continue
  fi

  if ! surface="$($herdr_bin agent read "$pane" --source visible --lines 12 --format text)"; then
    printf 'Skipped %s: unable to read the native footer.\n' "$pane" >&2
    failed=1
    continue
  fi

  footer_line="$(printf '%s\n' "$surface" | sed '/^[[:space:]]*$/d' | tail -n 1)"
  candidates="$(
    printf '%s\n' "$footer_line" |
      "$rg_bin" "$footer_pattern" |
      "$rg_bin" -o "$uuid_pattern" |
      tr '[:upper:]' '[:lower:]' |
      sort -u
  )"
  candidate_count="$(printf '%s\n' "$candidates" | sed '/^$/d' | wc -l | tr -d ' ')"
  if [ "$candidate_count" -ne 1 ]; then
    printf 'Skipped %s: expected one native session candidate, found %s.\n' \
      "$pane" "$candidate_count" >&2
    failed=1
    continue
  fi
  candidate="$(printf '%s\n' "$candidates" | sed -n '1p')"

  matches="$(find "$sessions_root" -type f \
    \( -name "$candidate.jsonl" -o -name "*-$candidate.jsonl" \) -print 2>/dev/null)"
  match_count="$(printf '%s\n' "$matches" | sed '/^$/d' | wc -l | tr -d ' ')"
  if [ "$match_count" -ne 1 ]; then
    printf 'Skipped %s: expected one matching Codex transcript, found %s.\n' \
      "$pane" "$match_count" >&2
    failed=1
    continue
  fi

  if "$herdr_bin" pane report-agent-session "$pane" \
    --source herdr:simple-prompts-existing-sessions \
    --agent codex \
    --agent-session-id "$candidate" \
    --session-start-source resume >/dev/null; then
    printf 'Registered %s.\n' "$pane"
  else
    printf 'Skipped %s: Herdr rejected the session report.\n' "$pane" >&2
    failed=1
  fi
done < <(
  printf '%s' "$agents_json" |
    "$jq_bin" -r '
      .result.agents[]
      | select(.agent == "codex" or .agent == "claude")
      | select((.agent_session.kind? // "") != "id")
      | [.pane_id, .agent]
      | @tsv
    '
)

if [ "$seen" -eq 0 ]; then
  printf 'No unregistered Codex or Claude panes found.\n'
fi
exit "$failed"
```

While implementing, ensure a failed `rg` no-match does not terminate the script
and no error path interpolates `candidate` or a transcript filename. Keep
transcript validation filename-only; never open the JSONL file.

- [ ] **Step 4: Run syntax and black-box tests and observe GREEN**

Run:

```bash
bash -n scripts/register-existing-sessions.sh
bash -n tests/register-existing-sessions.sh
bash tests/register-existing-sessions.sh
```

Expected: every helper case passes, no session ID reaches captured output, and
the success case makes exactly one fake report call.

- [ ] **Step 5: Add the helper test to CI**

Append this step after the Rust tests in `.github/workflows/ci.yml`:

```yaml
      - run: bash tests/register-existing-sessions.sh
```

- [ ] **Step 6: Request code review and verify the helper batch**

Invoke `superpowers:requesting-code-review`, address actionable findings, then
run:

```bash
bash tests/register-existing-sessions.sh
git diff --check
```

- [ ] **Step 7: Commit the helper**

```bash
git add scripts/register-existing-sessions.sh tests/register-existing-sessions.sh .github/workflows/ci.yml
git commit -m "add existing session recovery helper"
```

## Task 4: Publish the English README and one privacy-safe screenshot

**Files:**
- Create: `assets/simple-prompts.png`
- Modify: `README.md`

**Required skill checkpoints:**
- This task is documentation and asset capture only, so the tester-oriented
  checkpoint does not apply.
- Invoke `superpowers:requesting-code-review` after the README/asset batch.
- Invoke `superpowers:verification-before-completion` before marking this task done.

- [ ] **Step 1: Capture the actual English Simple Prompts pane**

Build/link the task worktree, open a temporary Codex demo session, and use only
invented English content such as:

```text
Summarize why a focused prompt-and-answer view helps during long agent sessions.
```

Capture only the Simple Prompts pane—no Herdr sidebar, usernames, local paths,
session identifiers, or unrelated tabs—and save it as:

```text
assets/simple-prompts.png
```

On macOS, the interactive capture command is:

```bash
mkdir -p assets
screencapture -i assets/simple-prompts.png
```

Inspect the saved image before committing it. If any private data is present,
discard it and recapture; do not blur or cover leaked data after the fact.

- [ ] **Step 2: Rewrite the README opening and quick start**

Replace the current opening through “Bind the toggle” with this exact public
structure:

```markdown
# Herdr Simple Prompts

[![CI](https://github.com/AlexSamarsky/herdr_simple_prompts/actions/workflows/ci.yml/badge.svg)](https://github.com/AlexSamarsky/herdr_simple_prompts/actions/workflows/ci.yml)
[![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)
![Herdr 0.7.5+](https://img.shields.io/badge/herdr-0.7.5%2B-6f42c1.svg)
![macOS and Linux](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey.svg)

Simple Prompts is a focused full-pane view for Herdr. It keeps real user
prompts, final Codex or Claude answers, native Working/status, and a capable
composer visible while hiding reasoning, tool traffic, and other agent noise.

![Simple Prompts showing an English Codex conversation](assets/simple-prompts.png)

## Why Simple Prompts?

- Read the conversation you care about: prompts and final answers.
- Keep native Working, interruption, questions, approvals, images, and large
  pastes available.
- Reopen pane-scoped history without copying reasoning or tool logs.
- Inspect and build the complete source locally—no prebuilt plugin binary.

## Quick start

### Requirements

- Herdr 0.7.5 or newer
- Rust 1.85 or newer with Cargo
- `jq` and `rg` for the optional existing-session helper
- Codex CLI or Claude Code

Install the source-only plugin and the native integration you use:

```bash
herdr plugin install AlexSamarsky/herdr_simple_prompts
herdr integration install codex
# or
herdr integration install claude
```

Add the toggle to `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+m"
type = "plugin_action"
command = "herdr.simple-prompts.toggle"
description = "Toggle Simple Prompts"
```

Reload Herdr configuration:

```bash
herdr server reload-config
```

Focus a registered Codex or Claude pane and press the Herdr prefix (normally
`ctrl+b`) followed by `m`. Press the same hotkey to return to the unchanged
native pane.
```

- [ ] **Step 3: Add the existing-session recovery section**

Place this immediately after Quick start:

```markdown
## Existing sessions

Sessions started after the Herdr Codex/Claude integration is installed register
automatically. To recover already-running Codex panes that predate the
integration, inspect and run the repository helper:

```bash
git clone https://github.com/AlexSamarsky/herdr_simple_prompts.git
cd herdr_simple_prompts
sed -n '1,240p' scripts/register-existing-sessions.sh
bash scripts/register-existing-sessions.sh
```

The helper reads no conversation transcript content and prints no recovered
session IDs. It registers only one unambiguous Codex footer whose ID has exactly
one matching local transcript filename. Ambiguous panes are skipped. Existing
Claude panes whose identity is unavailable must be restarted or resumed after
installing the Claude integration.

This is a one-time metadata recovery tool, not a general `prefix+m` repair
command. Already registered panes are left unchanged.
```

- [ ] **Step 4: Correct controls and retain technical documentation**

Keep the existing source-only trust, conversation rendering, composer safety,
blocked interaction, images, privacy/state, development, troubleshooting,
limitations, marketplace, and license sections. Remove repeated installation
material and replace all `<github-owner>` placeholders with `AlexSamarsky`.

In both the conversation bullets and key table:

- document only `PageUp` / `PageDown` for in-app history scrolling;
- remove `Mouse wheel | Scroll conversation history`;
- add one sentence: “Drag across text and release the mouse to use Herdr's
  native selection and automatic copy behavior.”

Update the manual smoke test to require drag/release auto-copy and stop claiming
that the mouse wheel scrolls Simple Prompts history.

- [ ] **Step 5: Validate the public documentation and image**

Run:

```bash
test -s assets/simple-prompts.png
file assets/simple-prompts.png
rg -n '<github-owner>|herdr_simple_promts|Mouse wheel' README.md && exit 1 || true
rg -n 'AlexSamarsky/herdr_simple_prompts|prefix\+m|register-existing-sessions.sh|PageUp' README.md
git diff --check
```

Expected: the PNG is non-empty, no owner typo/placeholder or mouse-wheel claim
remains, and the exact repository/install/helper/control strings are present.

- [ ] **Step 6: Request documentation review and commit**

Invoke `superpowers:requesting-code-review`, address factual or privacy
findings, then commit:

```bash
git add README.md assets/simple-prompts.png
git commit -m "publish simple prompts readme"
```

## Task 5: Full verification, live smoke, merge, and plugin refresh

**Files:**
- Verify all modified files; no new production behavior is introduced in this task.

**Required skill checkpoints:**
- Invoke `superpowers:requesting-code-review` for the complete branch diff.
- Invoke `superpowers:verification-before-completion` before any success claim.
- Invoke `superpowers:finishing-a-development-branch` before merging and cleanup.

- [ ] **Step 1: Run the complete automated gate from a clean branch snapshot**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
bash -n scripts/register-existing-sessions.sh
bash -n tests/register-existing-sessions.sh
bash tests/register-existing-sessions.sh
git diff main...HEAD --check
git status --short
```

Expected: 0 failures/warnings, a successful locked release build, green helper
tests, no whitespace errors, and no uncommitted files.

- [ ] **Step 2: Request final code review**

Invoke `superpowers:requesting-code-review` against `main...HEAD`. Address every
confirmed correctness, safety, privacy, portability, and documentation finding;
rerun the complete gate after any change.

- [ ] **Step 3: Exercise live behavior without targeting another session**

Rebuild/link only this worktree and reload plugin metadata:

```bash
cargo build --locked --release
herdr plugin link /Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts/.worktrees/public-readme-working-composer
herdr server reload-config
```

From the intended source pane, close/reopen Simple Prompts with `prefix+m` and
verify:

1. during native Working with only the dim placeholder, the plugin editor stays
   visible;
2. real unsent native text still shows the occupied warning and is preserved;
3. mouse drag/release selects and automatically copies exactly as in Herdr;
4. `PageUp`/`PageDown`, OSC 8 links, input, large paste, image paste, and
   `prefix+m` still work.

Do not use an unscoped `herdr plugin action invoke`; it can act on whichever
Herdr pane is globally active.

- [ ] **Step 4: Finish and merge the development branch**

Invoke `superpowers:finishing-a-development-branch`. Because the user requested
the completed plugin in the main checkout, merge
`feature/public-readme-working-composer` into `main` non-interactively after all
gates are green. Do not rewrite unrelated history.

- [ ] **Step 5: Relink the merged main checkout and reverify installation**

```bash
cd /Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts
cargo build --locked --release
herdr plugin link /Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts
herdr server reload-config
herdr plugin list --plugin herdr.simple-prompts
git status --short --branch
```

Expected: Herdr reports `herdr.simple-prompts` from the main source checkout,
the repository is clean on `main`, and the user can close/reopen the intended
session with `prefix+m` to load the merged binary.

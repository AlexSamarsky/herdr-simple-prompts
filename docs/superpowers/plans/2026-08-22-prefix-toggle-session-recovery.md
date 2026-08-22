# Prefix Toggle Session Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `prefix+m` recover missing ID-based metadata for only the current Codex pane, verify the registration, and then open Simple Prompts without pane-specific operator commands.

**Architecture:** Keep the existing fail-closed recovery algorithm in `scripts/register-existing-sessions.sh`, adding an optional `--pane` selector that preserves its all-pane operator mode. Route the manifest toggle action through a small wrapper that runs selected-pane recovery and executes the existing Rust toggle only after recovery succeeds or proves unnecessary.

**Tech Stack:** Bash 3.2-compatible shell, Herdr 0.7.5 CLI, `jq`, `rg`, Rust manifest contract tests, Cargo verification gates.

---

## File Map

- Modify `scripts/register-existing-sessions.sh`: parse `--pane`, filter before recovery work, and distinguish selected-pane no-op output from all-pane no-op output.
- Modify `tests/register-existing-sessions.sh`: drive the helper with arguments and prove selection, isolation, existing-registration, overlay passthrough, argument validation, and privacy.
- Create `scripts/toggle-with-session-recovery.sh`: require Herdr action context, run current-pane recovery, then replace itself with the Rust toggle.
- Create `tests/toggle-with-session-recovery.sh`: prove ordering and fail-closed boundaries with executable fakes.
- Modify `herdr-plugin.toml`: point only the toggle action at the wrapper; keep the UI pane command unchanged.
- Modify `tests/manifest_contract.rs`: bind the manifest contract to the wrapper action and direct Rust UI command.
- Modify `README.md`: make `jq` and `rg` installation prerequisites and describe automatic current-pane recovery.
- Modify `docs/behavior.md`: define the hotkey recovery and privacy contract.
- Modify `docs/troubleshooting.md`: replace manual-first guidance with automatic recovery plus bounded fallback diagnostics.

### Task 1: Add fail-closed current-pane selection to the recovery helper

**Files:**
- Modify: `tests/register-existing-sessions.sh`
- Modify: `scripts/register-existing-sessions.sh`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before editing the helper.
- Invoke `superpowers:requesting-code-review` after the helper batch is green.
- Invoke `superpowers:verification-before-completion` before marking this task done.

- [ ] **Step 1: Extend the shell harness so each case can pass helper arguments**

Change `run_helper` to forward its arguments after the script path:

```bash
run_helper() {
  set +e
  PATH="$fake_bin:$PATH" \
    HERDR_BIN="$fake_bin/herdr" \
    FAKE_AGENT_LIST="$agents_file" \
    FAKE_SURFACE_ROOT="$surface_root" \
    FAKE_REPORT_LOG="$report_log" \
    FAKE_REPORT_FAIL="${FAKE_REPORT_FAIL:-0}" \
    FAKE_REPORT_DROP="${FAKE_REPORT_DROP:-0}" \
    CODEX_SESSIONS_ROOT="$sessions_root" \
    bash "$repo_root/scripts/register-existing-sessions.sh" "$@" \
      >"$output_log" 2>&1
  RUN_STATUS=$?
  set -e
}
```

- [ ] **Step 2: Add failing selected-pane isolation tests**

Append cases that create two unregistered Codex agents, select only the first,
and require exactly one report for that pane:

```bash
reset_case
write_agents "{\"result\":{\"agents\":[
  {\"pane_id\":\"w1:p1\",\"agent\":\"codex\",\"agent_session\":null},
  {\"pane_id\":\"w1:p2\",\"agent\":\"codex\",\"agent_session\":null}
]}}"
write_surface "w1:p1" "gpt-5.6-sol xhigh · /repo · weekly 47% left · $primary_id"
write_surface "w1:p2" "gpt-5.6-sol xhigh · /other · weekly 47% left · $secondary_id"
add_transcript "one" "$primary_id.jsonl"
add_transcript "two" "$secondary_id.jsonl"
run_helper --pane "w1:p1"
assert_status 0
[ "$(wc -l <"$report_log" | tr -d ' ')" -eq 1 ] || fail "selected recovery reported more than once"
rg -q '^pane report-agent-session w1:p1 ' "$report_log" || fail "selected pane was not reported"
! rg -q 'w1:p2' "$report_log" || fail "unrelated pane was reported"
assert_no_session_ids_in_output
```

Add a case where the selected pane is already registered while another pane is
not; require status 0, no report, and `No recovery needed for w1:p1.`. Add a
case for an overlay-like pane absent from `agent list` with the same successful
no-op contract. Add invalid invocation cases for `--pane` without a value and
for an unknown option; both must exit 2, print `Usage:`, and create no report.

- [ ] **Step 3: Run the helper tests and confirm RED**

Run:

```bash
bash tests/register-existing-sessions.sh
```

Expected: FAIL because `register-existing-sessions.sh` ignores or rejects no
selector and processes the unrelated unregistered pane.

- [ ] **Step 4: Parse one optional selector before command discovery**

Add after `set -u`:

```bash
target_pane=""
if [ "$#" -gt 0 ]; then
  if [ "$#" -ne 2 ] || [ "$1" != "--pane" ] || [ -z "${2:-}" ]; then
    printf 'Usage: %s [--pane PANE_ID]\n' "$0" >&2
    exit 2
  fi
  target_pane="$2"
fi
```

Keep `target_pane` opaque: do not validate or rewrite the Herdr pane ID.

- [ ] **Step 5: Filter the agent list before the recovery loop**

Pass the selector to `jq` and select it before checking registration:

```bash
if ! unregistered="$(
  printf '%s' "$agents_json" |
    "$jq_bin" -r --arg target_pane "$target_pane" '
      .result.agents[]
      | select($target_pane == "" or .pane_id == $target_pane)
      | select(.agent == "codex" or .agent == "claude")
      | select((.agent_session.kind? // "") != "id")
      | [.pane_id, .agent]
      | @tsv
    ' 2>/dev/null
)"; then
```

Replace the final no-op message with:

```bash
if [ "$seen" -eq 0 ]; then
  if [ -n "$target_pane" ]; then
    printf 'No recovery needed for %s.\n' "$target_pane"
  else
    printf 'No unregistered Codex or Claude panes found.\n'
  fi
fi
```

- [ ] **Step 6: Run focused syntax and contract tests and confirm GREEN**

Run:

```bash
bash -n scripts/register-existing-sessions.sh
bash -n tests/register-existing-sessions.sh
bash tests/register-existing-sessions.sh
```

Expected: all commands exit 0 and the test prints
`existing-session helper tests passed`.

- [ ] **Step 7: Review the helper batch**

Invoke `superpowers:requesting-code-review` and inspect:

```bash
git diff --check
git diff -- scripts/register-existing-sessions.sh tests/register-existing-sessions.sh
```

The review must confirm that default all-pane behavior is unchanged, selected
mode cannot reach another pane, and no diagnostic prints a recovered ID or
transcript path.

- [ ] **Step 8: Commit the helper batch**

```bash
git add scripts/register-existing-sessions.sh tests/register-existing-sessions.sh
git commit -m "add scoped Codex session recovery"
```

### Task 2: Route the toggle through a verified recovery wrapper

**Files:**
- Create: `scripts/toggle-with-session-recovery.sh`
- Create: `tests/toggle-with-session-recovery.sh`
- Modify: `herdr-plugin.toml`
- Modify: `tests/manifest_contract.rs`

**Required skill checkpoints:**
- Continue `superpowers:test-driven-development` for the wrapper and manifest contract.
- Invoke `superpowers:requesting-code-review` after the wrapper batch is green.
- Invoke `superpowers:verification-before-completion` before marking this task done.

- [ ] **Step 1: Write the failing wrapper contract test**

Create an executable test that uses a temporary directory and two fakes. The
recovery fake records `recover:$*` and exits with `RECOVERY_STATUS`; the toggle
fake records `toggle:$*` and exits with `TOGGLE_STATUS`:

```bash
#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/simple-prompts-toggle-test.XXXXXX")"
trap 'rm -rf "$fixture_root"' EXIT
call_log="$fixture_root/calls.log"
output_log="$fixture_root/output.log"

cat >"$fixture_root/recover.sh" <<'FAKE_RECOVERY'
#!/usr/bin/env bash
printf 'recover:%s\n' "$*" >>"$CALL_LOG"
exit "${RECOVERY_STATUS:-0}"
FAKE_RECOVERY

cat >"$fixture_root/toggle" <<'FAKE_TOGGLE'
#!/usr/bin/env bash
printf 'toggle:%s\n' "$*" >>"$CALL_LOG"
exit "${TOGGLE_STATUS:-0}"
FAKE_TOGGLE
chmod +x "$fixture_root/recover.sh" "$fixture_root/toggle"
```

Add a `run_wrapper` helper that clears the log and runs the real wrapper with
`SIMPLE_PROMPTS_RECOVERY_SCRIPT`, `SIMPLE_PROMPTS_TOGGLE_BIN`, `CALL_LOG`, and
an optional `HERDR_PANE_ID`. Cover:

```text
success:       recover:--pane w1:p1, then toggle:toggle, status 0
recovery fail: recover only, status 7
toggle fail:   both calls in order, status 9
missing pane:  no calls, status 2, diagnostic names HERDR_PANE_ID
```

- [ ] **Step 2: Change the manifest expectation before its implementation**

Update only the action assertion in `tests/manifest_contract.rs`:

```rust
assert_eq!(
    strings(&parsed["actions"][0]["command"]),
    vec!["./scripts/toggle-with-session-recovery.sh"]
);
```

Keep the pane command assertion unchanged so the UI entrypoint still executes
`./target/release/herdr-simple-prompts ui` directly.

- [ ] **Step 3: Run both focused tests and confirm RED**

Run:

```bash
bash tests/toggle-with-session-recovery.sh
cargo test --test manifest_contract manifest_is_source_only_and_registers_targeted_zoomed_view -- --exact
```

Expected: the shell test fails because the wrapper does not exist, and the Rust
test fails because the manifest still names the Rust toggle directly.

- [ ] **Step 4: Implement the minimal wrapper**

Create `scripts/toggle-with-session-recovery.sh`:

```bash
#!/usr/bin/env bash
set -eu

pane="${HERDR_PANE_ID:-}"
if [ -z "$pane" ]; then
  printf 'herdr-simple-prompts: toggle: HERDR_PANE_ID is not set\n' >&2
  exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
recovery_script="${SIMPLE_PROMPTS_RECOVERY_SCRIPT:-$script_dir/register-existing-sessions.sh}"
toggle_bin="${SIMPLE_PROMPTS_TOGGLE_BIN:-$script_dir/../target/release/herdr-simple-prompts}"

bash "$recovery_script" --pane "$pane"
exec "$toggle_bin" toggle
```

Mark it executable. The test-only override variables must not appear in public
documentation and must not alter the production defaults.

- [ ] **Step 5: Route the manifest action through the wrapper**

Change only:

```toml
command = ["./scripts/toggle-with-session-recovery.sh"]
```

Do not change the build command, action ID, contexts, pane ID, placement, or UI
command.

- [ ] **Step 6: Run focused tests and confirm GREEN**

Run:

```bash
bash -n scripts/toggle-with-session-recovery.sh
bash -n tests/toggle-with-session-recovery.sh
bash tests/toggle-with-session-recovery.sh
cargo test --test manifest_contract manifest_is_source_only_and_registers_targeted_zoomed_view -- --exact
```

Expected: all commands exit 0; the shell test prints
`toggle recovery wrapper tests passed`; the Rust test reports one pass.

- [ ] **Step 7: Review the wrapper batch**

Invoke `superpowers:requesting-code-review` and inspect:

```bash
git diff --check
git diff -- scripts/toggle-with-session-recovery.sh tests/toggle-with-session-recovery.sh herdr-plugin.toml tests/manifest_contract.rs
```

The review must confirm recovery finishes before toggle execution, a recovery
failure cannot open an overlay, exit codes are preserved, every path is quoted,
and the wrapper carries no pane or session constant.

- [ ] **Step 8: Commit the wrapper batch**

```bash
git add scripts/toggle-with-session-recovery.sh tests/toggle-with-session-recovery.sh herdr-plugin.toml tests/manifest_contract.rs
git commit -m "recover Codex sessions before toggle"
```

### Task 3: Align public and behavior documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/behavior.md`
- Modify: `docs/troubleshooting.md`

**Required skill checkpoints:**
- Tester-oriented TDD does not apply because this task changes prose only after the executable contract is green.
- Invoke `superpowers:requesting-code-review` for accuracy against the implemented scripts.
- Invoke `superpowers:verification-before-completion` before marking the documentation aligned.

- [ ] **Step 1: Update installation prerequisites and everyday use**

Change the README prerequisite sentence to require `jq` and `rg` for automatic
Codex recovery. Add one concise sentence after the hotkey instructions:

```markdown
If a focused Codex pane is missing native session metadata, `prefix+m`
recovers and verifies only that pane before opening Simple Prompts. Recovery is
fail-closed and never reads transcript contents.
```

- [ ] **Step 2: Add the behavior contract under Pane targeting**

Document that the action wrapper scopes recovery to `HERDR_PANE_ID`, skips
recovery for an already registered or non-agent overlay pane, validates exactly
one footer ID and transcript filename, verifies Herdr retention, and never runs
the Rust toggle after a failed recovery.

- [ ] **Step 3: Rewrite troubleshooting around automatic recovery**

Keep the standalone all-pane helper as an inspectable operator fallback, but
state that ordinary `prefix+m` performs current-pane recovery automatically.
List the remaining actionable failure classes: missing `jq`/`rg`, unreadable
agent surface, ambiguous footer, ambiguous transcript match, report rejection,
and unretained metadata. Do not instruct operators to hardcode a pane or
session ID.

- [ ] **Step 4: Check documentation consistency**

Run:

```bash
rg -n 'prefix\+m|register-existing-sessions|jq|rg|agent_session' README.md docs/behavior.md docs/troubleshooting.md
git diff --check
```

Expected: `prefix+m` consistently describes automatic current-pane recovery;
the manual helper is labeled a fallback; no text claims that every pane is
scanned before a toggle.

- [ ] **Step 5: Review and commit documentation**

Invoke `superpowers:requesting-code-review`, compare the prose to the implemented
scripts, then commit:

```bash
git add README.md docs/behavior.md docs/troubleshooting.md
git commit -m "document automatic toggle recovery"
```

### Task 4: Run full verification and the live hotkey acceptance check

**Files:**
- Verify all files changed since `9b08cd9` on `fix/codex-session-recovery`.
- Do not edit production or test files during this task unless a verification finding returns execution to the relevant TDD task.

**Required skill checkpoints:**
- Invoke `superpowers:verification-before-completion` before running or reporting these gates.
- Invoke `superpowers:requesting-code-review` once for the complete branch diff if earlier batch reviews found no unresolved issues.

- [ ] **Step 1: Run shell contracts and syntax checks**

```bash
bash -n scripts/register-existing-sessions.sh
bash -n scripts/toggle-with-session-recovery.sh
bash -n tests/register-existing-sessions.sh
bash -n tests/toggle-with-session-recovery.sh
bash tests/register-existing-sessions.sh
bash tests/toggle-with-session-recovery.sh
```

Expected: all six commands exit 0 and both test scripts print their success
messages.

- [ ] **Step 2: Run the repository Rust gates**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
```

Expected: every command exits 0 with no Clippy warning or failed test.

- [ ] **Step 3: Inspect exact branch evidence**

```bash
git status --short
git diff --check 9b08cd9...HEAD
git diff --stat 9b08cd9...HEAD
git log --oneline --decorate 9b08cd9..HEAD
```

Expected: the worktree is clean, diff check is silent, and the log contains the
design plus the scoped helper, wrapper, and documentation commits.

- [ ] **Step 4: Rebuild and relink the local plugin**

```bash
cargo build --locked --release
herdr plugin link .
herdr server reload-config
```

Expected: Herdr reports the local plugin link and reload without error. This is
the only activation step; do not run `pane report-agent-session` manually.

- [ ] **Step 5: Perform the live user-facing acceptance check**

Use a live Codex pane that Herdr detects but that lacks ID-based
`agent_session`. Without running the standalone helper or any pane-specific
registration command, focus that pane and press `prefix+m` once.

Expected evidence:

```text
the selected pane changes from missing metadata to kind=id/source=herdr:codex
exactly one Simple Prompts overlay opens and receives focus
the plugin action exits successfully
no other unregistered pane receives a session report
```

If the environment lacks a naturally unregistered pane, stop and ask the user
to perform this one hotkey acceptance check; do not manufacture one by deleting
or corrupting native metadata.

- [ ] **Step 6: Final review and completion audit**

Review the complete diff against
`docs/superpowers/specs/2026-08-22-prefix-toggle-session-recovery-design.md`.
Record verification command results, live-smoke coverage, and any remaining
provider limitation. Do not claim the issue fixed unless both automated gates
and the live hotkey acceptance check pass.

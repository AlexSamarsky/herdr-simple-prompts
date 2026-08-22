# Prefix Toggle Session Recovery Design

## Context

Simple Prompts opens only after Herdr exposes an ID-based native agent session
for the focused Codex pane. The Codex integration normally reports that
metadata during `SessionStart`, but a live pane can still be detected as Codex
without carrying `agent_session`. In that state `prefix+m` reaches the plugin
action, `agent.get` succeeds, and the toggle exits with `native agent session is
unavailable` instead of opening Simple Prompts.

The repository already provides `scripts/register-existing-sessions.sh` for
fail-closed recovery. It validates a Codex footer, requires one transcript
filename match, reports the session through Herdr, and verifies that Herdr kept
the report. Requiring the operator to run that helper separately leaves the
ordinary toggle broken, while registering a hardcoded pane is not reusable.

## Decision

Keep `prefix+m` and make its action command a small wrapper script. The wrapper
uses only `HERDR_PANE_ID` supplied by Herdr and never embeds a pane or session
identifier.

Before executing the Rust toggle, the wrapper invokes the existing recovery
helper in a new current-pane mode. That mode inspects only the requested pane:

1. Read `herdr agent list` once.
2. If the requested pane already has an ID-based session, return success
   without reporting it again.
3. If the requested pane is an unregistered Codex agent, apply the existing
   strict footer and transcript-filename validation, report it with source
   `herdr:codex`, and verify the retained metadata.
4. If the requested pane is not an agent entry, return success without
   recovery. This preserves toggling from an existing Simple Prompts overlay;
   the Rust toggle remains responsible for resolving that overlay back to its
   source.
5. If Codex recovery is ambiguous or Herdr rejects or drops the report, return
   failure and do not launch the Rust toggle.
6. If recovery succeeds or was unnecessary, `exec` the existing
   `herdr-simple-prompts toggle` command.

The helper's default no-argument behavior remains the operator command that
scans all unregistered panes. Current-pane mode is an additive option used by
the wrapper, so existing documentation and recovery workflows remain valid.

## Alternatives

### Recover every pane before each toggle

Rejected. It touches unrelated sessions, makes one pane's hotkey depend on
every other pane, and repeats work that the focused action does not need.

### Recover inside the Rust toggle

This would avoid shell dependencies and could share the Herdr client directly,
but it duplicates the already tested recovery contract in another language and
expands the binary change. The approved scope is script-level error handling.

### Keep manual recovery only

Rejected. It preserves the exact recurring failure: `prefix+m` cannot repair
the current pane on its own.

## Error Handling

- The wrapper requires `HERDR_PANE_ID`; a missing value is a configuration
  error and stops before the toggle.
- The helper keeps pane-scoped diagnostics and never prints a recovered session
  identifier or transcript path.
- Missing commands, unreadable agent state, malformed footers, zero or multiple
  candidates, zero or multiple transcript matches, rejected reports, and
  unretained reports remain fail-closed.
- A recovery failure must produce a non-zero action result and must not create
  or focus a Simple Prompts overlay.
- An already registered pane takes the fast path and does not read its footer or
  scan transcripts.
- Overlay-to-source closing continues through the existing Rust state mapping;
  the wrapper must not treat a non-agent overlay as a recovery failure.

## Files

- `herdr-plugin.toml`: route the `toggle` action through the wrapper.
- `scripts/toggle-with-session-recovery.sh`: validate action context, run
  current-pane recovery, and `exec` the Rust toggle.
- `scripts/register-existing-sessions.sh`: accept a current-pane selector while
  preserving the default all-pane mode.
- `tests/register-existing-sessions.sh`: cover selector behavior and prove
  unrelated panes are untouched.
- `tests/toggle-with-session-recovery.sh`: cover wrapper ordering, exit status,
  overlay passthrough, and the no-toggle-on-recovery-failure boundary.
- `tests/manifest_contract.rs`: require the action to use the wrapper while the
  pane entrypoint continues to launch the Rust UI binary directly.
- `README.md`: move `jq` and `rg` from optional recovery-helper tools into the
  installation prerequisites because current-pane recovery now runs from the
  everyday toggle action.
- `docs/behavior.md` and `docs/troubleshooting.md`: document automatic
  current-pane recovery and the remaining fail-closed cases.

## Testing

Test-first coverage will prove:

- an unregistered selected Codex pane is reported once, verified, and followed
  by exactly one toggle execution;
- an already registered selected pane skips footer and transcript work and
  executes the toggle once;
- current-pane recovery never reports another unregistered pane;
- an overlay or other pane absent from `agent list` passes through to the Rust
  toggle without a recovery report;
- ambiguous candidates, ambiguous transcripts, report rejection, and dropped
  reports stop before the toggle and keep session identifiers out of output;
- missing `HERDR_PANE_ID` stops before both recovery and toggle;
- the helper's existing all-pane behavior remains unchanged;
- the manifest continues to expose `herdr.simple-prompts.toggle` on
  `prefix+m` through the user's existing key binding.

Run the shell contract tests, then the repository gates:

```bash
bash tests/register-existing-sessions.sh
bash tests/toggle-with-session-recovery.sh
bash -n scripts/register-existing-sessions.sh
bash -n scripts/toggle-with-session-recovery.sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
```

The live smoke test starts an unregistered Codex pane without a Simple Prompts
overlay, focuses it, and presses `prefix+m` once. The expected result is one
verified native-session registration followed by one focused Simple Prompts
overlay. No pane-specific recovery command is allowed before the hotkey test.

## Privacy and Scope

Recovery reads only the bottom visible native surface and transcript filenames;
it never reads transcript contents. It stores no new data and performs no
network access. Claude recovery, Herdr integration internals, transcript
parsers, UI rendering, and keybinding changes are outside this change.

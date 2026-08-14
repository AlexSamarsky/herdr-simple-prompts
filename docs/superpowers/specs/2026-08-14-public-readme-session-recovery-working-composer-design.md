# Public README, Existing-Session Recovery, and Working Composer Safety

## Context

Simple Prompts is ready for a public source-only GitHub repository, but its root
README currently leads with implementation detail instead of a concise product
introduction and installation path. It also lacks a maintained command for
registering Codex panes that were already running before the Herdr integration
was installed.

A live Codex layout exposed a related composer-classification defect. While a
turn is running, Codex places its current composer immediately after a native
`• Working (<elapsed> • esc to interrupt)` row. The existing classifier accepts
only a plain separator or a completed `─ Worked for <elapsed> ─…` row as the
composer boundary. It therefore reports `Unknown` even when the native composer
contains only a dim placeholder, and Simple Prompts replaces its editor with an
unnecessary safety warning.

## Goals

- Present Simple Prompts clearly to a first-time GitHub visitor in English.
- Provide the exact public install command for
  `AlexSamarsky/herdr_simple_prompts` and document `prefix+m`.
- Include one English, privacy-safe screenshot of the real terminal UI.
- Provide a repository-owned helper for safely registering already-running
  Codex panes whose native session metadata is absent from Herdr.
- Treat the exact native Codex `Working` row as a valid current-composer
  boundary without weakening protection against unsent native input.
- Keep distribution source-only and preserve the existing Herdr 0.7.5+, Rust
  1.85+, macOS/Linux, Codex/Claude, history, and privacy contracts.

## Non-goals

- Do not publish prebuilt binaries or add a release-download installation path.
- Do not change the Simple Prompts visual design for the screenshot.
- Do not infer a session identity from arbitrary UUIDs in conversation text.
- Do not guess a Claude session identity from directory timestamps or working
  directory alone.
- Do not weaken the fail-closed behavior for malformed or unfamiliar composer
  layouts.
- Do not modify Herdr or the native Codex/Claude applications.

## Public README design

The existing root `README.md` remains the single canonical project entrypoint.
It will be reorganized rather than replaced with a second case-variant filename.
The opening sequence will be:

1. project title and compact badges for CI, MIT, Rust 1.85+, Herdr 0.7.5+, and
   macOS/Linux;
2. a one-paragraph value proposition explaining that Simple Prompts shows user
   prompts and final answers while retaining native Working/status and input;
3. one screenshot at `assets/simple-prompts.png`;
4. a short “Why Simple Prompts?” section;
5. a quick-start path with prerequisites, install, integration setup, restart
   expectations, and `prefix+m`;
6. a dedicated “Existing sessions” section using the repository helper;
7. concise feature and behavior sections;
8. the existing architecture, safety, troubleshooting, development, and
   verification material, edited for accuracy and reduced repetition.

The canonical installation command is:

```bash
herdr plugin install AlexSamarsky/herdr_simple_prompts
```

The screenshot will be captured from a temporary English demo session. It will
show only the Simple Prompts pane, with invented prompt/answer content, no local
usernames, private paths, session identifiers, unrelated panes, or repository
data. The asset is documentation, not a separately maintained UI mockup.

## Existing-session registration helper

Create `scripts/register-existing-sessions.sh` as an inspectable Bash script.
It depends only on `herdr`, `jq`, `rg`, and standard Unix tools and performs no
network access.

The helper will:

1. read `herdr agent list` once;
2. select Codex and Claude panes whose `agent_session` is not already an
   ID-based registration;
3. inspect only the bottom native status/composer area for each unregistered
   Codex pane;
4. accept a UUID only when it appears on a recognized Codex footer line;
5. deduplicate candidates and require exactly one candidate;
6. verify that exactly one transcript filename under
   `${CODEX_HOME:-$HOME/.codex}/sessions` matches that candidate;
7. call `herdr pane report-agent-session` with source
   `herdr:simple-prompts-existing-sessions`, agent `codex`, and start source
   `resume`;
8. print pane-scoped success, skip, and error messages without printing the
   recovered session identifier.

Already registered panes remain unchanged. A pane with no candidate, multiple
candidates, an ambiguous transcript match, a failed read, or a failed report is
skipped with a reason and contributes to a non-zero exit status. Claude panes
without ID metadata are skipped with an explicit restart/resume instruction;
the helper will not invent their identity.

The README will explain that new sessions register automatically after the
Herdr Codex/Claude integrations are installed. The helper is a one-time recovery
tool for panes that predate integration installation, not a general hotkey or
stale-overlay repair command.

## Working composer classification

Extend the Codex boundary predicate with a narrowly defined native Working
shape:

```text
• Working (<valid elapsed> • esc to interrupt)
```

The elapsed portion uses the existing numeric `h`/`m`/`s` validation and may
contain one or more components such as `2s`, `3m 4s`, or `1h 2m 3s`. The leading
bullet, spaces, parentheses, separator bullet, and `esc to interrupt` suffix
must match exactly. A malformed label, different suffix, missing footer, or
truncated surface remains `Unknown`.

Recognizing the boundary does not determine whether submission is safe. The
existing content classifier still decides:

- a dim or bright-black native placeholder is `Clear`;
- exact plugin-owned attachment markers are `OwnedAttachments(n)`;
- any real user text is `Occupied`;
- incomplete styling or layout evidence remains fail-closed.

Submission retains its second, immediate preflight. No new race or path around
the native-draft protection is introduced.

## Testing

### Composer regression

- A live-shaped Working row followed by a dim placeholder classifies `Clear`.
- The same boundary followed by plain text classifies `Occupied`.
- Valid multi-component elapsed labels are accepted.
- Malformed elapsed text, wrong suffix, wrong separator, missing footer, and
  truncated surfaces classify `Unknown`.
- Existing completed-turn, Claude, attachment-count, and submission-preflight
  tests remain green.

### Helper

The shell helper will expose command paths through environment overrides used
only by its test harness. Tests will use temporary fake `herdr`, `jq`/input, and
transcript fixtures to prove:

- already registered panes are not reported again;
- one validated Codex footer/transcript pair is registered once;
- arbitrary UUIDs outside a recognized footer are ignored;
- zero, multiple, and transcript-ambiguous candidates are skipped;
- Claude recovery is skipped safely;
- failures do not reveal session identifiers and produce a non-zero exit.

### Documentation and asset

- README commands and repository URLs use
  `AlexSamarsky/herdr_simple_prompts` consistently.
- The screenshot link resolves to a committed image with English demo content.
- Manifest and CLI contract tests remain unchanged unless a documentation
  assertion already owns the affected text.

## Verification

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
bash -n scripts/register-existing-sessions.sh
```

Run the helper tests, verify the README links locally, rebuild/reload the
source-linked plugin, and smoke-test a live Codex turn. During Working, the
Simple Prompts editor must remain visible for an empty native composer; real
native input must still replace it with the occupied warning.

## Privacy and security

The helper never sends session identifiers to stdout/stderr and never reads
conversation transcript contents; it validates identity from transcript
filenames only. The screenshot contains synthetic data. The project continues
to ship source only, performs no new network requests at runtime, and stores no
new persistent user data.

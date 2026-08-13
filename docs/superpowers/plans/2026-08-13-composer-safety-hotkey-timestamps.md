# Composer Safety, Hotkey Recovery, and Prompt Timestamps Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. Use `superpowers:test-driven-development` for each behavior change, request code review after each task, and use `superpowers:verification-before-completion` before claiming success.

**Goal:** Prevent duplicate submissions when the native Codex/Claude composer already contains input, recover the Simple Prompts hotkey from stale overlay pane state, and show the local date/time of every user prompt without increasing prompt-band height.

**Architecture:** Add one pure ANSI-screen classifier that recognizes only proven-safe Codex and Claude composer states. The UI uses it for early warnings, while `AgentTransport` repeats the same check immediately before `agent.prompt`, so submission fails closed even if UI state is stale. Keep overlay state recovery in `toggle.rs`, and isolate local timestamp formatting behind a small `chrono`-backed helper.

**Tech Stack:** Rust 2024, Ratatui, Herdr SDK 0.7.5, Chrono 0.4.45 (`clock` + `std`, default features disabled), existing JSONL history journal and test helpers.

**Approved design:** `docs/superpowers/specs/2026-08-13-composer-safety-hotkey-timestamps-design.md`

## Scope and file map

- Create `src/composer.rs`: pure native-composer classifier and access policy.
- Modify `src/lib.rs`: expose the new internal modules.
- Modify `src/transport.rs`: authoritative pre-submit ANSI preflight.
- Modify `src/ui/runtime.rs`: observe one ANSI surface and carry composer state to the UI.
- Modify `src/app.rs`: store the observed native-composer state and compute access using the plugin-owned attachment count.
- Modify `src/ui/mod.rs`: block editing/submission when the native composer is occupied or cannot be verified; pass the expected attachment count to the action worker.
- Modify `src/ui/render.rs`: show a concise warning and hide the duplicate editor surface/cursor while guarded.
- Modify `src/toggle.rs`: replace a stale overlay mapping in one hotkey invocation after focusing the verified source pane.
- Create `src/local_time.rs`: local timestamp formatting with non-panicking conversion.
- Modify `src/ui/visual_rows.rs`: render the timestamp in the existing top gray prompt row.
- Modify `Cargo.toml` and `Cargo.lock`: add the pinned, minimal-feature Chrono dependency.
- Modify `README.md`: document composer protection, hotkey recovery, timestamp format, and the limitation of non-atomic Herdr APIs.
- Extend existing focused tests in `tests/transport_status.rs`, `tests/toggle_state.rs`, `tests/ui_render.rs`, `src/ui/runtime.rs`, `src/ui/mod.rs`, and module-local unit tests.

## Invariants to preserve

1. Never read, copy, clear, persist, or log native composer text. The classifier returns only a coarse state.
2. Submission is allowed only when the native composer is structurally verified as clear, or contains exactly the plugin-owned confirmed image placeholders. The periodic UI may also count in-flight plugin-owned image work solely to avoid a false warning while that work settles; the transport receives only the confirmed count.
3. `Unknown` is unsafe. Truncated, malformed, or unrecognized ANSI input must fail closed.
4. The transport preflight is authoritative; UI observation is only an early warning.
5. A user prompt keeps exactly one top gray row and one bottom gray row. The timestamp replaces the currently empty top row.
6. Existing history records remain valid; `timestamp_ms` is already persisted, so no journal migration is permitted.
7. Stale overlay recovery may delete only mappings proven to belong to the current source/overlay pair. Transient Herdr errors preserve state.

---

## Task 1: Add a pure native-composer classifier

**Files:**

- Create: `src/composer.rs`
- Modify: `src/lib.rs`
- Test: `src/composer.rs`

### Step 1: Write failing classifier tests

Create table-driven unit tests using `herdr_sdk::StyledText`/styled lines for these cases:

- Codex clear composer with a recognized footer and a dim placeholder such as `Write a prompt`.
- Codex clear composer with a dim suggestion such as `Summarize recent commits`; prove the result depends on dim/default-placeholder styling, not the literal English text.
- Codex composer containing ordinary user text -> `Occupied`.
- Codex composer containing only `[Image #1]` and `[Image #2]` tokens -> `OwnedAttachments(2)`.
- Codex composer containing an image token plus ordinary text -> `Occupied`.
- Codex surface with no recognizable current composer/footer -> `Unknown`.
- Codex truncated surface -> `Unknown`.
- Claude clear prompt box with two horizontal rules and a dim placeholder after the `❯` prompt marker.
- Claude prompt box with ordinary text -> `Occupied`.
- Claude prompt box containing exact image tokens only -> `OwnedAttachments(n)`.
- Claude prompt box without both structural rules -> `Unknown`.
- ANSI colors represented as `BrightBlack` or indexed color 8 are accepted as placeholder styling; arbitrary RGB text is not silently treated as a placeholder.
- Access policy: `Clear` permits zero expected attachments, `OwnedAttachments(n)` permits exactly `n`, and every mismatch/`Occupied`/`Unknown` blocks.

Use explicit expected values so no test merely checks that classification returned something.

Run:

```bash
cargo test composer --lib
```

Expected: compilation or assertion failures because the module and types do not exist yet.

### Step 2: Implement the minimal classifier API

Define privacy-safe enums:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeComposerState {
    Clear,
    OwnedAttachments(usize),
    Occupied,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComposerAccess {
    Ready,
    Occupied,
    Unknown,
}
```

Implement:

```rust
pub fn classify_native_composer(
    agent_kind: AgentKind,
    surface: &StyledText,
) -> NativeComposerState;

impl NativeComposerState {
    pub fn access(self, expected_attachments: usize) -> ComposerAccess;
}
```

Implementation rules:

- Reuse the structural rules documented by the official Herdr Codex/Claude manifests, but keep this module pure and independent of I/O.
- For Codex, locate the current prompt area and a recognized footer/status line. Reject a candidate if a later block marker (`•`, `■`, `✗`, `✓`) proves it is historical output.
- For Claude, locate the prompt box between the second horizontal rule from the bottom and the next rule, then find the `❯` prompt marker.
- Strip only structural prompt glyphs and whitespace before inspecting content.
- Treat a line as an empty placeholder only when all non-whitespace content is styled with an explicit dim/default-placeholder representation already emitted by the supported clients. Do not maintain a language-specific placeholder string list.
- Recognize image placeholders only as complete tokens matching `\[Image #\d+\]`, separated by whitespace/newlines. Any other character makes the state `Occupied`.
- Return only counts/states; never expose captured composer text from this module.

Add `mod composer;` (or the repository's existing visibility convention) in `src/lib.rs`.

### Step 3: Run the focused tests

```bash
cargo test composer --lib
```

Expected: all classifier and access-policy tests pass.

### Step 4: Review the task diff

Invoke `superpowers:requesting-code-review` for Task 1. Fix all correctness, privacy, structural-recognition, and test-quality findings before continuing.

### Step 5: Verify and commit Task 1

```bash
cargo fmt --check
cargo test composer --lib
git diff --check
git add src/composer.rs src/lib.rs
git commit -m "add native composer classifier"
```

Expected: formatting and tests pass; one focused commit is created.

---

## Task 2: Enforce composer safety in transport and UI

**Files:**

- Modify: `src/transport.rs`
- Modify: `src/ui/runtime.rs`
- Modify: `src/app.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/render.rs`
- Modify: `tests/transport_status.rs`
- Modify: `tests/ui_render.rs`
- Test: unit tests in `src/ui/runtime.rs`, `src/app.rs`, and `src/ui/mod.rs`

### Step 1: Write failing authoritative transport tests

Extend `tests/transport_status.rs` and `tests/support/mod.rs` only as needed so `ScriptedHerdr` can return styled visible-pane data and record whether `agent.prompt` was called.

Add tests proving:

- a verified clear native composer calls `agent.prompt` once with the original plugin text;
- exactly matching native image placeholders permit submission;
- ordinary native text blocks submission and never calls `agent.prompt`;
- an attachment-count mismatch blocks submission;
- an unrecognized or truncated surface blocks submission;
- a `pane.read` error blocks submission;
- error messages are privacy-safe and contain no captured native text;
- source validation still happens before composer inspection;
- composer inspection happens immediately before `agent.prompt`.

Change the intended transport signature in the tests to:

```rust
transport.submit(text, expected_attachments)
```

Run:

```bash
cargo test --test transport_status
```

Expected: compilation/assertion failures until the transport contract is implemented.

### Step 2: Implement transport preflight

In `AgentTransport::submit`:

1. validate the source pane/session identity;
2. call `client.pane_read_visible_ansi(source_pane, 200)` directly;
3. sanitize the ANSI surface once using the existing surface sanitizer;
4. classify with `classify_native_composer` and evaluate access against `expected_attachments`;
5. call `agent.prompt` only for `ComposerAccess::Ready`.

Do not call a helper that revalidates the session a second time. Return stable, privacy-safe errors:

- occupied: `native composer contains unsent input; prefix+m to return`
- unknown: `cannot verify native composer is safe to submit; prefix+m to return`

Do not attach surface fragments or raw client errors containing pane contents to these messages.

Run:

```bash
cargo test --test transport_status
```

Expected: all transport preflight tests pass.

### Step 3: Write failing observation/state tests

Add unit tests for the runtime observation path:

- one ANSI pane read produces status text, composer state, and optional blocked-interaction surface;
- the observer does not perform a separate plain-text read plus a second ANSI read;
- occupied and unknown classifications propagate through `SourceObservation`;
- clear/attachments classifications propagate without retaining native text.

Add `AppState` tests:

- real source observation defaults to `Unknown` until the first successful observation;
- UI observation access compares against confirmed plus pending plugin-owned images so an image insertion owned by this plugin does not flash a false conflict while it settles;
- matching image count produces `Ready`;
- occupied/unknown states preserve the plugin draft and attachments;
- a failed observation changes the state to `Unknown` instead of preserving a stale `Clear`.

Run:

```bash
cargo test ui::runtime --lib
cargo test app::tests --lib
```

Expected: failing tests for the new state and single-read observation contract.

### Step 4: Implement one-surface observation and state propagation

In `src/ui/runtime.rs`:

- add `native_composer: NativeComposerState` to `SourceObservation`;
- always read the visible source as ANSI once per observation cycle;
- sanitize once, derive the existing status text from the sanitized surface, classify the composer, and derive the blocked-interaction surface from the same value;
- on read/classification failure, publish `Unknown` and a connection/status error without retaining the previous safe state;
- change `ActionCommand::Submit` and `RuntimeHandle::submit` to carry `expected_attachments: usize`;
- pass that count to `transport.submit`.

In `src/app.rs`:

- store the latest `NativeComposerState`;
- expose a small method returning `ComposerAccess` using `confirmed + pending` image counts;
- initialize real runtime state as `Unknown` (test-only/default constructors may explicitly use `Clear` where isolation requires it);
- keep prompt and attachment recovery behavior unchanged.

Run:

```bash
cargo test ui::runtime --lib
cargo test app::tests --lib
```

Expected: observation and state tests pass.

### Step 5: Write failing interaction and render tests

Add interaction tests in `src/ui/mod.rs` for this priority order:

1. active blocked-question routing remains first;
2. PageUp/PageDown history navigation still works;
3. Esc interrupt still works;
4. connection validation still applies;
5. occupied/unknown native composer blocks text edits, paste, image paste, and Enter submission;
6. clear/matching-attachments state permits the existing editor behavior.

Also prove:

- pressing Enter captures the confirmed plugin attachment count before `AppEvent::PromptSubmitted` clears optimistic state;
- a transport race/failure restores the exact draft and attachment recovery payload once;
- opening Simple Prompts while native text exists never copies that native text into the plugin editor;
- returning to a safe composer reveals the preserved plugin draft again.

Extend `tests/ui_render.rs` to prove:

- occupied state renders `Native composer has unsent input · prefix+m to return`;
- unknown state renders `Unable to verify native composer · prefix+m to return`;
- plugin draft text, attachment labels, and editor cursor are hidden while guarded;
- history and status/working area remain visible;
- safe state renders the existing editor unchanged.

Run:

```bash
cargo test ui::tests --lib
cargo test --test ui_render
```

Expected: failures until input routing and guarded rendering are implemented.

### Step 6: Implement guarded interaction and rendering

In `src/ui/mod.rs`:

- preserve the approved event priority;
- after blocked routing/navigation/Esc/connection checks, inspect `app.composer_access()`;

- for `Occupied` and `Unknown`, ignore editor mutations and submission while preserving the plugin draft in memory;
- when submitting, compute `expected_attachments` before dispatching `PromptSubmitted`, then send it in `RuntimeHandle::submit`;
- keep current optimistic history and `SendFailed` restoration semantics.

In `src/ui/render.rs`:

- render the concise state-specific warning in the input area;
- do not render plugin draft or attachment labels while guarded;
- do not call `set_cursor_position` while guarded;
- do not obscure history, working/status output, or an active blocked question.

Run:

```bash
cargo test ui::tests --lib
cargo test --test ui_render
cargo test --test transport_status
```

Expected: all Task 2 focused tests pass.

### Step 7: Review the task diff

Invoke `superpowers:requesting-code-review` for Task 2. Specifically ask the reviewer to check:

- time-of-check/time-of-use limitations are documented, not misrepresented as atomic;
- the transport remains authoritative;
- no native text is stored/logged;
- blocked-question and Esc behavior did not regress;
- attachment counts are captured before optimistic clearing.

Fix all findings before continuing.

### Step 8: Verify and commit Task 2

```bash
cargo fmt --check
cargo test --test transport_status
cargo test --test ui_render
cargo test ui::runtime --lib
cargo test ui::tests --lib
git diff --check
git add src/transport.rs src/ui/runtime.rs src/app.rs src/ui/mod.rs src/ui/render.rs tests/transport_status.rs tests/ui_render.rs tests/support/mod.rs
git commit -m "guard submissions against native drafts"
```

Expected: focused tests pass and one coherent transport/UI commit is created.

---

## Task 3: Recover the hotkey from stale overlay pane state

**Files:**

- Modify: `src/toggle.rs`
- Modify: `tests/toggle_state.rs`

### Step 1: Write failing stale-overlay tests

Extend the scripted Herdr expectations in `tests/toggle_state.rs` for:

- the current pane reverse-maps to an overlay whose `pane.get` returns `pane_not_found`;
- the mapped source still exists and has the expected Codex/Claude agent identity;
- the plugin removes only that stale mapping, focuses the source pane, opens a replacement overlay, and persists the new pair during the same toggle invocation;
- an overlay opened from a stale current overlay uses the recovered source as its source context, not whatever pane happened to be active;
- if the source pane is also `pane_not_found`, the plugin removes only the exact stale source/overlay pair and returns a scoped error without opening elsewhere;
- permission, transport, timeout, and other transient errors do not delete state;
- unrelated mappings survive every recovery path;
- a healthy overlay still closes/focuses exactly as before.

Record call order and assert:

```text
pane.get(stale overlay)
pane.get(source)
agent identity verification
pane.focus(source)
plugin.pane.open(...)
state save
```

Run:

```bash
cargo test --test toggle_state stale_overlay
```

Expected: failures because `pane_not_found` is currently fatal.

### Step 2: Implement narrow recovery

Refactor `toggle.rs` into small helpers for:

- classifying only a real `pane_not_found` response;
- verifying a source pane still exists and still hosts the expected supported agent;
- removing one exact source/overlay mapping;
- opening an overlay from a verified source after explicitly focusing it.

When the current pane reverse-maps to a stale overlay:

1. confirm `pane.get(current_overlay)` is specifically `pane_not_found`;
2. recover the mapped source ID from persisted state;
3. verify the source pane and supported-agent identity;
4. remove only the old pair in memory;
5. focus the source pane;
6. open a fresh overlay;
7. persist the replacement mapping only after successful open.

If opening fails after focus, persist the stale-pair cleanup but do not invent an overlay ID. For non-`pane_not_found` failures, preserve the original mapping and return the original error.

Run:

```bash
cargo test --test toggle_state stale_overlay
cargo test --test toggle_state
```

Expected: all stale and existing healthy toggle tests pass.

### Step 3: Review the task diff

Invoke `superpowers:requesting-code-review` for Task 3. Require the review to examine cleanup scope, focus-before-open ordering, rollback/persistence behavior, and transient-error handling.

### Step 4: Verify and commit Task 3

```bash
cargo fmt --check
cargo test --test toggle_state
git diff --check
git add src/toggle.rs tests/toggle_state.rs
git commit -m "recover stale overlay hotkeys"
```

Expected: the focused suite passes and the recovery is isolated in one commit.

---

## Task 4: Render the local prompt timestamp in the existing gray row

**Files:**

- Create: `src/local_time.rs`
- Modify: `src/lib.rs`
- Modify: `src/ui/visual_rows.rs`
- Modify: `tests/ui_render.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`

### Step 1: Add the reviewed Chrono dependency

Add exactly:

```toml
chrono = { version = "0.4.45", default-features = false, features = ["clock", "std"] }
```

Do not enable `serde`, time-zone databases, or unrelated default features. Regenerate the lockfile only through Cargo:

```bash
cargo check --locked
```

The first run is expected to report that `Cargo.lock` needs an update. Then run:

```bash
cargo check
```

Expected: `Cargo.lock` changes only for Chrono and its required transitive dependencies; the crate remains compatible with the repository's Rust floor.

### Step 2: Write failing timestamp-helper tests

Create `src/local_time.rs` tests for a helper shaped like:

```rust
pub fn format_timestamp_at_offset(
    timestamp_ms: Option<u64>,
    offset: FixedOffset,
) -> Option<String>;

pub fn format_local_timestamp(timestamp_ms: Option<u64>) -> Option<String>;
```

Required cases:

- `1_786_638_720_000` at `+03:00` formats as `13.08.2026 19:32`;
- `None` returns `None`;
- an out-of-range millisecond value returns `None` without panicking;
- a negative/fixed-west offset boundary formats the correct local calendar date;
- formatting contains no seconds and always uses two-digit day/month/hour/minute.

Run:

```bash
cargo test local_time --lib
```

Expected: compilation/assertion failures until the helper exists.

### Step 3: Implement non-panicking time conversion

Use `DateTime::from_timestamp_millis` or `timestamp_millis_opt(...).single()` and `with_timezone`, never a panicking timestamp constructor. The production helper obtains `chrono::Local::now().offset()` only as the local offset source and delegates deterministic formatting to the fixed-offset helper.

Return `None` for missing or invalid timestamps. Do not substitute the current time for historical records with no timestamp.

Run:

```bash
cargo test local_time --lib
```

Expected: all helper tests pass.

### Step 4: Write failing visual-row tests

Extend `tests/ui_render.rs` or module-local `visual_rows` tests to prove:

- the first gray row of a user prompt contains `13.08.2026 19:32` for the fixed timestamp/offset fixture;
- the prompt body still begins on the following row;
- the bottom gray row is unchanged;
- total prompt-band height does not grow;
- the timestamp row uses the existing prompt fill and a subdued/dim foreground;
- narrow widths clip or truncate the timestamp within one row instead of wrapping;
- absent/invalid timestamps leave the top row empty but styled, preserving legacy records;
- assistant/history formatting is unchanged.

To keep rendering deterministic, pass a formatter/offset into the visual-row construction at the smallest practical seam; do not make snapshot tests depend on the machine time zone.

Run:

```bash
cargo test --test ui_render timestamp
cargo test visual_rows --lib
```

Expected: failures because the top row is currently empty.

### Step 5: Render the timestamp without adding rows

In `HistoryDocument::from_app` (or the nearest existing user-prompt row builder):

- replace `filled_empty_row(prompt_fill())` for the top row with a one-line filled timestamp row;
- use `Message.timestamp_ms`, never journal file modification time or render time;
- keep `prompt_fill()` unchanged;
- use `BrightBlack`/dim styling consistent with the approved appearance;
- clip the timestamp to the available width and fill the remainder of that same row;
- retain the existing content rows and bottom empty filled row exactly.

Run:

```bash
cargo test local_time --lib
cargo test --test ui_render timestamp
cargo test visual_rows --lib
```

Expected: timestamp and layout tests pass.

### Step 6: Review the task diff

Invoke `superpowers:requesting-code-review` for Task 4. Check dependency minimality, invalid timestamp handling, deterministic tests, one-row clipping, and absence of history migration.

### Step 7: Verify and commit Task 4

```bash
cargo fmt --check
cargo test local_time --lib
cargo test --test ui_render timestamp
cargo test visual_rows --lib
git diff --check
git add Cargo.toml Cargo.lock src/local_time.rs src/lib.rs src/ui/visual_rows.rs tests/ui_render.rs
git commit -m "show local prompt timestamps"
```

Expected: focused tests pass and dependency/code changes are committed together.

---

## Task 5: Document, verify, merge, and reload the installed plugin

**Files:**

- Modify: `README.md`
- Verify: all source and test files changed in Tasks 1-4

### Step 1: Update user and maintainer documentation

Document in `README.md`:

- Simple Prompts displays prompt/answer history while keeping the native working/status area;
- user prompts show local `DD.MM.YYYY HH:MM` in the existing top gray row;
- if the native Codex/Claude composer contains unsent text, Simple Prompts preserves its own draft, disables duplicate editing/submission, and asks the user to return with `prefix+m`;
- matching plugin-owned image placeholders remain safe;
- an unrecognized composer is blocked conservatively;
- stale overlay pane metadata is repaired automatically on the next hotkey use when the source pane is still valid;
- Herdr 0.7.5 has no atomic inspect-and-submit API, so the final preflight closes the reported single-client duplication path but cannot promise cross-client atomicity;
- existing history does not require migration.
- the bulk `agent_session == null` registration command repairs missing native-integration metadata only; it is not a general hotkey repair;
- `pane_not_found` in the plugin action log indicates a stale overlay context that the next toggle now repairs.

Keep installation source-only and do not add binaries or generated artifacts.

### Step 2: Run the full quality gate

Invoke `superpowers:verification-before-completion`, then run from the task worktree:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release --locked
git diff --check
git status --short
```

Expected:

- formatting passes;
- Clippy has zero warnings;
- all tests pass (baseline was 229 tests before this change, so the new total must be higher);
- locked build succeeds;
- only the intended README change remains before its commit.

If any command fails, diagnose the root cause with `superpowers:systematic-debugging`, add/regress a test first, fix minimally, and rerun the complete gate from the start.

### Step 3: Perform final review

Invoke `superpowers:requesting-code-review` against the complete branch diff from `develop`/the recorded base. Resolve every high/medium correctness issue and rerun the full quality gate after any code change.

Pay special attention to:

- privacy: no native draft contents in app state, journal, logs, or errors;
- conservative classification across Codex and Claude;
- exact attachment-count handling;
- guarded UI event priority and active interactive questions;
- stale overlay focus/open ordering;
- timestamp layout parity and time-zone correctness;
- README claims matching the real non-atomic guarantee.

### Step 4: Commit documentation and final adjustments

```bash
git add README.md
git commit -m "document composer safety and timestamps"
git status --short
```

Expected: clean task worktree.

### Step 5: Merge through the repository's existing branch workflow

Invoke `superpowers:finishing-a-development-branch`. Because the user has already requested integration, use the repository's established local integration branch (currently the main checkout branch for this standalone plugin) and preserve unrelated user changes.

Before merging:

```bash
git log --oneline --decorate --max-count=10
git status --short
```

Then merge the verified task branch non-destructively from the integration checkout. Do not reset, force-push, or discard unrelated work. After the merge, rerun:

```bash
cargo test --all-targets --all-features
cargo build --release --locked
git status --short
```

Expected: integration checkout contains all task commits and remains clean.

### Step 6: Rebuild/relink and activate the plugin

Use the repository's existing documented source-only install/relink command; do not invent a second installation path. Inspect the current symlink/config target before changing it. Rebuild the installed source checkout, then restart only the current Simple Prompts overlay/pane so the new binary is loaded without disrupting unrelated Herdr sessions.

Run the documented smoke checks:

1. existing session with clear native composer: `prefix+m` opens and closes Simple Prompts;
2. stale-overlay reproduction: one `prefix+m` invocation recreates the overlay from the correct source pane;
3. native composer containing text: plugin shows the guard, preserves plugin draft, and sends nothing;
4. native composer clear: one prompt is submitted exactly once;
5. image paste: matching placeholders remain sendable;
6. user history rows show local date and time;
7. when a live Claude session is available, repeat the native-draft conflict case there;
8. active Codex and Claude interactive-question surfaces remain operable;
9. history still scrolls and the working area remains visible.

Never print session IDs or transcript contents during smoke verification.

### Step 7: Record final evidence

Report:

- branch and merge commit identifiers;
- exact verification commands and pass counts;
- installed/relinked plugin status;
- smoke-check outcomes;
- whether any known limitation remains (the Herdr API's cross-client non-atomic window);
- Obsidian maintenance decision: no CoachTM Obsidian update is needed because this change is isolated to the standalone Herdr plugin repository.

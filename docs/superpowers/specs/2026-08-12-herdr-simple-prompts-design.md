# Herdr Simple Prompts Design

**Date:** 2026-08-12  
**Status:** Approved in conversation  
**Target:** Herdr 0.7.5+, Codex CLI, Claude Code  
**Platforms:** macOS and Linux

## Summary

Herdr Simple Prompts is a source-only Rust plugin that opens a full-screen
terminal overlay for the agent in the currently focused Herdr pane. The overlay
shows only:

1. real user prompts,
2. final agent answers,
3. the active prompt immediately after submission,
4. the agent's current `Working` state,
5. a native-like multiline composer, and
6. a truthful bottom status line.

Reasoning, commentary, tool calls, tool results, system/developer messages, and
subagent traffic remain in the native agent pane and are never rendered in the
simple view.

The mode is toggled with a user-configured `prefix+m` keybinding. This avoids
Herdr's default `prefix+p` binding for the previous tab.

## Goals

- Offer a calm prompt/final-answer view without replacing or wrapping the agent.
- Preserve the current Codex or Claude session and let it continue running.
- Show a submitted prompt immediately, including while the agent is working.
- Forward text, multiline paste, large paste, image paste, and interrupt actions
  to the native agent input path.
- Resume existing session history when the overlay opens.
- Work locally and through Herdr remote attach on macOS and Linux.
- Be publishable as a normal public Herdr plugin repository.
- Build from inspectable source on the user's machine, with no downloaded
  release binary.

## Non-goals

- Replacing the native Codex or Claude TUI.
- Rendering reasoning, progress commentary, tools, approvals, or tool output.
- Editing or rewriting native transcript files.
- Rendering image pixels inside the terminal; images use compact attachment
  placeholders.
- Browsing unrelated sessions or providing a global conversation index.
- Supporting agents other than Codex and Claude in version 0.1.
- Supporting Windows in version 0.1.
- Providing a native, non-terminal Herdr pane; Herdr plugin API v1 exposes
  managed terminal panes.

## User experience

### Toggle

The manifest exposes a `toggle` plugin action. The README documents this binding:

```toml
[[keys.command]]
key = "prefix+m"
type = "plugin_action"
command = "herdr.simple-prompts.toggle"
description = "Toggle Simple Prompts"
```

When invoked from a supported Codex or Claude pane, the action opens and focuses
a Herdr-managed overlay. When invoked again from the overlay, it closes the
overlay and restores focus to the source pane. If an overlay already exists for
the source pane but is not focused, the action focuses it instead of opening a
duplicate.

The overlay never terminates, restarts, reparents, or resumes the source agent.
Closing it returns the user to the unchanged native agent pane.

### Layout

The screen contains, from top to bottom:

1. scrollable prompt/final-answer history,
2. the current user prompt if it has already been submitted,
3. `Working (<elapsed> • esc to interrupt)` while the source agent is working,
4. the multiline composer,
5. a native-like source status line.

The composer is always anchored at the bottom. New prompts and final answers
auto-scroll into view. Manual scrolling upward suspends auto-scroll until the
user returns to the bottom.

### Message hierarchy and sticky prompt context

User prompts use a full-width, neutral raised band with a compact `YOU` label.
Final agent answers remain unboxed and start with a compact green `ANSWER`
label. The distinction therefore does not depend on color alone: role labels,
surface treatment, and spacing all identify message ownership. The palette uses
the terminal theme's existing colors and avoids fixed RGB backgrounds that
could become unreadable in light or customized terminal themes.

While history scrolls through a turn, the start of that turn's user prompt acts
as a sticky section header:

- At most the first two visual rows remain at the top of the history viewport.
  A visual row is measured after Unicode-aware wrapping to the current history
  width, not by newline-delimited source lines.
- A one-row prompt pins only one row. An empty-text image prompt uses its first
  attachment placeholder as context.
- The sticky copy appears only after those rows would otherwise leave the
  viewport; it never duplicates a prompt that is still visible in its natural
  position.
- The following user prompt is the section boundary. As its first visual row
  reaches the top boundary, it pushes the previous sticky prompt upward one row
  at a time. Once the new prompt occupies the boundary, the old prompt is gone
  and the new one becomes eligible to stick.
- The header never overlaps the following prompt, agent answer, error row,
  working indicator, composer, or footer. If the history viewport is too short,
  it pins only the rows that fit while preserving at least one row for scrolling
  history.

Sticky context follows the same manual scroll offset as the history. Returning
to the bottom restores normal auto-scroll behavior without changing which turn
owns the header.

### Composer behavior

- `Enter` submits the current prompt.
- `Shift+Enter` inserts a newline when the terminal reports the modifier.
- `Ctrl+J` is the portable newline fallback.
- `Esc` interrupts the source agent only while it is working.
- Pasted text is inserted atomically and preserves all newlines.
- The plugin imposes no arbitrary prompt-length truncation. Herdr or agent-side
  limits are surfaced as explicit send errors.
- Resize reflows the screen without losing the composer buffer or cursor.
- A draft is stored in plugin state and survives a temporary overlay close.

### Image paste

Image paste stays on the native Codex/Claude attachment path:

- For a local Herdr client, the overlay forwards the image-paste key to the
  source pane and verifies the attachment marker from the source screen.
- For remote attach, Herdr stages the local image on the remote host and pastes
  its private temporary path. The overlay recognizes a Herdr-staged image path
  and forwards it to the source agent using bracketed paste.
- The composer and history render the attachment as `[Image #N]` with an
  optional sanitized basename/path, not as terminal image pixels.
- Submission sends composer text while preserving already attached native
  images in the source agent composer.
- If the native attachment marker cannot be verified, the overlay reports the
  failure instead of pretending the image was attached.

## Architecture

The repository produces one Rust executable with two entry modes:

- `toggle`: short-lived Herdr plugin action.
- `ui`: long-lived managed overlay pane.

The manifest declares both the action and the pane entrypoint. Herdr injects the
plugin root, config/state paths, socket path, invocation context, and current
pane identifiers.

### Toggle controller

The toggle controller:

1. reads the focused pane from the invocation context,
2. loads the per-Herdr-session overlay registry from the plugin state directory,
3. validates any remembered overlay through the Herdr socket,
4. closes the overlay and focuses its source when invoked from that overlay,
5. focuses an existing overlay for the source when one exists, or
6. opens the declared overlay pane with the source pane id passed as an
   authoritative environment value.

State writes are atomic and private to the user. Stale overlay records are
discarded after validation.

### Overlay process

The overlay owns five cooperating components:

1. **Herdr client** — request/response calls and event subscription over the
   injected local socket.
2. **Agent adapter** — source-agent identity, native transcript resolution,
   source screen inspection, prompt submission, and interrupt forwarding.
3. **Transcript follower** — initial parse plus incremental JSONL tailing with
   truncation/replacement detection.
4. **Application reducer** — normalized turns, optimistic prompts, working state,
   attachment state, draft, scrolling, and errors.
5. **Terminal UI** — raw input, bracketed paste, Unicode-safe editor behavior,
   resize handling, and rendering.

Components communicate through bounded standard-library channels. Filesystem
following uses a small polling interval and file metadata instead of a platform
watcher dependency. The UI thread remains the only owner of terminal state.

### Dependency policy

The direct dependency set is intentionally small and locked:

- `ratatui` for deterministic layout and rendering,
- `crossterm` for raw terminal input and bracketed paste,
- `serde` and `serde_json` for Herdr and transcript payloads,
- `unicode-width` for cursor and wrapping correctness.

The implementation uses standard threads, channels, sockets, filesystem APIs,
and `Instant`; it does not require Tokio, an HTTP client, telemetry, or runtime
network access. `Cargo.lock` is committed. Any dependency added later requires
an explicit security and necessity review.

## Herdr integration

The plugin requires Herdr 0.7.5 or newer and uses only documented plugin/socket
surfaces:

- invocation context and managed plugin environment variables,
- managed overlay panes,
- pane focus/close/read/input operations,
- agent inspect/prompt/send-keys operations,
- agent/pane lifecycle subscriptions.

The source pane id is captured before opening the overlay. Calls never rely on
the overlay becoming the globally focused pane. Before every mutating input
operation, the plugin confirms that the source pane still exists and still
contains the expected supported agent session.

## Transcript resolution and normalization

The transcript follower never writes native session files. It resolves the file
from Herdr's detected agent kind, native session id, and source working directory.
Environment overrides such as `CODEX_HOME` and Claude's configuration directory
are respected before conventional user paths.

### Normalized model

```text
Turn {
  prompt: Message,
  final_answer: Option<Message>,
  submission: Native | Optimistic,
}

Message {
  stable_id: String,
  text: String,
  attachments: Vec<Attachment>,
  timestamp: Option<Timestamp>,
}
```

The UI renders only `Turn`. Raw transcript event variants do not reach the view
layer.

### Codex adapter

The Codex adapter accepts:

- `event_msg` payloads whose type is `user_message`, and
- `event_msg` payloads whose type is `agent_message` and phase is
  `final_answer`.

It rejects commentary-phase agent messages, reasoning, tools, tool results,
system/developer context, environment context, approvals, and subagent items.
Image references associated with a real user message become normalized
attachments.

### Claude adapter

The Claude adapter accepts real top-level user prompt content and visible
assistant text that completes that user turn. It rejects:

- `thinking` blocks,
- `tool_use` blocks,
- tool-result user records,
- system/meta/injected context,
- progress and hook records,
- sidechain/subagent records.

For tool-using turns, the adapter commits only the terminal visible assistant
text after the tool cycle is complete. A prompt with no completed visible answer
remains a turn with `final_answer = None`.

### Optimistic reconciliation

Submitting a prompt appends an optimistic turn before sending input to Herdr.
When the corresponding native user event arrives, the reducer reconciles it by
submission order, normalized text, attachment count, and a bounded time window.
The native stable id replaces the optimistic id. It never appends a second copy.

Prompts submitted while the agent is working are kept in source submission
order. Final answers attach in native transcript order. If a request is rejected,
the optimistic entry is marked locally as unsent and the composer is restored;
it is not silently presented as native history.

## Working and status state

The source agent's lifecycle state comes from Herdr, not transcript timing.
Elapsed working time starts on a Herdr working transition and stops on done,
waiting, interruption, source close, or session replacement.

The bottom status adapter reads the visible source screen and extracts only
agent-specific, verified fields such as model, cwd, branch, and usage. When an
agent version changes its layout and a field cannot be proven, the plugin omits
that field and shows the known agent kind/cwd/state. It never fabricates usage
or quota values.

## Failure behavior

- **Unsupported pane:** show that Simple Prompts requires a detected Codex or
  Claude agent and do not open a misleading empty view.
- **Source pane closed:** disable submission and offer the toggle key to return.
- **Session changed:** stop following the old transcript and require reopening
  against the new session.
- **Socket disconnected:** retain visible history and draft, retry read-only
  connection with bounded backoff, and disable sends until revalidated.
- **Transcript unavailable:** keep the composer usable only after native agent
  validation, show the resolution error, and never substitute pane screen text
  as conversation history.
- **Malformed JSONL tail:** retain the last valid state, wait for a completed
  line, and surface persistent parse failures without crashing.
- **Send failure:** restore the submitted text/attachments to the composer and
  visibly mark the optimistic item as unsent.
- **Terminal panic/exit:** restore raw mode, cursor visibility, and bracketed
  paste state through an RAII terminal guard.

## Persistence and privacy

The plugin state directory stores only:

- source-to-overlay registry data,
- the active draft and attachment placeholders,
- small UI preferences such as scroll position if needed.

It does not copy full conversation transcripts into its own database. State
files use user-only permissions and atomic replacement. The plugin has no
telemetry, analytics, update checker, HTTP client, or runtime network access.

## Source-only distribution

The public repository contains source, manifest, tests, documentation, license,
and a committed lockfile. It does not publish or download executable release
artifacts.

`herdr-plugin.toml` declares the local build directly:

```toml
[[build]]
command = ["cargo", "build", "--locked", "--release"]
```

Installation is:

```bash
herdr plugin install <github-owner>/herdr_simple_prompts
```

Herdr clones the inspectable repository, presents its trust/build preview, and
builds the executable on the user's machine. A Rust toolchain is therefore an
explicit prerequisite. GitHub tags identify releases, while GitHub Actions only
test and verify source builds; release workflows do not upload binaries.

For marketplace discovery, the eventual public repository adds the GitHub topic
`herdr-plugin`. Publication itself is outside the initial local implementation
scope.

## Testing strategy

### Unit tests

- Codex fixtures: user prompt, commentary, final answer, tool activity,
  interruption, attachments, and subagent exclusion.
- Claude fixtures: simple answer, thinking, tool cycle, tool results, sidechain,
  interruption, and attachments.
- Optimistic/native reconciliation and duplicate prevention.
- Editor operations, Unicode widths, multiline paste, large paste, cursor
  movement, and draft restoration.
- Reducer state transitions for working, done, interruption, disconnect,
  session replacement, and send failure.
- Status extraction fixtures for supported Codex and Claude layouts.
- Message-role hierarchy remains visible without relying on color alone.
- Sticky prompt rows for short, long, wrapped, Unicode, image-only, and
  constrained-height histories.
- The next prompt pushes the previous sticky context out one visual row at a
  time without overlap or duplication.

Fixtures are synthetic and contain no real user transcript data.

### Integration tests

- Fake Herdr Unix socket for request ordering, subscriptions, errors, and
  reconnection.
- Temporary transcript files for initial load, append, partial JSON line,
  truncation, and replacement.
- Pseudo-terminal harness for raw-mode restoration, bracketed paste, resize,
  large multiline input, image-path forwarding, and interrupt forwarding.
- Toggle lifecycle: open, focus existing, close, stale-state cleanup, and source
  focus restoration.

### Verification gates

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
```

A manual smoke matrix then covers local and remote Herdr sessions with current
Codex and Claude versions, including text, large paste, image paste, working
state, interruption, overlay toggle, and returning to the unchanged native pane.

## Repository deliverables

```text
herdr_simple_prompts/
├── herdr-plugin.toml
├── Cargo.toml
├── Cargo.lock
├── src/
├── tests/
├── docs/
│   └── superpowers/specs/
├── .github/workflows/ci.yml
├── README.md
├── LICENSE
└── .gitignore
```

The initial public-facing documentation will explain the source-only trust
model, Rust prerequisite, Herdr/Codex/Claude prerequisites, `prefix+m` setup,
installation, local development with `herdr plugin link`, privacy guarantees,
limitations, and troubleshooting.

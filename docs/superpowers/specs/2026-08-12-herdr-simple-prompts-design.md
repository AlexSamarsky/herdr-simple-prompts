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

While Herdr reports the source agent as `blocked`, the source agent's live
native interaction surface replaces that normal layout temporarily.

Final answers retain the source agent's visible ANSI-derived styling. History
uses one explicit Unicode-aware visual-row model, so styling, wrapping, scroll
geometry, and sticky prompt geometry all operate on the same rows. When the
source agent enters Herdr's `blocked` state, the normal history/composer view is
temporarily replaced by a sanitized live view of the native interactive
question or approval surface, and user input is passed through to the source
pane.

Reasoning, commentary, tool calls, tool results, system/developer messages, and
subagent traffic remain outside ordinary Simple Prompts history. A blocked
interaction is a temporary live passthrough view, not a stored history entry.

The mode is toggled with a user-configured `prefix+m` keybinding. This avoids
Herdr's default `prefix+p` binding for the previous tab.

## Goals

- Offer a calm prompt/final-answer view without replacing or wrapping the agent.
- Preserve the current Codex or Claude session and let it continue running.
- Show a submitted prompt immediately, including while the agent is working.
- Forward text, multiline paste, large paste, image paste, and interrupt actions
  to the native agent input path.
- Resume existing session history when the overlay opens.
- Preserve the visible styling of newly observed Codex and Claude final answers
  and restore that styling when the overlay is reopened.
- Keep visible prompt/final history in a pane-and-session-scoped private journal
  whose lifecycle follows the source pane.
- Let native questions, choices, and approval surfaces remain usable without
  leaving the overlay.
- Work locally and through Herdr remote attach on macOS and Linux.
- Be publishable as a normal public Herdr plugin repository.
- Build from inspectable source on the user's machine, with no downloaded
  release binary.

## Non-goals

- Replacing the native Codex or Claude TUI.
- Adding reasoning, progress commentary, tools, approvals, questions, or tool
  output to the conversation history. A live native question or approval may be
  shown only while the agent is blocked and disappears after interaction.
- Editing or rewriting native transcript files.
- Rendering image pixels inside the terminal; images use compact attachment
  placeholders.
- Browsing unrelated sessions or providing a global conversation index.
- Retaining a pane's visible-history journal after the source pane/session has
  been proven gone.
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

History is laid out into final terminal rows before geometry is calculated.
The renderer does not ask Ratatui to wrap the same content a second time. This
single-row model is the source of truth for viewport height, bottom offset,
manual scrolling, sticky selection, full-width prompt backgrounds, and final
rendering. Consequently the newest lines remain reachable and every wrapped
prompt row is filled to the right edge.

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
- The two-row limit applies only to the sticky copy. Ordinary prompt text in
  the natural history entry remains complete and scrollable. When Codex or
  Claude represents a large paste with its native compact marker (for example,
  a marker containing `1000 chars`), the plugin preserves that marker exactly
  and does not reconstruct or reveal the hidden pasted body. This keeps pasted
  logs compact in both the natural history entry and sticky context.
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

### Styled final answers

Styled output is the primary final-answer representation, not an optional
decoration over a plain renderer. After the transcript adapter emits a real
final answer, the runtime asks Herdr for the source agent's recent unwrapped
terminal output in ANSI format. It locates the final-answer block by comparing
the ANSI-stripped candidate with the canonical transcript text, removes only
known Codex/Claude presentation chrome around that block, and converts the
remaining SGR state into styled message spans.

Because transcript append and terminal paint can arrive in either order, ANSI
capture retries for a short bounded window after the final event. It accepts a
capture only after exact canonical-text matching; timing alone never selects a
terminal block.

The sanitizer accepts printable text plus a conservative SGR subset: named,
indexed, and RGB foreground/background colors and the bold, dim, italic, and
underline modifiers. Cursor movement, alternate-screen commands, OSC/title
commands, hyperlinks, clipboard commands, and all other terminal controls are
discarded. No ANSI command is ever replayed into the user's terminal.

The green `ANSWER` role label remains plugin-owned. The answer body retains the
source agent's colors and emphasis and is reflowed by the same visual-row engine
as the rest of history. Style runs split safely at UTF-8 boundaries and survive
Unicode-aware wrapping.

If an old final answer is no longer present in Herdr's scrollback and has no
saved styled record, the plugin renders the canonical transcript text with a
small built-in styled-Markdown fallback. The fallback covers paragraphs,
headings, lists, inline code, fenced code, emphasis, and links without adding a
runtime dependency. It is deterministic fallback presentation and is never
recorded as if it were native ANSI. Every newly observed answer is captured and
persisted as the exact sanitized styled representation, so it is not
reconstructed on a later overlay open.

### Interactive blocked mode

Herdr's source-agent status is authoritative. While it is `blocked`, Simple
Prompts temporarily replaces the ordinary history/working/composer region with
`INTERACTION REQUIRED` and a frequently refreshed, sanitized ANSI snapshot of
the native agent's visible question, choice, permission, or approval surface.
The existing history and editor draft remain in memory and persisted state but
are hidden, not discarded.

Text input and the native interaction keys (`Up`, `Down`, `Left`, `Right`,
`Tab`, `BackTab`, `Space`, `Enter`, `Backspace`, `Delete`, and `Esc`) are sent
to the source pane through Herdr input APIs. Mouse interaction is not
translated in version 0.1. The overlay never parses a provider question into a
plugin-owned form or invents answer semantics. When the source leaves
`blocked`, the ordinary history and unchanged composer automatically return.

If the live ANSI snapshot cannot be read or sanitized, the overlay does not
guess. It displays the error and directs the user to `prefix+m` to answer in
the unchanged native pane.

### Composer behavior

- `Enter` submits the current prompt.
- `Shift+Enter` inserts a newline when the terminal reports the modifier.
- `Ctrl+J` is the portable newline fallback.
- `Esc` interrupts the source agent only while it is working.
- A paste for which Rust `text.chars().count()` is below 1,000 is inserted
  normally and preserves all newlines.
- A paste for which `text.chars().count()` is 1,000 or more is shown
  immediately as one compact atomic token: `[Pasted Content · N chars]`. `N`
  is the actual `chars()` count. Multiple large pastes produce separate tokens.
- The compact token is only a display projection. The editor retains the exact
  original paste and sends its full text, including every newline, to the
  source agent. The cursor skips over the token; `Backspace` or `Delete` at its
  edge removes the whole pasted segment instead of revealing or partially
  editing the log body.
- The plugin imposes no arbitrary prompt-length truncation. Herdr or agent-side
  limits are surfaced as explicit send errors.
- Resize reflows the screen without losing the composer buffer or cursor.
- A draft is stored as chunked editor state, including the full source text
  behind compact paste tokens, and survives a temporary overlay close.

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

The overlay owns eight cooperating components:

1. **Herdr client** — request/response calls and event subscription over the
   injected local socket.
2. **Agent adapter** — source-agent identity, native transcript resolution,
   source screen inspection, prompt submission, and interrupt forwarding.
3. **Transcript follower** — initial parse plus incremental JSONL tailing with
   truncation/replacement detection.
4. **ANSI capture and sanitizer** — source-screen ANSI reads, exact final-block
   matching, safe SGR decoding, and live blocked-surface snapshots.
5. **Visible-history journal** — pane/session-scoped prompt and final-answer
   records with sanitized style runs and lifecycle cleanup.
6. **Application reducer** — normalized turns, optimistic prompts, working and
   blocked state, attachment state, draft, scrolling, and errors.
7. **Visual-row engine** — the only history wrapper and geometry owner for
   Unicode cells, full-width styles, scrolling, and sticky sections.
8. **Terminal UI** — raw input, bracketed paste, Unicode-safe editor behavior,
   interaction passthrough, resize handling, and rendering.

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
network access. ANSI sanitization and the styled-Markdown fallback are
implemented in visible Rust source without another dependency. `Cargo.lock` is
committed. Any dependency added later requires an explicit security and
necessity review.

## Herdr integration

The plugin requires Herdr 0.7.5 or newer and uses only documented plugin/socket
surfaces:

- invocation context and managed plugin environment variables,
- managed overlay panes,
- pane focus/close/read/input operations,
- agent inspect/prompt/send-keys operations,
- ANSI `agent.read`/`pane.read` output,
- `pane_closed` and agent-status lifecycle subscriptions.

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
  submission: Native | Optimistic {
    complete_text: String,
    editor_snapshot: EditorBuffer,
  },
}

Message {
  stable_id: String,
  text: String,
  attachments: Vec<Attachment>,
  timestamp: Option<Timestamp>,
  presentation: NativeAnsi(Vec<StyleRun>) | MarkdownFallback,
}

StyleRun {
  start_byte: usize,
  end_byte: usize,
  foreground: Option<AnsiColor>,
  background: Option<AnsiColor>,
  modifiers: StyleModifiers,
}
```

The UI renders only `Turn`. Raw transcript event variants do not reach the view
layer. `Message.text` is always the display-safe representation. The complete
text and editor snapshot exist only on a local optimistic delivery so they can
support transport reconciliation and lossless failure recovery.

Transcript text remains the canonical semantic value. Style runs only annotate
byte ranges in that exact text and are accepted only when their text
fingerprint, UTF-8 boundaries, ordering, and bounds validate. Styled capture
never changes message identity or reconciliation.

### Editor and compact-paste model

The editor models input as ordered chunks rather than replacing a large paste
with its label:

```text
EditorChunk = Text(String) | LargePaste {
  source_text: String,
  character_count: usize,
}
```

The rendering projection flattens `Text` chunks verbatim and renders each
`LargePaste` as `[Pasted Content · N chars]`. The submission projection flattens
both chunk types to their original text. Cursor movement and deletion treat a
`LargePaste` as one atomic editor item, while adjacent ordinary text remains
editable.

Submission produces both projections in one operation. The runtime transport
receives only the complete submission projection. The optimistic turn receives
the compact rendering projection and temporarily retains the complete text for
native-event reconciliation and send-failure restoration; the complete copy is
discarded after successful reconciliation. Draft persistence serializes the
chunk kinds, not just the flattened source text, so reopening the overlay does
not expand a compact token.

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
submission order, attachment count, a bounded time window, and either normalized
complete-text equality or equality of the compact paste-count signature. The
native stable id replaces the optimistic id. It never appends a second copy. If
the provider's native event contains the full large paste, the reducer keeps the
already compact local rendering; if the provider emits a native compact marker,
that marker is preserved.

Prompts submitted while the agent is working are kept in source submission
order. Final answers attach in native transcript order. If a request is rejected,
the optimistic entry is marked locally as unsent and the chunked composer,
including full large-paste source text, is restored; it is not silently
presented as native history.

## Working and status state

The source agent's lifecycle state comes from Herdr, not transcript timing.
Elapsed working time starts on a Herdr working transition and stops on done,
blocked, waiting, interruption, source close, or session replacement. A blocked
transition activates the native interaction surface; it is not interpreted as a
final answer.

The bottom status adapter reads the visible source screen and extracts only
agent-specific, verified fields such as model, cwd, branch, and usage. When an
agent version changes its layout and a field cannot be proven, the plugin omits
that field and shows the known agent kind/cwd/state. It never fabricates usage
or quota values.

## Failure behavior

- **Unsupported pane:** show that Simple Prompts requires a detected Codex or
  Claude agent and do not open a misleading empty view.
- **Source pane closed:** disable submission and offer the toggle key to return.
- **Source pane closed while the overlay is alive:** delete the source
  pane/session history namespace after the authoritative `pane_closed` event,
  then disable the overlay.
- **Session changed:** stop following the old transcript and require reopening
  against the new session.
- **Socket disconnected:** retain visible history and draft, retry read-only
  connection with bounded backoff, and disable sends until revalidated.
- **ANSI capture mismatch:** retain the canonical final text, render the
  styled-Markdown fallback, and never attach styles from a different terminal
  block.
- **Blocked snapshot unavailable:** keep the draft untouched, show a concise
  error, and direct the user to the unchanged source pane with `prefix+m`.
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

The plugin state directory contains four deliberately separate classes of
state:

- source-to-overlay registry data,
- the active draft and attachment placeholders,
- compact-paste display metadata for prompts submitted through the plugin, and
- a visible-history journal scoped to one source pane and one native session.

The journal is auditable JSON Lines under:

```text
history/<safe-source-pane-id>/<native-session-id>.jsonl
```

Each record is versioned and keyed by the native stable message id. It stores
only the display-safe user prompt or visible final-answer text, sanitized
attachment labels, timestamp/order data, a text fingerprint, and validated
style ranges. Repeating a stable id appends an upsert whose latest valid record
wins, allowing a Markdown fallback to be replaced by later native ANSI capture
without rewriting earlier bytes. It never stores reasoning, commentary, tool
traffic, tool results, system context, native interaction surfaces, or hidden
large-paste bodies. A user prompt containing a large paste is journaled only
with its compact marker.

This journal is an intentional private copy of the visible conversation subset
so the plugin can restore exactly what it previously showed instead of
reconstructing presentation on every open. It is not a global conversation
database and cannot be queried across panes from the UI.

On reopen, journal records are loaded first and then reconciled with the native
transcript by session and stable message id. A journal record supplies the exact
previous display text and styles; transcript-only messages are added without
duplicating recorded messages and use native capture when still available or
the explicit Markdown fallback otherwise.

Compact-paste history metadata is scoped by native session and message id and
contains only paste ranges, character counts, and a deterministic integrity
fingerprint; it never contains the pasted body. This lets the plugin retain the
compact rendering after reopening even when a provider transcript stores the
full prompt. The unsent draft is the only plugin state that can contain the
original hidden pasted text.

All directories use mode `0700`; journal, draft, and registry files use `0600`.
New records are flushed without blocking the UI, tolerate an incomplete final
line, and are size-bounded by the source pane's lifecycle rather than a global
retention pool.

Lifecycle cleanup follows the approved non-resident policy:

1. While the overlay is running, it subscribes to Herdr `pane_closed`. Closing
   the source pane deletes that pane/session journal, draft, compact metadata,
   and overlay registry entry immediately.
2. Closing only the overlay keeps the journal so reopening the same source
   pane/session restores history.
3. Every toggle and overlay startup compares saved namespaces with live Herdr
   panes and detected native session ids. Proven-missing panes and replaced
   sessions have all pane-scoped state deleted before normal use.
4. If live validation is unavailable, state is not deleted merely because the
   socket is temporarily down. Orphan namespaces that remain unverifiable for
   seven days are removed on the next plugin invocation as crash/restart
   cleanup. No detached watcher or permanent plugin daemon is created.

The plugin has no telemetry, analytics, update checker, HTTP client, or runtime
network access.

For prompts that predate the plugin metadata, the plugin preserves any compact
marker already recorded by Codex or Claude but does not guess that arbitrary
historical text originated from a paste.

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
- Native compact large-paste markers remain unchanged and are never expanded
  into the hidden pasted body.
- Pasted input below 1,000 characters remains directly editable; input at or
  above the threshold renders one atomic `[Pasted Content · N chars]` token
  while submission receives the byte-for-byte original text.
- Multiple compact paste tokens preserve ordering, cursor movement, whole-token
  deletion, draft restoration, send-failure restoration, and Unicode counts.
- Optimistic/native reconciliation and duplicate prevention.
- Editor operations, Unicode widths, multiline paste, large paste, cursor
  movement, and draft restoration.
- Reducer state transitions for working, done, interruption, disconnect,
  session replacement, and send failure.
- Status extraction fixtures for supported Codex and Claude layouts.
- Message-role hierarchy remains visible without relying on color alone.
- Visual rows preserve Unicode cell widths, span styles, explicit newlines, and
  full-width prompt backgrounds; row count is identical for geometry and
  rendering, including narrow multiword answers.
- Safe ANSI fixtures cover named, indexed, and RGB colors plus supported text
  modifiers; cursor/OSC/clipboard/title/control sequences are discarded.
- Exact final-answer matching never borrows ANSI styles from commentary, a tool
  result, a neighboring final answer, or the native composer.
- Styled-Markdown fallback fixtures cover paragraphs, headings, lists, code,
  emphasis, and links without claiming native provenance.
- Journal reload reproduces the same styled spans, rejects invalid
  fingerprints/ranges, never contains a hidden pasted body, and keeps no
  reasoning or interaction snapshot.
- Sticky prompt rows for short, long, wrapped, Unicode, image-only, and
  constrained-height histories.
- The next prompt pushes the previous sticky context out one visual row at a
  time without overlap or duplication.

Fixtures are synthetic and contain no real user transcript data.

### Integration tests

- Fake Herdr Unix socket for request ordering, subscriptions, errors, and
  reconnection.
- Fake Herdr ANSI reads for exact final capture, unavailable scrollback,
  mismatch fallback, and blocked-surface refresh.
- Temporary transcript files for initial load, append, partial JSON line,
  truncation, and replacement.
- Pseudo-terminal harness for raw-mode restoration, bracketed paste, resize,
  compact large multiline input with full-payload submission, image-path
  forwarding, and interrupt forwarding.
- Toggle lifecycle: open, focus existing, close, stale-state cleanup, and source
  focus restoration.
- History lifecycle: overlay close retains the current namespace, live source
  `pane_closed` deletes it, startup removes proven-stale pane/session state, and
  temporary socket failure does not delete unverified state.
- Blocked passthrough: text and every supported interaction key are forwarded
  exactly once; unsupported mouse/control input is ignored; leaving `blocked`
  restores the unchanged draft and normal history.

### Verification gates

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
```

A manual smoke matrix then covers local and remote Herdr sessions with current
Codex and Claude versions, including text, large paste, image paste, working
state, interruption, styled final answers, long-answer bottom scrolling,
full-width prompt bands, blocked questions/approvals, overlay toggle, source
pane deletion cleanup, and returning to the unchanged native pane.

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

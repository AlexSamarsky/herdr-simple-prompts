# Herdr Simple Prompts

Herdr Simple Prompts is a terminal overlay for [Herdr](https://herdr.dev) that
keeps an agent conversation intentionally quiet. It shows real user prompts,
final answers, the current `Working` state, a multiline composer, and a compact
status footer. Reasoning, commentary, tool calls, tool results, system context,
and subagent traffic stay in the native agent pane.

Version 0.1 supports Codex CLI and Claude Code on macOS and Linux.

## Source-only trust model

This repository does not publish or download executable plugin binaries. Herdr
clones the public source and runs this visible build command locally:

```bash
cargo build --locked --release
```

`Cargo.lock` fixes the complete dependency graph. The plugin has no HTTP client,
telemetry, analytics, update checker, or runtime network access. Cargo still
needs access to crates.io during the first local build unless the locked crate
sources are already cached.

## Requirements

- Herdr 0.7.5 or newer
- Rust 1.85 or newer with Cargo
- Codex CLI or Claude Code
- The corresponding Herdr native integration, so the plugin can identify the
  exact native session:

```bash
herdr integration install codex
herdr integration install claude
```

## Install from GitHub

After this repository is published, install it with its GitHub owner:

```bash
herdr plugin install <github-owner>/herdr_simple_prompts
```

Herdr displays the manifest and build command in its trust preview before
building. Confirm the plugin is available:

```bash
herdr plugin list --plugin herdr.simple-prompts
```

## Bind the toggle

Add this command binding to `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+m"
type = "plugin_action"
command = "herdr.simple-prompts.toggle"
description = "Toggle Simple Prompts"
```

Reload configuration:

```bash
herdr server reload-config
```

Focus a running Codex or Claude pane and press the Herdr prefix (normally
`ctrl+b`) followed by `m`. The same binding closes the overlay and restores the
unchanged native agent pane.

`prefix+p` is intentionally not used because Herdr assigns it to the previous
tab by default.

## Conversation view

History is laid out once by a Unicode-aware visual-row engine. The same rows
drive rendering, bottom alignment, scrolling, and sticky prompt context, so a
long answer remains reachable instead of being wrapped a second time by the
terminal widget.

- Each user prompt is a full-width neutral-gray block with one gray blank row
  above and below its text. There is no `YOU` label.
- Each final answer begins directly with its styled text on the normal terminal
  surface. There is no `ANSWER` label or answer box.
- After a prompt scrolls out of its natural position, at most its first two
  wrapped content rows stay at the top. The gray top padding stays with them
  when the viewport has room. The next prompt pushes the old block away one row
  at a time. The sticky copy never replaces or truncates the complete prompt in
  ordinary history.
- `PageUp`, `PageDown`, and the mouse wheel scroll the conversation. Returning
  the offset to the bottom resumes live bottom-following.

For a newly observed final answer, Simple Prompts reads recent ANSI output from
the source agent and accepts a styled block only when its sanitized text exactly
matches the canonical transcript answer at a known Codex or Claude boundary.
Only text plus safe SGR colors and bold, dim, italic, and underline attributes
are retained. Cursor movement, alternate-screen commands, OSC title, hyperlink,
clipboard, and other terminal controls are discarded and never replayed.

When exact native ANSI is unavailable, the canonical transcript text is still
shown using the built-in dependency-free Markdown fallback for headings, lists,
inline and fenced code, emphasis, and links. This is fallback presentation; it
is not treated or persisted as captured native styling.

## Composer keys

| Key | Action |
|---|---|
| `Enter` | Submit the prompt |
| `Shift+Enter` | Insert a newline when supported by the terminal |
| `Ctrl+J` | Portable newline fallback |
| `Ctrl+V` | Attach an image through the native agent |
| `Esc` | Interrupt the agent while it is working |
| `PageUp` / `PageDown` | Scroll conversation history |
| Mouse wheel | Scroll conversation history |

Pastes below 1,000 characters remain directly editable. A paste of 1,000
characters or more appears as one atomic `[Pasted Content · N chars]` token in
the composer and prompt history, while Codex or Claude receives the complete
original text with all newlines. Multiple large pastes remain separate; the
cursor skips each token and deletion removes it as a whole. The plugin imposes
no arbitrary prompt-length truncation. Any Herdr or agent-side rejection is
shown and the exact draft, including the hidden source behind compact tokens,
is restored.

## Native questions and approvals

When Herdr reports that the source agent is blocked, the overlay temporarily
shows `INTERACTION REQUIRED` and a refreshed, sanitized view of the native
Codex or Claude question, choice, permission, or approval surface. Conversation
history and the composer are hidden during this mode, but their contents remain
unchanged and return automatically when the agent leaves the blocked state.

Typed text and pasted text are forwarded to the native surface. These native
keys are supported: `Up`, `Down`, `Left`, `Right`, `Tab`, `Shift+Tab`, `Space`,
`Enter`, `Backspace`, `Delete`, and `Esc`. Each accepted input is sent once;
unsupported control keys are ignored. Mouse interaction is not mapped in
version 0.1.

If the native interaction cannot be read, Simple Prompts shows an error instead
of guessing the question or its answer. Press `prefix+m` to close the overlay
and answer directly in the unchanged native pane.

## Images and remote attach

For local sessions, `Ctrl+V` is forwarded to the native Codex or Claude composer.
The overlay records the attachment only after the native pane exposes its image
marker.

During remote attach, Herdr stages a locally pasted image in its private remote
temporary directory and pastes the path. Simple Prompts recognizes only an
existing image file under a `herdr-clipboard-images-*` path and forwards it to
the native agent. Images appear as compact `[Image #N]` placeholders; the plugin
does not render or copy their pixels.

## Privacy and state

Simple Prompts never modifies the native Codex or Claude transcript. It does
keep an intentional private copy of the visible prompt/final-answer subset so
reopening the overlay can reproduce what it previously showed. This is scoped
to one source pane and one native session; it is not a global conversation
database or cross-pane browser.

The Herdr-managed state directory contains:

- the source-to-overlay pane registry;
- the current draft and local attachment placeholders;
- compact-paste display ranges, character counts, and integrity fingerprints;
- the pane/session visible-history journal.

The journal is auditable JSON Lines at:

```text
history/<safe-source-pane-id>/<native-session-id>.jsonl
```

Each versioned record contains only a display-safe user prompt or visible final
answer, its native stable and turn identifiers, sanitized attachment labels,
attachment IDs, timestamp and display order, a text fingerprint, and either
validated native style ranges or fallback/plain presentation provenance. A
repeated stable id is an append-only upsert whose latest valid record wins, so
later exact native ANSI can replace fallback presentation.

The journal never stores reasoning, commentary, tool calls or results, system
context, subagent traffic, blocked interaction surfaces, native attachment
paths, or the hidden body of a large paste. A submitted large paste is copied to
history only as its compact marker. Only an unsent draft may retain the complete
hidden paste so editing and send-failure recovery stay lossless.

State directories use mode `0700`; registry, draft, namespace, and journal files
use mode `0600`. Journal writes are asynchronous, append newline-terminated
records, and ignore an incomplete final line during recovery.

State retention follows the source pane rather than the overlay:

| Event | Result |
|---|---|
| Close only Simple Prompts with `prefix+m` | Keep the pane/session history and draft for reopening |
| Close the native source pane | Delete that pane's registry, draft, compact metadata, and history namespace |
| Reuse a pane for a different native session | Delete the replaced session's saved state during validation |
| Source cannot be verified temporarily | Keep its state; after seven continuously unverifiable days, remove it on the next plugin invocation |

Before every prompt, interrupt, image mutation, or blocked-input forwarding, the
plugin verifies that the source pane still contains the original agent kind and
native session id. No detached cleanup watcher or resident plugin daemon is
created.

## Local development

```bash
git clone https://github.com/<github-owner>/herdr_simple_prompts
cd herdr_simple_prompts
cargo test --all-targets --all-features
cargo build --locked --release
herdr plugin link .
```

Edits remain in the local checkout. Rebuild and relink as needed; unlinking does
not delete source files:

```bash
herdr plugin unlink herdr.simple-prompts
```

## Verification

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
```

CI runs the source build on macOS and Linux and uploads no executable artifact.

## Manual Codex and Claude smoke test

Build and link the current source, then install both native integrations:

```bash
cargo build --locked --release
herdr plugin link .
herdr integration install codex
herdr integration install claude
herdr server reload-config
```

For Codex, focus a current native Codex pane and press `prefix+m`. Verify this
sequence with synthetic, non-sensitive input:

1. Submit a normal prompt and confirm it appears above the native `Working` row
   as a full-width gray block with one blank gray row above and below, without
   `YOU` or `ANSWER` labels.
2. Request a long answer containing a heading, list, emphasis, inline code, and
   fenced code. Confirm the bottom is reachable, `PageUp`/`PageDown` and the
   mouse wheel scroll, and the first two prompt rows stick and are pushed away
   by the following prompt.
3. Paste 1,000 or more characters. Confirm the composer and saved prompt show
   only the compact marker while the native agent receives the complete text.
4. Use a workflow that asks a question or permission. Confirm
   `INTERACTION REQUIRED`, exercise the supported keys, and confirm the
   unchanged draft returns afterward.
5. Close and reopen only the overlay with `prefix+m`; confirm styled visible
   history returns. Then close the native source pane and confirm its private
   pane state is removed.

Repeat steps 1, 2, 4, and 5 in a current Claude Code pane. Include one
tool-using prompt and confirm thinking, tool use/results, and progress remain in
the native pane while only the final visible answer appears in Simple Prompts.
If no live Claude session is available, report that prerequisite as missing;
the automated Claude fixtures do not constitute a live smoke test.

## Troubleshooting

### Native session is unavailable

Install the corresponding Herdr integration and restart the agent pane:

```bash
herdr integration install codex
# or
herdr integration install claude
```

### An image is not attached

Open the native agent pane and confirm its own image paste works first. Simple
Prompts deliberately reports failure when the native attachment marker cannot be
verified.

### Status fields are missing

Model and usage parsing is conservative. When a new agent version changes its
footer, Simple Prompts omits unproven fields instead of inventing values. The
agent kind and known working directory remain visible.

### Uninstall

```bash
herdr plugin uninstall herdr.simple-prompts
```

## Limitations

- Only Codex and Claude are supported in version 0.1.
- Windows is not supported.
- The view belongs to the currently focused native session; it is not a global
  conversation browser.
- Image pixels are not rendered inside the overlay.
- Agent footer extraction is best-effort and intentionally conservative.

## Publishing to the Herdr marketplace

Make the GitHub repository public and add the `herdr-plugin` repository topic.
Herdr's marketplace discovers public repositories with that topic automatically.

## License

MIT

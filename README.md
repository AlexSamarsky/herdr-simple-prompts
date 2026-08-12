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

Normal and large bracketed-paste input is inserted atomically without a
plugin-defined text limit. Any Herdr or agent-side rejection is shown and the
exact draft is restored.

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

Simple Prompts reads the current native JSONL transcript but never modifies or
copies it into another conversation database. Its Herdr-managed state directory
contains only:

- the source-to-overlay pane registry;
- the current draft;
- local attachment placeholders.

State files use user-only permissions. Before every prompt, interrupt, or image
mutation, the plugin verifies that the source pane still contains the original
agent kind and native session id.

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

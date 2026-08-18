# Working in this repository

Read this first, then open only the document the task needs. The README is for
people installing the plugin; it is not the specification.

## Where things are

| Path | What it holds |
|---|---|
| `src/agent/` | Transcript adapters: `claude.rs`, `codex.rs`, the shared `follower.rs` tail reader, session resolution |
| `src/ui/` | Visual-row layout, rendering, runtime loop, terminal setup |
| `src/editor.rs`, `src/composer.rs`, `src/paste.rs` | Draft editing, submission, compact large pastes |
| `src/history.rs`, `src/state.rs` | Journal records and in-memory conversation state |
| `src/herdr/` | Herdr client and protocol types |
| `tests/`, `tests/fixtures/` | Integration tests and transcript fixtures |
| `docs/behavior.md` | The behavior contract - shown, hidden, forwarded, stored |
| `docs/development.md` | Verification gates, manual smoke test, publishing |
| `docs/troubleshooting.md` | Operator-facing failure modes |
| `docs/superpowers/` | Historical plans and design specs, newest wins |

## Invariants

- The native transcript is read-only. Never write to it, never copy native draft
  text or attachment paths into plugin state, logs, or errors.
- Hidden content stays hidden: reasoning, tool calls and results, system
  context, subagent traffic, and large-paste bodies never enter the journal.
- Prefer omission to invention. When a footer, style capture, or interaction
  surface cannot be parsed with certainty, show nothing rather than a guess.
- No network access, no telemetry, no runtime downloads. Dependencies change
  only through `Cargo.toml` and `Cargo.lock` together.
- Transcript formats are discovered from real transcripts, not assumed. A parser
  change lands with a fixture in `tests/fixtures/` that captures the real shape.

## Before pushing

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
```

CI runs the same gates on macOS and Linux against Rust 1.88.0 and stable.

## Documentation rules

- Human-facing text goes in `README.md` and stays short: install, everyday use,
  privacy in a paragraph, links out.
- Detail goes to `docs/`. Extend the existing file instead of growing the README
  back; a behavior change updates `docs/behavior.md` in the same commit.
- Commit messages state what changed and why in plain prose, imperative first
  line, no ticket prefixes.

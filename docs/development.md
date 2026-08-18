# Development

## Build and link

```bash
git clone https://github.com/AlexSamarsky/herdr-simple-prompts.git
cd herdr-simple-prompts
cargo test --all-targets --all-features
cargo build --locked --release
herdr plugin link .
```

Edits stay in the local checkout; rebuild and relink as needed. Unlinking
deletes no source files:

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

CI runs the same gates plus `bash tests/register-existing-sessions.sh` on macOS
and Linux, against Rust 1.88.0 and stable, and uploads no executable artifact.

## Manual smoke test

Automated fixtures do not replace a live session. Build, link, and install both
integrations first:

```bash
cargo build --locked --release
herdr plugin link .
herdr integration install codex
herdr integration install claude
herdr server reload-config
```

Focus a live Codex pane, press `prefix+m`, and walk this sequence with
synthetic, non-sensitive input:

1. Submit a normal prompt. It appears above the native `Working` row as a
   full-width gray block with the local `DD.MM.YYYY HH:MM` in its top gray row
   and one blank gray row below, with no `YOU` or `ANSWER` label.
2. Ask for a long answer containing a heading, bold and emphasized text, inline
   code, fenced code, and a Markdown link. Against the native pane, confirm:
   one undimmed gray timestamp row directly above the answer, no box or fill;
   the same visible words in the same order; no delimiters and no visible link
   destination; native colors and emphasis when exact capture succeeds; the
   label opens its HTTP(S) destination in an OSC 8 terminal; the last line is
   reachable by scrolling. Also confirm a `mailto:` fixture stays a plain label,
   `PageUp`/`PageDown` scroll, drag-and-release selection copies, and the first
   two prompt rows stick until the next prompt pushes them away.
3. Paste 1,000+ characters. The composer and saved prompt show only the compact
   marker while the agent receives the complete text.
4. Trigger a question or permission prompt. Confirm `INTERACTION REQUIRED`,
   exercise the supported keys, and confirm the draft returns unchanged.
5. Close and reopen only Simple Prompts with `prefix+m`: the exact text and
   styles from step 2 are restored. Then close the native source pane and
   confirm its private state is removed.
6. Type synthetic text in the native composer without submitting, open Simple
   Prompts, and confirm editing is blocked with both drafts intact. Clear the
   native draft; one subsequent submission produces exactly one prompt.
7. Remove an open Simple Prompts pane, then press `prefix+m` from that stale
   action context: one invocation targets the still-live source and opens a
   replacement view on the original source tab.

Repeat steps 1, 2, 4, 5, and 6 in a live Claude Code pane, including one
tool-using prompt: thinking, tool use and results, and progress stay in the
native pane while only the final answer appears here. Also queue a prompt while
Claude is working and confirm it appears in the view where it was queued. If no
live Claude session is available, report that prerequisite as missing.

## Publishing to the Herdr marketplace

Make the GitHub repository public and add the `herdr-plugin` repository topic.
Herdr's marketplace discovers public repositories with that topic automatically.

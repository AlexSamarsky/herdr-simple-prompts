# Width-Independent Native ANSI Capture Design

## Problem

Simple Prompts currently preserves Codex and Claude styling only when the
Markdown-projected transcript text has the same physical line layout as the
agent pane and a pure separator appears immediately before the answer. A final
answer can therefore be stored as `fallback` even though its native ANSI output
is still available. This happens when the source pane wraps text at a different
width or emits the final role row without a leading separator. Fallback Markdown
then renders fenced shell code differently from the source agent.

## Approved behavior

The transcript remains the canonical answer text. Native ANSI remains the
authoritative source for colors and modifiers. Physical terminal wrapping is
presentation-only and must not participate in answer identity.

Capture accepts a native final-answer candidate only when all of the following
hold:

- it begins at the provider's known final role prefix;
- it ends at a known final boundary followed by the provider composer;
- after removing provider chrome and whitespace, its Unicode scalar sequence is
  exactly equal to the Markdown-projected transcript sequence;
- exactly one candidate in the bounded recent output satisfies that equality;
- the content after the composer contains only known provider footer rows.

The existing strict, line-for-line matcher remains the first path. The new
width-independent matcher runs only when strict matching finds no candidate.
More than one strict or width-independent candidate remains an unsafe result and
falls back to deterministic Markdown.

## Style projection

The width-independent matcher aligns identical non-whitespace Unicode scalar
values from the sanitized native candidate to the projected transcript. Each
aligned native scalar transfers its foreground, background, and modifiers to
the corresponding transcript byte range. Transcript whitespace and logical
newlines remain unchanged and unstyled unless the strict matcher already
captured them exactly.

This gives Simple Prompts stable, reflowable text while retaining the meaningful
native styling of shell commands, links, emphasis, and headings. It deliberately
does not store the source pane's physical wrapping and does not implement a
second syntax highlighter.

## Native prompt-band tone

The user-prompt band must match the reviewed Codex prompt surface rather than
the terminal's bright `DarkGray` palette entry. Pixel sampling of two empty
points in the supplied Codex reference gives RGB `52, 53, 54` (`#343536`); the
current Simple Prompts band resolves to RGB `102, 102, 102` (`#666666`).

Simple Prompts therefore uses `AnsiColor::Rgb(52, 53, 54)` as the prompt fill
background while retaining the current bright foreground, full-width fill,
one-row top and bottom padding, wrapping, and sticky behavior. This is an exact
parity correction for the approved Herdr/Codex surface, not a new theme system.

## Boundaries and safety

The capture implementation stays in `src/ansi.rs`, and the prompt-tone change
stays in the existing `prompt_fill` owner in `src/ui/visual_rows.rs`; the
runtime, history schema, and renderer contracts do not change. ANSI is
sanitized before matching, and no escape sequence is replayed. Candidate
discovery uses only the existing provider-specific role, continuation,
trailing-boundary, composer, and footer contracts.

Whitespace-insensitive matching is intentionally narrow: only whitespace may
differ. Punctuation, letters, digits, symbols, and their order must match
exactly. Missing content, changed commands, partial answers, ambiguous duplicate
answers, unknown chrome, or unexpected footer content continue to produce
`MarkdownFallback`.

## Tests

Regression tests in `tests/ansi_style.rs` cover:

1. A Codex final answer with no leading separator and physical soft wraps is
   captured as native ANSI.
2. A wrapped fenced shell body maps native command colors onto the canonical
   logical lines without storing terminal wraps.
3. A non-whitespace mismatch is rejected.
4. Duplicate normalized candidates are rejected as ambiguous.
5. Multibyte Unicode scalars keep valid UTF-8 style ranges.
6. Provider footers require a reviewed model label plus an absolute or
   home-relative working-directory field; prefix-only lookalikes are rejected.
7. Existing strict Codex and Claude captures remain unchanged.

The rendered-buffer regression in `tests/ui_render.rs` additionally asserts
that every cell in every wrapped prompt-band row uses
`Color::Rgb(52, 53, 54)`.

Focused verification runs `cargo test --test ansi_style` plus the prompt-band
render regression in `cargo test --test ui_render`. Full verification runs
`cargo test --all-targets` and
`cargo clippy --all-targets --all-features -- -D warnings`, followed by a release
build of the source-only plugin.

## Out of scope

- A general Markdown or shell syntax-highlighting dependency.
- Reconstructing native styles for final answers that have already been stored
  as fallback and are no longer present in source scrollback.
- Relaxing equality for non-whitespace edits.
- Changing prompt-band geometry, scrolling, composer behavior, or blocked
  interaction forwarding.

# Local File Links Design

## Context

Codex renders Markdown links to local files as the absolute path, with native
link styling. Simple Prompts currently accepts only `http` and `https` targets.
For a canonical answer such as `[TDD](/Users/example/SKILL.md)`, its Markdown
projection therefore becomes only `TDD`. That loses the target and prevents the
exact native capture from matching Codex's visible `/Users/example/SKILL.md`
text, so both the link and native styling disappear.

## Approved behavior

- A Markdown link whose destination is a safe absolute POSIX path is projected
  as that full destination path, matching the visible Codex output.
- The projected path is styled as a link and receives an OSC 8 target in the
  form `file:///absolute/path`.
- Existing `http` and `https` links keep their label-based display and current
  target unchanged.
- Relative paths, `mailto:`, editor schemes, remote `file://host/path` targets,
  double-leading-slash paths, empty paths, and targets containing whitespace or
  control characters remain non-clickable and do not reach OSC 8 output.
- Local paths are not resolved, canonicalized, opened, or checked for existence.
  Clicking remains an explicit terminal/user action, and historical links keep
  working even if the file is temporarily absent.
- Unicode path characters are preserved. Paths containing literal whitespace
  remain outside this change because the existing Markdown candidate grammar
  rejects whitespace in link destinations.

## Architecture

The Markdown projection classifies each valid link destination as HTTP, safe
absolute path, or non-clickable. HTTP links continue to project the label. A
safe absolute path instead projects the destination bytes and records a
`file://` hyperlink target over that projected range.

The renderer performs a second, independent allow-list check before emitting
OSC 8. It accepts only the existing safe HTTP URLs and local `file:///...` URLs
created by the projection. This preserves defense in depth against persisted or
captured hyperlink metadata.

Because the projected text now matches Codex's visible local path, the existing
exact native-capture path can retain Codex's ANSI styling while the Markdown
metadata supplies the clickable target.

## Verification

Regression coverage will prove that:

1. `[TDD](/Users/example/SKILL.md)` projects to
   `/Users/example/SKILL.md`, styled cyan and underlined;
2. its hyperlink metadata targets `file:///Users/example/SKILL.md`;
3. exact native capture succeeds when Codex visibly renders the absolute path;
4. wrapped Unicode local paths retain one balanced OSC 8 target;
5. HTTP behavior is unchanged;
6. relative, remote-file, double-slash, whitespace, and control-bearing targets
   cannot produce clickable metadata or OSC 8 sequences.

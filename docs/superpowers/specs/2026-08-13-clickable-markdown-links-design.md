# Clickable Markdown Links

## Context

Simple Prompts currently projects `[label](destination)` to a cyan, underlined
label and removes the destination. The label looks like a link, but it is not
clickable because Ratatui styles do not carry a hyperlink target.

## Selected behavior

- Markdown destinations beginning with `http://` or `https://` are rendered as
  OSC 8 terminal hyperlinks.
- The visible content remains only the Markdown label. The destination is not
  printed beside it.
- Safe links keep the existing cyan, underlined presentation.
- Destinations with any other scheme are projected to their label as ordinary
  answer text without underline or hyperlink behavior.
- A destination containing whitespace or terminal control characters is not a
  valid clickable target.

## Architecture

Markdown projection returns the existing `StyledText` plus ephemeral hyperlink
ranges measured in projected UTF-8 byte offsets. The persisted message and
history-journal schemas remain unchanged.

The visual-row projection carries an optional URL on each link span and keeps
it while splitting lines and wrapping Unicode text. Native ANSI answers reuse
the Markdown hyperlink ranges only when the Markdown-projected visible text is
byte-identical to the captured native visible text; native colors and emphasis
remain authoritative.

Ratatui first draws an ordinary frame. The terminal path then redraws only the
linked spans through the backend with an OSC 8 open sequence, visible label,
and OSC 8 close sequence. This avoids storing escape sequences in Ratatui's
layout buffer, so width, scrolling, and neighboring cells continue to use the
plain label width. The input cursor is restored after the hyperlink redraw.

## Security and failure behavior

Only control-free `http://` and `https://` targets enter an OSC sequence. The
renderer never treats persisted ANSI/control bytes as link metadata and never
persists URLs separately. Unsupported or unsafe destinations degrade to plain
labels, so a malformed link cannot inject terminal commands or leave an OSC 8
link open across following text.

## Testing

- Safe HTTP(S) links retain only the label, carry their exact target, and keep
  cyan underline styling.
- Unsupported schemes and control-bearing destinations produce ordinary,
  non-clickable labels.
- Hyperlink metadata survives line wrapping and Unicode labels.
- Native ANSI presentations receive hyperlinks only after the exact visible
  text match and retain their native styles.
- The terminal backend receives balanced OSC 8 sequences and restores the
  composer cursor.
- Existing Markdown, history, scrolling, and renderer tests remain green.

## Non-goals

- No mouse-event handling inside Simple Prompts; opening is delegated to the
  terminal emulator.
- No automatic linking of bare URLs.
- No support for `file:`, `mailto:`, editor-specific, or custom schemes.
- No history migration or dependency addition.

# Global Horizontal Gutter Design

## Goal

Give every Simple Prompts surface one empty terminal cell on the left and one
on the right so history, prompt bands, status text, the composer, footer, and
blocked interaction UI do not visually touch the pane edges.

## Layout contract

The renderer derives one shared content rectangle from the terminal frame:

- the content rectangle starts one cell to the right of the frame;
- its width is the frame width minus the two horizontal gutter cells;
- its vertical position and height are unchanged;
- the two outer columns remain unpainted by Simple Prompts;
- widths below three cells degrade safely to an empty content rectangle.

Both the ordinary and blocked render paths use this same content rectangle.
All vertical layout, wrapping, scrolling, hyperlink coordinates, and cursor
placement continue to derive from the rectangles passed to their existing
owners. No message text is prefixed with spaces and no widget receives an
independent ad hoc padding value.

## Visible behavior

- Prompt gray bands fill the content rectangle but stop before both gutters.
- Agent answers, timestamps, errors, `Working`, attachments, composer text,
  composer rule, footer, blocked header/body/error/footer, and the cursor align
  to the same one-cell inset.
- Text wraps two cells earlier than it does today because the content width is
  reduced by two.
- OSC 8 hyperlinks remain clickable only over their visible text at the new
  coordinates.
- Vertical spacing, sticky prompt behavior, scrolling semantics, colors,
  message formatting, input behavior, and pane lifecycle remain unchanged.

## Implementation boundary

Add one small renderer helper that safely insets a `Rect` horizontally. Apply
it once at the start of the ordinary render path and once at the start of the
blocked render path. Existing child layout and coordinate calculations then
consume the inset rectangle without further special cases.

## Verification

Renderer tests must prove that:

1. ordinary history, prompt background, status, composer, footer, and cursor
   leave both outer columns untouched;
2. blocked interaction UI follows the same gutter contract;
3. prompt wrapping uses the reduced content width;
4. narrow terminal widths do not panic or produce invalid coordinates;
5. the existing renderer and full project test suites remain green.

The plugin remains source-only and gains no dependency or configuration
change.

# Prompt Background and Date Alignment Design

## Context

The global one-cell horizontal gutter correctly separates ordinary interface
text from the terminal edges, but applying the inset to the complete history
rectangle also clips the gray background of user prompts. User prompt
timestamps currently begin at the same horizontal position as prompt text.

## Approved behavior

- Keep the one-cell horizontal text gutter for history, working state,
  composer, footer, errors, and blocked interaction content.
- Extend the gray background of every visible user-prompt row through both
  terminal edge cells. This applies to the timestamp row, prompt body rows,
  bottom padding row, wrapped rows, and sticky prompt copies.
- Keep user-prompt body text at column `1`.
- Start a present user-prompt timestamp at column `3`, two cells to the right
  of the prompt body text.
- Start a present answer timestamp at column `3` as well.
- Do not change answer text, wrapping width, hyperlink coordinates, scrolling,
  composer cursor placement, or blocked mode.
- For terminal widths below three cells, preserve the existing safe empty
  rendering behavior.

## Implementation shape

History continues to render inside the shared one-cell content rectangle so
text geometry and wrapping remain unchanged. After the visible history rows
are selected, the renderer paints only the two outer edge cells for rows whose
`VisualRow::fill` is present, using that row's fill style. The history document
adds a two-cell prefix to both user and answer timestamp rows; clipping accounts
for the prefix so each timestamp remains a single visual row.

This keeps background geometry in the renderer and message content geometry in
the history-document builder without introducing widget-local padding.

## Verification

Regression coverage will prove that:

1. all user-prompt background rows reach both terminal edges;
2. prompt text still starts at column `1`;
3. a user timestamp starts at column `3`;
4. an answer timestamp starts at column `3`, while answer text and non-prompt
   UI retain clear edge gutters;
5. sticky and wrapped prompt rows preserve the same full-width background;
6. widths `1` and `2` remain safe and unpainted.

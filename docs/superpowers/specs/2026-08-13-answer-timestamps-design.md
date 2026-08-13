# Final Answer Timestamps

## Context

Simple Prompts renders a local `DD.MM.YYYY HH:MM` timestamp in the existing top
gray row of every user prompt. Final answers retain their own `timestamp_ms` in
the normalized message and history journal, but the history renderer currently
starts directly with the answer body, so that time is no longer visible.

## Selected presentation

Render one subdued metadata row immediately above every final answer:

```text
13.08.2026 19:32
Final answer body
```

The row has no background fill, border, role label, or icon. It uses the same
`DD.MM.YYYY HH:MM` local-time formatter as prompt timestamps and therefore
honors the local offset that applied at the answer instant, including DST.

## Rendering contract

- Read only `final_answer.timestamp_ms`; never derive answer time from the
  prompt, journal file metadata, render time, or working duration.
- Add the metadata row only when the timestamp exists and converts successfully.
  Missing or invalid legacy timestamps do not create an empty gap.
- Clip the timestamp to the available terminal width so it always occupies at
  most one visual row.
- Use a subdued `BrightBlack`/dim foreground on the ordinary terminal surface.
- Keep the answer's existing native ANSI or Markdown fallback body unchanged.
- Keep prompt bands, sticky prompt behavior, bottom-following, and scrolling
  semantics unchanged apart from the intentional extra answer timestamp row.

## Persistence and privacy

No state or journal migration is needed. Final-answer timestamps are already
persisted in `VisibleHistoryRecord.timestamp_ms`. The renderer only displays
that existing value and introduces no new stored data.

## Testing

- A fixed answer epoch at `+03:00` renders `13.08.2026 19:32` above the answer.
- The metadata style is dim/unboxed and the answer body starts on the next row.
- A narrow viewport clips the metadata within one row.
- Missing and invalid answer timestamps add no blank row.
- Native ANSI and Markdown fallback answer body text/styles remain unchanged.
- Hydrated history uses the final answer's persisted timestamp.
- Existing scrolling and last-answer-row reachability tests remain green.

## Non-goals

- Do not place the timestamp inline with answer text.
- Do not add timestamps to working/status, blocked interaction, or tool output.
- Do not change transcript parsing or answer identity.

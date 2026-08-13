# Composer Safety, Stale Overlay Recovery, and Prompt Timestamps

## Context

Three related usability failures need one bounded correction:

1. Simple Prompts owns a private draft, while Codex and Claude also own a native
   composer. Herdr 0.7.5 `agent.prompt` inserts text into the current native
   composer and sends Enter after 300 ms; it does not replace or inspect an
   existing draft. If the native composer already contains text, submitting the
   apparently empty Simple Prompts composer concatenates both drafts.
2. A plugin overlay can disappear while its source-to-overlay registry entry and
   the keybinding invocation context still name the removed pane. The current
   close path treats `pane_not_found` as fatal, so repeated `prefix+m` calls fail
   instead of recovering the live source pane.
3. User prompt blocks already reserve a gray top-padding row, but the row carries
   no timestamp even though native, optimistic, and persisted messages already
   retain `timestamp_ms`.

The registration command that selects Codex agents with
`agent_session == null` is not a remedy for the second problem. It correctly
prints nothing when every live Codex agent already has session metadata.

## Goals

- Never intentionally append a Simple Prompts submission to a detected native
  text draft.
- Preserve both the native draft and the complete Simple Prompts draft when a
  conflict or inconclusive preflight is found.
- Let one `prefix+m` invocation recover a stale overlay context and open a fresh
  overlay for the still-live source pane.
- Show `DD.MM.YYYY HH:MM` in local system time at the left edge of the existing
  top gray row of every user prompt block.
- Preserve the existing block height, scrolling model, sticky behavior, privacy
  boundary, and source-only distribution model.
- Support Codex and Claude without guessing about unknown terminal layouts.

## Non-goals

- Do not import, copy, clear, or edit the native Codex or Claude draft.
- Do not attempt to reconstruct native draft text from terminal pixels.
- Do not change final-answer rendering or add timestamps to agent answers.
- Do not modify Herdr itself or rely on a new Herdr socket method.
- Do not add a resident daemon or a second source of conversation history.

## Approaches considered

### 1. Plugin-side conservative preflight (selected)

Read the native composer presentation, classify it conservatively, expose a
conflict before editing when possible, and repeat the check immediately before
`agent.prompt`. Recover stale overlay registry/context inside the existing
toggle action. Render prompt timestamps from the timestamp already owned by the
message.

This ships with the plugin and never mutates an unowned draft. Its limitation is
that Herdr 0.7.5 offers no atomic "submit only if composer is empty" operation;
another client could theoretically type between the final read and
`agent.prompt`. The preflight closes the reported single-client failure and
fails closed whenever the source cannot be proved safe.

### 2. New atomic Herdr API

Add a Herdr method such as `agent.prompt_if_composer_empty`. This would close the
cross-client race but is an upstream Herdr change, cannot fix the installed
plugin alone, and still requires agent-specific composer awareness in Herdr.

### 3. Check only when opening the overlay

This is smaller but unsafe because the native composer can change after the
overlay opens. It also gives no protection at the actual submit boundary.

## Native composer safety model

Introduce an agent-specific, presentation-only classifier with four outcomes:

- `Clear`: a known empty Codex or Claude composer is present;
- `OwnedAttachments(n)`: the composer contains no text and exactly `n` native
  attachment markers;
- `Occupied`: user-authored native text or unowned attachments are present;
- `Unknown`: the expected composer boundary cannot be proved from the snapshot.

Despite the four display outcomes, submission is binary. It is allowed only
when the classifier returns `Clear` and the plugin has no confirmed attachments,
or `OwnedAttachments(n)` and `n` exactly equals the plugin's confirmed draft
attachment count. `Occupied`, `Unknown`, a missing expected marker, an extra
marker, and a read error all reject submission.

The classifier operates only inside a recognized bottom composer boundary. It
must not count image markers from conversation history or treat status/footer
text as a draft. Codex and Claude rules remain separate and fixture-driven.
Unknown provider layouts fail closed instead of being interpreted as empty.

The periodic source observer carries the latest classification into `AppState`.
When it is unsafe, the ordinary composer is replaced by one of these messages:

- `Native composer contains unsent input · prefix+m to return`
- `Unable to verify native composer · prefix+m to return`

The plugin-owned editor snapshot and attachments remain untouched on disk and
in memory. When a later observation proves the source safe, editing is enabled
again.

The action worker repeats the classification immediately before submission and
passes the expected confirmed-attachment count into the transport. The existing
optimistic submission path may clear the visible editor before the asynchronous
result arrives; a rejected preflight uses the existing `SendFailed` recovery to
restore the exact editor snapshot, large-paste chunks, and attachments. The
transport must not call `agent.prompt` on a rejected preflight.

Blocked questions and approvals keep their current routing. They bypass the
ordinary composer and therefore do not use this preflight.

## Stale overlay recovery

Toggle routing distinguishes a live overlay from a stale overlay identifier:

1. Reverse-lookup the action context in the plugin registry.
2. If the context names a live overlay, close it, focus its source, and remove
   the registry entry as today.
3. If the context names a missing overlay, remove the stale mapping, verify the
   source pane and its native session, explicitly focus that source because
   Herdr's `plugin.pane.open` is active-pane scoped, and continue the same
   invocation as a source-side open.
4. Open and persist a new overlay for that now-active source. If the source is
   also gone, remove only its scoped plugin state and return a precise error.

A stale overlay mapping discovered from the live source side continues to use
the existing replacement behavior. Recovery never closes or mutates unrelated
panes.

The README troubleshooting section will state that `agent_session == null` is a
diagnostic for missing native integration metadata, not a general hotkey
activation mechanism. For failed keybindings it will point to the plugin action
log, where `pane_not_found` identifies stale overlay context.

## Prompt timestamp presentation

`Message.timestamp_ms` remains the canonical value. Native Codex and Claude
messages already populate it from their transcript records. Optimistic prompts
already use submission time, and the history journal already persists the
timestamp.

Render a dim timestamp in the existing top padding row of each prompt block:

```text
13.08.2026 19:32
prompt text

```

The entire row retains the prompt gray fill. The timestamp begins in column zero
and uses a subdued foreground; it has no label or icon. It consumes the existing
padding row, so prompt height and answer position do not grow. When sticky
context has room for top padding, the timestamp naturally stays with the first
two prompt content rows; constrained sticky views continue to prioritize prompt
content.

Formatting uses the machine's local timezone and the exact
`DD.MM.YYYY HH:MM` shape. Add `chrono` as a source dependency with only the
features required for local clock conversion. No network call or runtime update
is introduced. If a message has no timestamp or conversion fails, keep the row
blank rather than inventing a value.

## Persistence and privacy

No state format migration is required. The timestamp is already present in
versioned history records, and composer classification is ephemeral. Native
draft contents are never copied to plugin state, logs, errors, or history.

The new error messages describe only the state class. They never include the
native draft text, attachment paths, or session identifier.

## Testing

### Composer safety

- Codex and Claude fixtures classify known empty composers as clear.
- Single-line, multiline, pasted, and unowned image-only native drafts are
  occupied.
- Conversation-history text and image markers are ignored outside the composer.
- Exact plugin-owned attachment counts are accepted; missing or extra markers
  are rejected.
- Unknown/truncated layouts fail closed.
- Submit preflight rejection makes zero `agent.prompt` calls and restores the
  exact plugin editor snapshot and attachments.
- Blocked interaction forwarding remains unchanged.

### Toggle recovery

- A live overlay still closes and refocuses its source.
- A stale overlay discovered from a live source is replaced.
- A stale overlay used as the action context recovers the source and opens a
  fresh overlay in the same invocation.
- A missing overlay plus missing source performs only scoped cleanup and returns
  a precise failure.

### Timestamps

- A fixed epoch plus fixed offset formats exactly as `13.08.2026 19:32`.
- Native, optimistic, and hydrated prompts use their owned timestamps.
- Missing timestamps retain a blank top row.
- Timestamp rows retain the full gray fill at wide and narrow widths.
- Prompt block row count, answer placement, bottom alignment, scrolling, and
  constrained sticky-content priority remain unchanged.

### Verification

Run formatting, Clippy with warnings denied, all targets/features tests, and a
locked release build. Rebuild the linked source plugin and exercise these live
smoke cases:

1. Type but do not submit text in native Codex, open Simple Prompts, and confirm
   the plugin blocks editing without altering native text.
2. Clear the native draft, reopen, and submit once; confirm one user message.
3. Repeat the conflict flow with Claude when a live Claude session is available.
4. Reproduce a stale overlay context and confirm one `prefix+m` opens a fresh
   overlay.
5. Confirm old and new user prompts show local date/time in their existing top
   gray row.

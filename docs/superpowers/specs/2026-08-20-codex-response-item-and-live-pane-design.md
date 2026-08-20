# Codex response-item and live-pane design

## Problem

Current Codex sessions write visible conversation messages as
`response_item` records whose payload is a `message`. Simple Prompts only
recognizes the older `event_msg` message records, so a correctly registered
current session resolves to its transcript but shows no conversation.

The preceding agent-not-found design also made an incorrect lifecycle
assumption: `agent_not_found` proves only that Herdr cannot currently resolve an
agent in the pane. It does not prove that the pane itself is gone. Removing the
overlay mapping on that error can leave an open Simple Prompts pane that
`prefix+m` can no longer identify or close.

This design supersedes the missing-source decisions in
`2026-08-20-agent-not-found-design.md`. The exact Herdr error classifier remains
useful, but pane existence is the authority for destructive cleanup.

## Decision

### Codex transcript compatibility

Extend `CodexAdapter` to recognize both supported message layouts:

- legacy `event_msg` records keep their existing behavior;
- `response_item` payloads with `type = "message"` and `role = "user"`
  produce a user message from `input_text` content items;
- `response_item` payloads with `type = "message"`, `role = "assistant"`, and
  `phase = "final_answer"` produce a final answer from `output_text` content
  items.

Use the native payload identifier as the stable message identifier and the
record timestamp as the visible timestamp. Concatenate multiple matching text
items in their native order. Omit an event when no supported, non-empty text is
present.

Continue to ignore developer messages, assistant commentary, reasoning, tool
calls and results, subagent records, and unknown content item types. Do not
infer attachments from unverified response-item shapes.

The follower already scans a transcript from the beginning when an overlay
opens and then tails appended records. Supporting the new records at the
adapter boundary therefore restores the complete visible history and keeps
following new messages without a runtime or journal format change.

### Pane and agent lifecycle

Treat pane existence and agent detection as separate facts:

- `pane_not_found` from a pane lookup confirms that the source is gone and
  permits source-state cleanup;
- `agent_not_found` while the source pane still exists is transient for
  lifecycle purposes and must preserve the namespace and overlay mapping;
- opening a new overlay still requires a verified agent identity and native
  session identifier, so Simple Prompts remains fail-closed on non-agent panes;
- an existing overlay can close and focus its living source pane without
  re-resolving the agent.

Saved-namespace validation and the UI lifecycle worker will confirm source-pane
existence before destructive cleanup. Stale-overlay recovery may remove a
missing overlay mapping, but it must not describe or erase a living source as a
missing pane merely because agent detection is unavailable.

## Alternatives considered

1. Normalize new records into synthetic legacy records before parsing. This
   adds an unnecessary translation layer and makes native identifiers and
   content filtering less explicit.
2. Read the visible terminal surface instead of the native transcript. This
   cannot restore scrollback-independent history and would couple conversation
   semantics to terminal rendering.
3. Retry `agent_not_found` for a fixed interval before cleanup. A timing guess
   still conflates agent detection with pane lifetime; a direct pane lookup is
   the authoritative check.

## Verification

Add a privacy-safe Codex fixture that mirrors the observed response-item
structure without copying private conversation text. Test that it restores all
user messages and final answers in order while filtering commentary, developer
context, reasoning, tools, and unknown content.

Add a follower regression proving that existing response-item history is read
from the beginning and later appended messages are delivered. Update lifecycle
and namespace tests to prove that `agent_not_found` plus a live pane preserves
state, while confirmed `pane_not_found` still removes it. Add a toggle
regression proving that an overlay mapping survives validation long enough for
an overlay-context `prefix+m` action to close it.

Run the focused parser, follower, state, toggle, and runtime tests, then all
format, lint, test, and locked release-build gates from `docs/development.md`.

# Agent-not-found handling design

## Problem

Herdr 0.7.5 returns `agent_not_found` when `agent.get` targets a pane that no
longer contains a live agent. Simple Prompts currently recognizes only
`pane_not_found`, so stale source namespaces are retained and lifecycle workers
continue polling a source that is already gone.

The observed Codex transcript format still matches the adapter. This change is
therefore limited to Herdr API error classification and the cleanup paths that
call `agent.get`.

## Decision

Add an exact `HerdrError::is_agent_not_found` classifier. Keep
`is_pane_not_found` unchanged because `pane.get` and `agent.get` have distinct
API contracts.

Treat either `pane_not_found` or `agent_not_found` as a terminal missing-source
result only in paths that call `agent.get`:

- saved namespace validation removes the stale source state;
- stale overlay recovery removes the stale mapping;
- the UI lifecycle worker reports the source pane closed and exits.

Ordinary `agent_identity` lookup remains fail-closed. Opening Simple Prompts
from a non-agent pane or a pane without native session metadata must still
return an error instead of guessing a source.

## Alternatives considered

1. Broaden `is_pane_not_found` to match both codes. Rejected because it would
   erase the distinction between pane and agent APIs and could hide an invalid
   response at `pane.get` call sites.
2. Match the error message text. Rejected because messages are not stable API
   identifiers.
3. Ignore the error and retain state until the seven-day orphan timeout.
   Rejected because the server has already proved that the agent target is
   absent.

## Verification

Use test-first coverage for the exact `agent_not_found` code at the Herdr error
classifier, stale namespace cleanup, stale overlay recovery, and lifecycle
source-loss boundary. Existing tests must continue proving that unrelated API
errors retain state and that `pane_not_found` behavior remains intact.

Run the repository verification gates from `docs/development.md` after the
focused regression tests are green.

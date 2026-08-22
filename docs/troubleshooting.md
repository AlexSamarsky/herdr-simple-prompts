# Troubleshooting

## Native session is unavailable

Install the matching Herdr integration and restart the agent pane:

```bash
herdr integration install codex
# or
herdr integration install claude
```

For a Codex pane that was already running before the integration existed, focus
it and press `prefix+m`. The plugin recovers and verifies that pane before it
opens Simple Prompts. A Claude pane without metadata must still be restarted or
resumed after installing the Claude integration.

## Automatic recovery for already-running Codex panes

Ordinary `prefix+m` runs fail-closed recovery for only the focused Herdr pane.
It needs `jq` in addition to Herdr. The action opens Simple Prompts only after
Herdr retains the recovered id-based metadata. It never reads transcript
contents or prints a session identifier.
Herdr supplies its own executable path to the action, so the host CLI does not
need to be present in the plugin process `PATH`.

Recovery stops without opening a view when a required command is missing, the
agent surface is unreadable, the final native footer has zero or multiple
session identifiers, zero or multiple transcript filenames match, Herdr rejects
the report, or Herdr does not retain the reported metadata. Fix the reported
condition and press `prefix+m` again; do not hardcode a pane or session id.

For bounded operator diagnostics across all currently detected panes, this
repository keeps the same recovery logic available as a standalone fallback.
Clone the exact source, inspect the helper, then run it without arguments:

```bash
git clone https://github.com/AlexSamarsky/herdr-simple-prompts.git
cd herdr-simple-prompts
sed -n '1,240p' scripts/register-existing-sessions.sh
bash scripts/register-existing-sessions.sh
```

The fallback changes only unregistered Codex panes that meet the same strict
checks. Registered panes stay untouched. Claude panes without metadata are
skipped because their session identifier cannot be recovered with the same
guarantees.

## `prefix+m` does not open Simple Prompts

Confirm the binding exists, then reload the configuration:

```bash
herdr server reload-config
```

An already registered Codex pane needs no hardcoded session identifier. Simple
Prompts uses the id-based native session metadata reported by Herdr, resolves
the one matching local transcript, and supports both legacy `event_msg` and
current `response_item/message` conversation records.

If automatic recovery fails, confirm `jq` is installed and read the plugin
action diagnostic for the exact fail-closed reason listed above. The standalone
helper is a diagnostic fallback, not a required step before ordinary hotkey
use.

If the plugin action log reports `pane_not_found` for a removed Simple Prompts
pane, press `prefix+m` again: the mapped source is validated, only the stale
source/plugin pair is removed, and a replacement view is targeted at that source
in the same invocation. Temporary permission, transport, or timeout errors keep
the mapping for a later retry. A temporary `agent_not_found` also keeps the
mapping when the source pane still exists, so `prefix+m` can close the existing
view and return focus to that source.

## An image is not attached

Confirm the native agent pane attaches the same image on its own first. Simple
Prompts deliberately reports failure when the native attachment marker cannot be
verified.

## Status fields are missing

Model and usage parsing is conservative. When a new agent version changes its
footer, unproven fields are omitted rather than invented. The agent kind and the
known working directory stay visible.

## Uninstall

```bash
herdr plugin uninstall herdr.simple-prompts
```

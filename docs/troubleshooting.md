# Troubleshooting

## Native session is unavailable

Install the matching Herdr integration and restart the agent pane:

```bash
herdr integration install codex
# or
herdr integration install claude
```

For Codex panes that were already running before the integration existed, use
the recovery helper below.

## Recovering already-running Codex panes

Sessions started before their integration was installed may have no native
session metadata, so the hotkey cannot target them safely. This repository ships
an optional, fail-closed helper. It needs `jq` and `rg` in addition to Herdr.

Clone the exact source, read the helper, then run it:

```bash
git clone https://github.com/AlexSamarsky/herdr-simple-prompts.git
cd herdr-simple-prompts
sed -n '1,240p' scripts/register-existing-sessions.sh
bash scripts/register-existing-sessions.sh
```

It changes only unregistered Codex panes whose final visible footer holds one
unambiguous session identifier with exactly one matching local transcript. It
never reads transcript contents and never prints identifiers; registered panes
stay untouched. Claude panes without metadata are skipped, because their session
identifier cannot be recovered with the same guarantees - restart or resume
those panes after installing the Claude integration.

## `prefix+m` does not open Simple Prompts

Confirm the binding exists, then reload the configuration:

```bash
herdr server reload-config
```

An already registered Codex pane needs no hardcoded session identifier. Simple
Prompts uses the id-based native session metadata reported by Herdr, resolves
the one matching local transcript, and supports both legacy `event_msg` and
current `response_item/message` conversation records.

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

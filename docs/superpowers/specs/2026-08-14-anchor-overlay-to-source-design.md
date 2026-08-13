# Anchor Simple Prompts Overlay to Its Source Pane

## Problem

The toggle action knows the source pane, but `plugin.pane.open` currently sends
only `placement = "overlay"`. Herdr therefore has to infer the target workspace
and pane from mutable UI focus. If focus changes while the action runs, Simple
Prompts can cover a different pane, including a pane on the right, instead of
replacing the source agent pane.

## Options Considered

1. **Anchor the open request to the source pane (recommended).** Read the
   source pane metadata immediately before opening, then pass both its
   `workspace_id` and `target_pane_id` to `plugin.pane.open`. This uses the
   targeting fields already exposed by Herdr 0.7.5 and removes dependence on
   ambient focus.
2. **Focus the source pane before opening.** This still leaves a race between
   the focus request and the open request and causes an extra visible focus
   transition.
3. **Rely on manifest placement only.** The manifest controls the placement
   kind but does not identify which pane or workspace must own the overlay, so
   it does not solve the intermittent behavior.

## Design

`toggle` will continue to validate the source agent and session exactly as it
does now. Before opening the overlay, it will read the source pane with
`pane.get`, require a non-empty `workspace_id`, and pass a small target value to
`HerdrClient::plugin_pane_open`.

The open request will contain:

- `placement: "overlay"`;
- `target_pane_id`: the validated source pane ID;
- `workspace_id`: the workspace returned for that source pane;
- the existing source-pane environment variable and `focus: true`.

No fallback to active focus is allowed. If Herdr returns no usable workspace
for the source, the toggle fails without opening a pane. Existing stale-overlay
recovery uses the same anchored open path.

## Testing

The Herdr client contract test will first fail against the current request,
then prove that `plugin.pane.open` contains the explicit source target and
workspace. Toggle tests will prove that both ordinary opening and stale-overlay
recovery obtain the source metadata and use the anchored request. The full
Rust formatting, Clippy, test, and release-build gates remain required.

## Non-goals

- No layout, rendering, history, composer, or hotkey changes.
- No new dependency or persisted state.
- No automatic migration of unrelated stale registry entries.

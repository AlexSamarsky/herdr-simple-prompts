# Anchor Simple Prompts to Its Source Pane

## Problem

The toggle action knows the exact source pane, but Simple Prompts uses Herdr's
`overlay` placement. Herdr 0.7.5 deliberately rejects `target_pane_id` and
`workspace_id` for overlay panes and always derives their context from the
currently active pane. If focus changes while the action process is starting,
Simple Prompts can therefore cover a different pane.

The first implementation attempted to add both targeting fields to an overlay
request. Read-only review against the official Herdr 0.7.5 source showed that
this request is invalid: the API validation says that overlay panes target the
active pane, and `open_plugin_overlay_pane` never consumes either field.

## Options Considered

1. **Use Herdr's targeted `zoomed` placement (selected).** A zoomed plugin pane
   is created as a split next to an exact `target_pane_id`, then Herdr zooms the
   new pane to the full tab. The visible result remains a replacement view, but
   targeting no longer depends on mutable focus. When the plugin pane closes,
   Herdr removes the split and clears the tab's zoomed state.
2. **Add explicit overlay targeting to Herdr.** This would preserve the overlay
   implementation internally, but requires an upstream Herdr change, a release,
   and a higher `min_herdr_version` before the source-only plugin can rely on it.
3. **Focus the source before opening an overlay.** This retains the same race
   between the focus request and the overlay request and can visibly switch
   panes before opening.

## Design

`toggle` will continue to validate the exact source agent and session. Opening
Simple Prompts will call `plugin.pane.open` with:

- `placement: "zoomed"`;
- `target_pane_id`: the verified source pane ID;
- the existing source-pane environment variable;
- `focus: true`.

The request will not send `workspace_id`: Herdr 0.7.5 explicitly rejects it for
`split` and `zoomed` placement and resolves the workspace from the target pane.
The manifest will also declare `placement = "zoomed"` so its default matches
the runtime request.

Ordinary opening no longer needs an extra `pane.get` just to recover workspace
metadata. Stale-pane recovery still probes the source with `pane.get` before
reopening, but it no longer focuses the source first: the targeted zoomed open
selects the source tab and focuses the new plugin pane atomically inside Herdr.

The persisted source-to-plugin-pane mapping remains unchanged. Closing the
plugin pane through the existing toggle path removes the temporary split;
Herdr's `Tab::detach_pane` clears `zoomed`, after which the plugin explicitly
focuses the unchanged source pane.

## Evidence

The official Herdr 0.7.5 implementation establishes the host contract:

- `handle_plugin_pane_open` rejects target fields for `overlay`, but accepts
  `target_pane_id` for `zoomed`;
- `open_plugin_split_pane` resolves the exact target pane and sets `tab.zoomed`;
- `Tab::detach_pane`, reached by `plugin.pane.close`, resets `zoomed` to false.

Plugin tests cover the request and manifest contracts. The host-side behavior
is not reimplemented or simulated as proof of Herdr layout semantics.

## Testing

Client tests will first fail while the request still uses `overlay` and
`workspace_id`, then prove the exact `zoomed` request. Manifest tests will first
fail while the default remains `overlay`. Toggle tests will prove that ordinary
opening uses no focus-sensitive workspace lookup and stale recovery targets the
source without an intermediate `pane.focus`. The complete Rust formatting,
Clippy, test, and release-build gates remain required.

## Non-goals

- No Herdr host patch.
- No rendering, history, composer, or hotkey changes.
- No new dependency or persisted state.
- No automatic migration of unrelated stale registry entries.

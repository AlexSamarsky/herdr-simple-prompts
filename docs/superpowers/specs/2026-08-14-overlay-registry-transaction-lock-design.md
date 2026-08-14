# Overlay Registry Transaction Lock Design

## Problem

Simple Prompts stores the source-pane to overlay-pane association in one
`registry.json` file. The hotkey action and the newly spawned plugin UI run as
separate processes. Both validate pane namespaces and can mutate the registry
with an unlocked read-modify-write sequence.

This permits a live overlay mapping to be lost. A later `prefix+m` opens a new
zoomed overlay because the old one is no longer registered. Closing the new
overlay then reveals the old orphaned overlay in the split layout, which looks
like the second hotkey press opened a pane on the right.

## Chosen approach

Serialize lifecycle state mutations with an advisory file lock implemented
through the platform `flock` API already used by the history journal. No new
Rust dependency is introduced.

The lock covers complete lifecycle transactions rather than individual JSON
writes:

- toggle action: namespace validation, overlay lookup, Herdr open/focus/close,
  registry update, and namespace binding;
- UI startup: namespace validation and binding for the source session;
- source-pane cleanup: registry, namespace, draft, and history cleanup.

Holding one lock across the full toggle transaction also prevents two action
processes from both observing an absent mapping and opening duplicate overlays.
The existing atomic JSON write remains responsible for crash-safe file
replacement; the new advisory lock is responsible for process coordination.

## Lock contract

- The lock file lives inside `HERDR_PLUGIN_STATE_DIR` and is opened with private
  permissions and no symlink following.
- Acquisition retries non-blockingly for a bounded interval so a broken process
  cannot freeze the hotkey indefinitely.
- The operating system releases the advisory lock automatically when a process
  exits; normal scope exit releases it explicitly.
- Failure to acquire or operate the lock returns a visible plugin-state error
  without opening or closing a pane.
- Draft and history writers keep their existing independent synchronization;
  only lifecycle mutations use this lock.

## Code boundaries

- `StateStore` owns lock-file validation and exposes a scoped exclusive-lifecycle
  operation.
- `toggle::run_from_env` executes validation and `toggle` inside that scope.
- `ui::run_from_env` uses the same scope for startup validation/binding and for
  source-pane cleanup.
- Registry serialization and its on-disk schema remain unchanged.

## Tests

Add a regression test that acquires the lifecycle lock in one thread, starts a
second lifecycle mutation, and proves the second mutation cannot enter until
the first releases the lock. The completed serialized mutations must preserve
both independent overlay mappings.

Keep the existing toggle contract tests and run the full Rust test suite,
format check, and Clippy after implementation. A focused test must be observed
failing before production code is changed.

## Out of scope

- Discovering and closing overlays orphaned by versions installed before this
  fix.
- Changing Herdr zoom, split, or focus behavior.
- Changing history, draft, or transcript formats.
- Adding dependencies or changing the public plugin manifest.

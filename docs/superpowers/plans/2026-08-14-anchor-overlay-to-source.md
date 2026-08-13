# Targeted Zoomed Simple Prompts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. Before
> coding, invoke a tester-oriented skill. After each meaningful coding batch,
> invoke `superpowers:requesting-code-review`. Before any completion claim,
> invoke `superpowers:verification-before-completion`.

**Goal:** Make every `prefix+m` open Simple Prompts on the exact source agent
pane instead of whichever pane is active when Herdr handles the request.

**Architecture:** Use Herdr 0.7.5's targeted `zoomed` plugin-pane path. Send the
source as `target_pane_id`, omit the forbidden `workspace_id`, and let Herdr
create a temporary split and zoom it to the full source tab. Closing the plugin
pane removes that split and restores the source layout.

**Tech stack:** Rust 2024, serde_json, Unix-socket Herdr 0.7.5 API, existing
FakeHerdr/ScriptedHerdr integration fixtures.

---

### Task 1: Lock the supported host contract with failing tests

**Files:**

- Modify: `tests/herdr_client.rs`
- Modify: `tests/manifest_contract.rs`
- Modify: `tests/toggle_state.rs`

- [ ] Replace the invalid overlay client expectation with an exact request
  contract: `placement = "zoomed"`, `target_pane_id = source`, no
  `workspace_id`, unchanged source environment and focus.
- [ ] Change the manifest contract to require `placement = "zoomed"`.
- [ ] Change ordinary-open and stale-recovery toggle expectations to require no
  workspace metadata lookup and no intermediate source focus.
- [ ] Run the focused tests and record RED caused by the existing overlay
  request, manifest default, and extra calls.

### Task 2: Implement targeted zoomed opening

**Files:**

- Modify: `src/herdr/client.rs`
- Modify: `src/toggle.rs`
- Modify: `herdr-plugin.toml`
- Modify: `README.md`

- [ ] Replace `plugin_pane_open_anchored(source, workspace_id)` with
  `plugin_pane_open_targeted(source)` using `zoomed` plus exact
  `target_pane_id`, with no workspace field.
- [ ] Remove the now-unused workspace extractor and ordinary-open metadata
  lookup.
- [ ] Keep the stale source existence probe, but remove its focus call and open
  the targeted zoomed pane directly after verified agent identity.
- [ ] Change the manifest default to `zoomed` and document exact-pane behavior
  without claiming native overlay targeting.
- [ ] Run focused client, manifest, and toggle tests and record GREEN.

### Task 3: Review and verify

- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo clippy --all-targets --all-features -- -D warnings`.
- [ ] Run `cargo test --all-targets --all-features`.
- [ ] Run `cargo build --locked --release`.
- [ ] Run `git diff --check`.
- [ ] Request read-only review against this revised design and resolve every
  validated blocker.

### Task 4: Integrate and activate

- [ ] Commit the verified remediation.
- [ ] Merge the task branch into `main` and rerun the complete quality gate on
  the merged result.
- [ ] Relink the source checkout and reload Herdr configuration.
- [ ] Refresh only the registry mapping for this source session; never invoke
  the plugin action against Herdr's ambient active pane.

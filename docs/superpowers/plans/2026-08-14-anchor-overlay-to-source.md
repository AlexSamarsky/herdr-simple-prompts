# Anchor Simple Prompts Overlay to Source Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every `prefix+m` open place Simple Prompts over the exact source agent pane instead of relying on mutable active focus.

**Architecture:** Resolve and validate the source pane's workspace through `pane.get`, then send both `target_pane_id` and `workspace_id` in `plugin.pane.open`. Keep layout targeting inside the Herdr client and thread the validated workspace through the existing toggle and stale-overlay recovery paths.

**Tech Stack:** Rust 2024, serde_json, Unix-socket Herdr 0.7.5 API, existing FakeHerdr/ScriptedHerdr integration fixtures.

---

### Task 1: Encode an explicitly anchored Herdr open request

**Files:**
- Modify: `tests/herdr_client.rs`
- Modify: `src/herdr/client.rs`

**Required skill checkpoints:**
- Use `superpowers:test-driven-development` before the test change.
- Use `superpowers:requesting-code-review` after Tasks 1 and 2 form one meaningful batch.
- Use `superpowers:verification-before-completion` before claiming the task is complete.

- [ ] **Step 1: Write failing client contract tests**

Replace the active-focus contract test with a test that calls the intended API and asserts the complete target:

```rust
#[test]
fn overlay_open_is_anchored_to_the_source_pane_and_workspace() {
    let fake = support::FakeHerdr::start(|request| {
        assert_eq!(request["method"], "plugin.pane.open");
        assert_eq!(request["params"]["placement"], "overlay");
        assert_eq!(request["params"]["target_pane_id"], "w1:p1");
        assert_eq!(request["params"]["workspace_id"], "w1");
        assert_eq!(
            request["params"]["env"]["HERDR_SIMPLE_PROMPTS_SOURCE_PANE"],
            "w1:p1"
        );
        serde_json::json!({
            "id": request["id"],
            "result": {"plugin_pane":{"pane":{"pane_id":"w1:p9"}}}
        })
    });
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    let overlay = client.plugin_pane_open("w1:p1", "w1").unwrap();

    assert_eq!(overlay, "w1:p9");
}
```

Add extraction and rejection tests for source workspace metadata:

```rust
#[test]
fn pane_workspace_id_requires_non_empty_source_metadata() {
    let valid = support::ScriptedHerdr::start(vec![serde_json::json!({
        "pane": {"pane_id": "w1:p1", "workspace_id": "w1"}
    })]);
    let valid_client = HerdrClient::connect(valid.socket_path()).unwrap();
    assert_eq!(valid_client.pane_workspace_id("w1:p1").unwrap(), "w1");

    let missing = support::ScriptedHerdr::start(vec![serde_json::json!({
        "pane": {"pane_id": "w1:p1"}
    })]);
    let missing_client = HerdrClient::connect(missing.socket_path()).unwrap();
    assert!(missing_client.pane_workspace_id("w1:p1").is_err());
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test --test herdr_client overlay_open_is_anchored_to_the_source_pane_and_workspace -- --exact
cargo test --test herdr_client pane_workspace_id_requires_non_empty_source_metadata -- --exact
```

Expected: compilation fails because `plugin_pane_open` lacks the workspace argument and `pane_workspace_id` does not exist.

- [ ] **Step 3: Write the minimal Herdr client contract**

Add a typed workspace extractor:

```rust
pub fn pane_workspace_id(&self, pane_id: &str) -> Result<String, HerdrError> {
    let result = self.pane_get(pane_id)?;
    result
        .pointer("/pane/workspace_id")
        .and_then(Value::as_str)
        .filter(|workspace_id| !workspace_id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            HerdrError::Protocol(format!(
                "pane.get response for {pane_id} has no workspace id"
            ))
        })
}
```

Change the open method signature and request:

```rust
pub fn plugin_pane_open(
    &self,
    source: &str,
    workspace_id: &str,
) -> Result<String, HerdrError> {
    let result = self.call(
        "plugin.pane.open",
        json!({
            "plugin_id": "herdr.simple-prompts",
            "entrypoint": "simple-prompts",
            "placement": "overlay",
            "target_pane_id": source,
            "workspace_id": workspace_id,
            "env": {"HERDR_SIMPLE_PROMPTS_SOURCE_PANE": source},
            "focus": true
        }),
    )?;
    result
        .pointer("/plugin_pane/pane/pane_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| HerdrError::Protocol("plugin pane response has no pane id".to_owned()))
}
```

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run the two commands from Step 2. Expected: both tests pass.

### Task 2: Use the source workspace in every toggle open path

**Files:**
- Modify: `tests/toggle_state.rs`
- Modify: `src/toggle.rs`

**Required skill checkpoints:**
- Continue the active `superpowers:test-driven-development` cycle.
- Invoke `superpowers:requesting-code-review` after the meaningful coding batch.
- Invoke `superpowers:verification-before-completion` before completion.

- [ ] **Step 1: Make ordinary and stale-recovery tests require anchored targeting**

For `opening_an_overlay_persists_verified_session_namespace`, insert a
`pane.get` response between `agent.get` and `plugin.pane.open`, then assert the
request order and target:

```rust
let fake = support::ScriptedHerdr::start(vec![
    agent_info("w1:p1", "session-1"),
    json!({"type":"pane_info","pane":{"pane_id":"w1:p1","workspace_id":"w1"}}),
    json!({"plugin_pane":{"pane":{"pane_id":"w1:p9"}}}),
]);

// After toggle:
let requests = fake.requests();
assert_eq!(requests[1]["method"], "pane.get");
assert_eq!(requests[2]["params"]["target_pane_id"], "w1:p1");
assert_eq!(requests[2]["params"]["workspace_id"], "w1");
```

For `stale_overlay_action_context_refocuses_source_and_reopens_in_one_toggle`,
include `workspace_id: "w1"` in the existing successful source `pane.get`
response and assert the final open request contains both target fields:

```rust
assert_eq!(requests[4]["params"]["target_pane_id"], "w1:p1");
assert_eq!(requests[4]["params"]["workspace_id"], "w1");
```

Update the source `pane.get` response to carry `workspace_id: "w1"` in these
stale-context tests, preserving their existing error order:

- `transient_agent_probe_error_preserves_the_stale_mapping`;
- `source_disappearing_during_agent_probe_removes_only_the_stale_mapping`;
- `stale_overlay_focus_failure_preserves_the_mapping`;
- `source_disappearing_during_focus_removes_only_the_stale_mapping`;
- `replacement_open_failure_keeps_stale_cleanup_and_unrelated_mapping`.

Insert one source `pane.get` response with `workspace_id: "w1"` after
`agent.get` in the ordinary-open tests
`failed_registry_write_closes_the_new_overlay`,
`stale_overlay_is_replaced_without_disturbing_other_sources`, and
`opening_an_overlay_persists_verified_session_namespace`. Update their exact
method-order assertions to include `pane.get` before `plugin.pane.open`.

- [ ] **Step 2: Run the focused toggle tests and verify RED**

Run:

```bash
cargo test --test toggle_state opening_an_overlay_persists_verified_session_namespace -- --exact
cargo test --test toggle_state stale_overlay_action_context_refocuses_source_and_reopens_in_one_toggle -- --exact
```

Expected: failure because the toggle does not request or forward source
workspace metadata.

- [ ] **Step 3: Thread the validated workspace through toggle**

For ordinary opening:

```rust
fn open_overlay(client: &HerdrClient, state: &StateStore, source: &str) -> AppResult<()> {
    let identity = agent_identity(client, source)?;
    let workspace_id = client
        .pane_workspace_id(source)
        .map_err(|error| AppError::new("toggle", error.to_string()))?;
    open_verified_overlay(client, state, source, &identity.session_id, &workspace_id)
}
```

In stale recovery, replace the initial `pane_get(source)` existence probe with
`pane_workspace_id(source)` while keeping the same exact `pane_not_found`
cleanup behavior. Pass the resulting workspace into `open_verified_overlay`.
Change that function to accept `workspace_id: &str` and call:

```rust
let overlay = client
    .plugin_pane_open(source, workspace_id)
    .map_err(|error| AppError::new("toggle", error.to_string()))?;
```

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run the two commands from Step 2. Expected: both pass.

- [ ] **Step 5: Run the full toggle and client suites**

Run:

```bash
cargo test --test herdr_client
cargo test --test toggle_state
```

Expected: all tests pass with no warnings.

- [ ] **Step 6: Request code review and address only validated findings**

Review the diff from the design commit through the implementation commit for:

- any remaining unanchored `plugin.pane.open` path;
- missing workspace validation;
- stale-overlay cleanup regressions;
- accidental layout/history/composer changes.

- [ ] **Step 7: Run the final quality gate**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
git diff --check
```

Expected: every command exits 0.

- [ ] **Step 8: Commit, merge, install, and refresh only the current overlay**

Commit the implementation:

```bash
git add src/herdr/client.rs src/toggle.rs tests/herdr_client.rs tests/toggle_state.rs
git commit -m "fix simple prompts overlay targeting"
```

Merge the verified branch into `main`, rerun the final quality gate on the
merged result, then run:

```bash
herdr plugin link . --enabled
herdr server reload-config
```

Close and reopen only the registry mapping belonging to this source pane; do
not use a global active-pane plugin action.

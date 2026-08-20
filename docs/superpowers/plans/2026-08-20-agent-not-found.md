# `agent_not_found` Handling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Simple Prompts forget stale Codex sources when Herdr 0.7.5 reports `agent_not_found`, while preserving fail-closed behavior for transient and unrelated errors.

**Architecture:** Add an exact structured-error classifier to the Herdr client, then use it only at the three `agent.get` lifecycle boundaries that already treat `pane_not_found` as terminal. Keep ordinary agent identity lookup and all `pane.get` handling unchanged.

**Tech Stack:** Rust, Cargo integration tests, in-process fake Herdr JSON-RPC server.

---

## Task 1: Classify the exact Herdr error code

**Files:**
- Modify: `tests/herdr_client.rs`
- Modify: `src/herdr/client.rs`

- [x] Add a regression test proving `agent_not_found` is recognized and is not confused with `pane_not_found`:

```rust
#[test]
fn agent_not_found_is_classified_from_the_exact_herdr_api_code() {
    let fake = support::FakeHerdr::error("agent_not_found", "agent not found");
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    let error = client
        .call("agent.get", serde_json::json!({"target":"missing"}))
        .unwrap_err();

    assert!(error.is_agent_not_found());
    assert!(!error.is_pane_not_found());
}
```

- [x] Run the focused test and confirm RED because `is_agent_not_found` does not exist:

```bash
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --test herdr_client agent_not_found_is_classified_from_the_exact_herdr_api_code
```

- [x] Add the exact classifier next to `is_pane_not_found`:

```rust
pub fn is_agent_not_found(&self) -> bool {
    self.api_code() == Some("agent_not_found")
}
```

- [x] Re-run the focused test and confirm GREEN.

## Task 2: Treat `agent_not_found` as a terminal missing source

**Files:**
- Modify: `tests/toggle_state.rs`
- Modify: `src/ui/runtime.rs`
- Modify: `src/state.rs`
- Modify: `src/toggle.rs`

- [x] Add integration regressions proving that `agent_not_found`:
  - removes a saved namespace and its scoped state during startup validation;
  - removes only the exact stale overlay mapping during recovery;
  - is not generalized to unrelated/transient error codes.

- [x] Add a runtime unit regression proving that the lifecycle worker emits `SourcePaneClosed` when its post-timeout `agent.get` probe returns `agent_not_found`.

- [x] Run the new focused tests and confirm RED because the stale source is retained or the close event is not emitted:

```bash
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --test toggle_state agent_not_found
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --lib lifecycle_worker_detects_agent_not_found
```

- [x] Extend only the existing `agent.get` missing-source guards:

```rust
Err(error) if error.is_pane_not_found() || error.is_agent_not_found() => { /* existing cleanup */ }
```

Apply that guard in:

- `StateStore::validate_saved_namespaces`
- stale-overlay source recovery in `toggle`
- `source_pane_is_gone`

- [x] Re-run the focused tests and confirm GREEN.

- [x] Run the affected integration and library test targets:

```bash
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --test herdr_client
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --test toggle_state
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --lib
```

## Task 3: Review and verification

**Files:**
- Review: `src/herdr/client.rs`
- Review: `src/state.rs`
- Review: `src/toggle.rs`
- Review: `src/ui/runtime.rs`
- Review: `tests/herdr_client.rs`
- Review: `tests/toggle_state.rs`

- [x] Review the diff against `docs/superpowers/specs/2026-08-20-agent-not-found-design.md`, checking that no `pane.get` behavior or ordinary identity lookup changed.

- [x] Run formatting and lint gates:

```bash
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo fmt --check
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo clippy --locked --all-targets --all-features -- -D warnings
```

- [x] Run the complete test and release-build gates:

```bash
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --all-targets --all-features
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo build --locked --release
```

- [x] Build the linked repository target so the installed local Herdr plugin starts the fixed binary:

```bash
CARGO_TARGET_DIR=/Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts/target CARGO_NET_OFFLINE=true cargo build --locked --release
```

- [x] Confirm the working tree contains only the intended source, test, spec, and plan changes, then commit the implementation with a plain-prose message.

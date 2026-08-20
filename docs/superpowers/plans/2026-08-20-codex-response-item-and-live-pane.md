# Codex Response-Item and Live-Pane Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore the complete visible conversation for current Codex transcripts and keep an open Simple Prompts overlay closable when Herdr temporarily loses agent detection for a still-live pane.

**Architecture:** Extend only the Codex transcript adapter boundary to normalize the legacy and current native message records into the existing `ConversationEvent` model. Use `pane.get`, not `agent.get`, as the authority for destructive pane-lifecycle cleanup while keeping new-overlay creation dependent on verified native agent identity.

**Tech Stack:** Rust 1.88+, serde_json, Cargo integration tests, in-process fake Herdr JSON-RPC server.

---

## File map

- Create `tests/fixtures/codex/response_items.jsonl`: privacy-safe fixture for the current Codex record shape.
- Modify `src/agent/codex.rs`: parse current `response_item/message` records while retaining the legacy parser.
- Modify `tests/codex_parser.rs`: prove visible-message extraction and internal-message filtering.
- Modify `tests/transcript_follower.rs`: prove initial backfill and subsequent tailing for the current record shape.
- Modify `src/state.rs`: distinguish temporary agent loss from confirmed pane loss during namespace validation.
- Modify `src/toggle.rs`: preserve a stale-overlay mapping when its source pane is alive but its agent is temporarily unavailable.
- Modify `src/ui/runtime.rs`: probe pane lifetime directly after lifecycle wait timeouts.
- Modify `tests/toggle_state.rs`: cover namespace preservation, confirmed cleanup, stale recovery, and overlay-context closing.
- Modify `docs/behavior.md`: document both Codex transcript layouts and pane-authoritative cleanup.
- Modify `docs/troubleshooting.md`: explain current-session compatibility and retry behavior.

## Task 1: Parse current Codex message records

**Files:**
- Create: `tests/fixtures/codex/response_items.jsonl`
- Modify: `tests/codex_parser.rs`
- Modify: `src/agent/codex.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before changing fixture, test, or implementation code.
- Invoke `superpowers:requesting-code-review` after the parser batch is green.
- Reserve `superpowers:verification-before-completion` for the final repository gates.

- [x] **Step 1: Add a privacy-safe current-format fixture**

Create `tests/fixtures/codex/response_items.jsonl` with these complete records:

```jsonl
{"timestamp":"2026-08-20T17:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"first prompt"}],"id":"current-user-1"}}
{"timestamp":"2026-08-20T17:00:01Z","type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"hidden developer context"}],"id":"current-developer-1"}}
{"timestamp":"2026-08-20T17:00:02Z","type":"response_item","payload":{"type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"hidden progress"}],"id":"current-commentary-1"}}
{"timestamp":"2026-08-20T17:00:03Z","type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"hidden reasoning"}]}}
{"timestamp":"2026-08-20T17:00:04Z","type":"response_item","payload":{"type":"custom_tool_call","name":"hidden_tool","call_id":"hidden-call","input":"{}"}}
{"timestamp":"2026-08-20T17:00:05Z","type":"response_item","payload":{"type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"first answer"}],"id":"current-final-1"}}
{"timestamp":"2026-08-20T17:00:06Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"second"},{"type":"input_image","image_url":"file:///unverified.png"},{"type":"input_text","text":"prompt"}],"id":"current-user-2"}}
{"timestamp":"2026-08-20T17:00:07Z","type":"response_item","payload":{"type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"second"},{"type":"output_text","text":"answer"}],"id":"current-final-2"}}
```

- [x] **Step 2: Add the failing adapter regression**

Append this test to `tests/codex_parser.rs`:

```rust
#[test]
fn parses_current_response_items_and_filters_internal_content() {
    let events = parse_fixture("tests/fixtures/codex/response_items.jsonl");

    assert_eq!(events.len(), 4);
    assert!(matches!(
        &events[0],
        ConversationEvent::User(message)
            if message.stable_id == "current-user-1"
                && message.text == "first prompt"
                && message.timestamp_ms == Some(1_787_245_200_000)
    ));
    assert!(matches!(
        &events[1],
        ConversationEvent::Final(message)
            if message.stable_id == "current-final-1" && message.text == "first answer"
    ));
    assert!(matches!(
        &events[2],
        ConversationEvent::User(message)
            if message.stable_id == "current-user-2"
                && message.text == "second\nprompt"
                && message.attachments.is_empty()
    ));
    assert!(matches!(
        &events[3],
        ConversationEvent::Final(message)
            if message.stable_id == "current-final-2" && message.text == "second\nanswer"
    ));
}
```

- [x] **Step 3: Run the focused test and confirm RED**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --test codex_parser parses_current_response_items_and_filters_internal_content
```

Expected: FAIL because the adapter returns zero events for `response_item/message` records.

- [x] **Step 4: Implement the minimal dual-layout parser**

Refactor `CodexAdapter::ingest_value` and add the response-item helpers in `src/agent/codex.rs`:

```rust
pub fn ingest_value(&mut self, line_number: u64, record: &Value) -> Option<ConversationEvent> {
    if truthy(record, "is_subagent") || record.get("subagent_id").is_some() {
        return None;
    }
    let payload = record.get("payload")?;
    if truthy(payload, "is_subagent") || payload.get("subagent_id").is_some() {
        return None;
    }
    match (
        record.get("type").and_then(Value::as_str),
        payload.get("type").and_then(Value::as_str),
    ) {
        (Some("event_msg"), Some("user_message")) => {
            parse_user(line_number, record, payload).map(ConversationEvent::User)
        }
        (Some("event_msg"), Some("agent_message"))
            if payload.get("phase").and_then(Value::as_str) == Some("final_answer") =>
        {
            parse_final(line_number, record, payload).map(ConversationEvent::Final)
        }
        (Some("response_item"), Some("message")) => {
            parse_response_message(line_number, record, payload)
        }
        _ => None,
    }
}

fn parse_response_message(
    line_number: u64,
    record: &Value,
    payload: &Value,
) -> Option<ConversationEvent> {
    match payload.get("role").and_then(Value::as_str) {
        Some("user") => response_text(payload, "input_text").map(|text| {
            ConversationEvent::User(Message {
                stable_id: stable_id(line_number, payload),
                text,
                presentation: crate::style::MessagePresentation::Plain,
                attachments: Vec::new(),
                timestamp_ms: record_timestamp_ms(record),
            })
        }),
        Some("assistant")
            if payload.get("phase").and_then(Value::as_str) == Some("final_answer") =>
        {
            response_text(payload, "output_text").map(|text| {
                ConversationEvent::Final(Message::text(
                    stable_id(line_number, payload),
                    text,
                    record_timestamp_ms(record),
                ))
            })
        }
        _ => None,
    }
}

fn response_text(payload: &Value, expected_type: &str) -> Option<String> {
    let parts = payload
        .get("content")?
        .as_array()?
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some(expected_type))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join("\n"))
}
```

- [x] **Step 5: Run all Codex parser tests and confirm GREEN**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --test codex_parser
```

Expected: all legacy and current-format parser tests pass.

- [x] **Step 6: Review and commit the parser batch**

Invoke `superpowers:requesting-code-review`, address any concrete findings, then run `cargo fmt --check` and commit:

```bash
git add tests/fixtures/codex/response_items.jsonl tests/codex_parser.rs src/agent/codex.rs
git commit -m "parse current Codex transcript messages"
```

## Task 2: Prove initial history restoration and live tailing

**Files:**
- Modify: `tests/transcript_follower.rs`

**Required skill checkpoints:**
- Continue the active `superpowers:test-driven-development` workflow.
- Invoke `superpowers:requesting-code-review` after the follower regression is green.
- Reserve `superpowers:verification-before-completion` for the final repository gates.

- [x] **Step 1: Add the initial-backfill and append regression**

Append this test to `tests/transcript_follower.rs`:

```rust
#[test]
fn current_codex_history_is_backfilled_then_new_messages_are_tailed() {
    let file = support::GrowingFile::new();
    file.append(&std::fs::read_to_string("tests/fixtures/codex/response_items.jsonl").unwrap());
    let mut follower = TranscriptFollower::new(file.path(), Box::new(CodexAdapter)).unwrap();

    let initial = follower.poll_initial(AgentStatus::Done).unwrap();

    assert_eq!(
        initial
            .iter()
            .filter(|event| matches!(event, FollowerEvent::Conversation(_)))
            .count(),
        4
    );

    file.append(
        &serde_json::json!({
            "timestamp": "2026-08-20T17:01:00Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "appended prompt"}],
                "id": "current-user-3"
            }
        })
        .to_string(),
    );
    file.append("\n");

    let appended = follower.poll().unwrap();
    assert!(matches!(
        appended.as_slice(),
        [FollowerEvent::Conversation(
            herdr_simple_prompts::model::ConversationEvent::User(message)
        )] if message.text == "appended prompt"
    ));
}
```

- [x] **Step 2: Run the focused follower test**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --test transcript_follower current_codex_history_is_backfilled_then_new_messages_are_tailed
```

Expected: PASS with the Task 1 parser in place. This is a regression test for the existing follower contract, so no new production code is expected.

- [x] **Step 3: Review and commit the follower regression**

Invoke `superpowers:requesting-code-review`, address any concrete findings, then commit:

```bash
git add tests/transcript_follower.rs
git commit -m "cover current Codex history following"
```

## Task 3: Make pane existence authoritative for lifecycle cleanup

**Files:**
- Modify: `tests/toggle_state.rs`
- Modify: `src/state.rs`
- Modify: `src/toggle.rs`
- Modify: `src/ui/runtime.rs`

**Required skill checkpoints:**
- Continue the active `superpowers:test-driven-development` workflow.
- Invoke `superpowers:requesting-code-review` after the lifecycle batch is green.
- Reserve `superpowers:verification-before-completion` for the final repository gates.

- [x] **Step 1: Replace the stale agent-not-found expectations with live-pane regressions**

In `tests/toggle_state.rs`, replace `namespace_validation_removes_state_for_agent_not_found` with a test that scripts `agent.get -> agent_not_found` followed by `pane.get -> pane_info`, then asserts that the overlay, draft, journal, and namespace remain and `orphaned_since_ms` is set to `5_000`.

Use these two exact namespace tests:

```rust
#[test]
fn namespace_validation_preserves_state_for_agent_not_found_in_a_live_pane() {
    let directory = test_state_directory("namespace-agent-not-found-live-pane");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    let journal = create_scoped_state(&store, &directory, "w1:p1", "session-1");
    let fake = support::ScriptedHerdr::start_responses(vec![
        Err(json!({"code": "agent_not_found", "message": "agent unavailable"})),
        Ok(json!({"type": "pane_info", "pane": {"pane_id": "w1:p1"}})),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    store.validate_saved_namespaces(&client, 5_000).unwrap();

    let namespace: serde_json::Value = serde_json::from_slice(
        &std::fs::read(namespace_path(&directory, "w1:p1")).unwrap(),
    )
    .unwrap();
    assert_eq!(namespace["orphaned_since_ms"], 5_000);
    assert_eq!(
        store.overlay_for_source("w1:p1").unwrap().as_deref(),
        Some("w1:p9")
    );
    assert!(directory.join("draft-w1_p1.json").exists());
    assert!(journal.exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn namespace_validation_removes_state_when_agent_and_pane_are_missing() {
    let directory = test_state_directory("namespace-agent-and-pane-missing");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    let journal = create_scoped_state(&store, &directory, "w1:p1", "session-1");
    let fake = support::ScriptedHerdr::start_responses(vec![
        Err(json!({"code": "agent_not_found", "message": "agent unavailable"})),
        Err(json!({"code": "pane_not_found", "message": "pane missing"})),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    store.validate_saved_namespaces(&client, 5_000).unwrap();

    assert!(store.overlay_for_source("w1:p1").unwrap().is_none());
    assert!(!directory.join("draft-w1_p1.json").exists());
    assert!(!journal.exists());
    assert!(!namespace_path(&directory, "w1:p1").exists());
    std::fs::remove_dir_all(directory).unwrap();
}
```

Replace `agent_not_found_during_agent_probe_removes_only_the_stale_mapping` with this exact preservation test:

```rust
#[test]
fn agent_not_found_for_a_live_source_preserves_the_stale_mapping() {
    let directory = test_state_directory("toggle-agent-not-found-live-pane");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    store.save_overlay("w1:p1", "w1:stale").unwrap();
    let fake = support::ScriptedHerdr::start_responses(vec![
        Err(json!({"code":"pane_not_found","message":"overlay missing"})),
        Ok(json!({"type":"pane_info","pane":{"pane_id":"w1:p1","workspace_id":"w1"}})),
        Err(json!({"code":"agent_not_found","message":"agent unavailable"})),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    assert!(toggle(&client, &store, "w1:stale").is_err());
    assert_eq!(
        store.overlay_for_source("w1:p1").unwrap().as_deref(),
        Some("w1:stale")
    );
    assert_eq!(fake.requests().len(), 3);
    std::fs::remove_dir_all(directory).unwrap();
}
```

Add the direct reproduction for the broken overlay-context hotkey:

```rust
#[test]
fn validation_keeps_a_live_sources_overlay_closable_after_agent_not_found() {
    let directory = test_state_directory("toggle-live-source-agent-gap");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    store.save_overlay("w1:p1", "w1:p9").unwrap();
    write_namespace(&directory, "w1:p1", "session-1", 1_000, None);
    let fake = support::ScriptedHerdr::start_responses(vec![
        Err(json!({"code":"agent_not_found","message":"agent unavailable"})),
        Ok(json!({"type":"pane_info","pane":{"pane_id":"w1:p1"}})),
        Ok(json!({"type":"pane_info","pane":{"pane_id":"w1:p9"}})),
        Ok(json!({"type":"plugin_pane_closed"})),
        Ok(json!({"type":"pane_focused"})),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    store.validate_saved_namespaces(&client, 5_000).unwrap();
    toggle(&client, &store, "w1:p9").unwrap();

    assert!(store.overlay_for_source("w1:p1").unwrap().is_none());
    assert_eq!(
        fake.requests()
            .iter()
            .map(|request| request["method"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["agent.get", "pane.get", "pane.get", "plugin.pane.close", "pane.focus"]
    );
    std::fs::remove_dir_all(directory).unwrap();
}
```

- [x] **Step 2: Replace the lifecycle worker's agent-oriented test**

In `src/ui/runtime.rs`, replace `lifecycle_worker_detects_agent_not_found_after_wait_timeout` with `lifecycle_worker_keeps_running_when_the_source_pane_still_exists`. Script `events.wait -> timeout` and `pane.get -> pane_info`, wait until both requests arrive, set the stop flag, join the worker, and assert no `SourcePaneClosed` event was emitted. Update the missed-close test's expected methods to `["events.wait", "pane.get"]`.

Use this exact replacement test:

```rust
#[test]
fn lifecycle_worker_keeps_running_when_the_source_pane_still_exists() {
    let (directory, socket, request_rx, server) = lifecycle_sequence_server(vec![
        Err(serde_json::json!({
            "code": "timeout",
            "message": "timed out waiting for event match"
        })),
        Ok(serde_json::json!({
            "type": "pane_info",
            "pane": {"pane_id": "w1:p1"}
        })),
    ]);
    let client = HerdrClient::connect(&socket).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let (event_tx, event_rx) = sync_channel(1);
    let worker = spawn_lifecycle_worker(Arc::clone(&stop), event_tx, client, "w1:p1".into());

    let methods = [
        request_rx.recv().unwrap()["method"]
            .as_str()
            .unwrap()
            .to_owned(),
        request_rx.recv().unwrap()["method"]
            .as_str()
            .unwrap()
            .to_owned(),
    ];
    stop.store(true, Ordering::Release);
    worker.join().unwrap();

    assert_eq!(methods, ["events.wait", "pane.get"]);
    assert!(event_rx.try_recv().is_err());
    server.join().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
```

- [x] **Step 3: Run the new lifecycle regressions and confirm RED**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --test toggle_state agent_not_found
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --test toggle_state validation_keeps_a_live_sources_overlay_closable
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --lib lifecycle_worker
```

Expected: at least the namespace, overlay-closing, and lifecycle tests fail against the current cleanup behavior.

- [x] **Step 4: Preserve namespaces when the agent is absent but the pane is live**

Replace the combined missing-source guard in `StateStore::validate_saved_namespaces` with:

```rust
Err(error) if error.is_pane_not_found() => {
    self.remove_pane_state(&namespace.source_pane)?;
}
Err(error) if error.is_agent_not_found() => {
    match client.pane_get(&namespace.source_pane) {
        Ok(_) => self.retain_or_expire_orphan(namespace, now_ms)?,
        Err(error) if error.is_pane_not_found() => {
            self.remove_pane_state(&namespace.source_pane)?;
        }
        Err(_) => self.retain_or_expire_orphan(namespace, now_ms)?,
    }
}
Err(_) => self.retain_or_expire_orphan(namespace, now_ms)?,
```

- [x] **Step 5: Preserve stale-overlay state on temporary agent loss**

In `recover_stale_overlay_context`, narrow the cleanup guard to confirmed pane loss:

```rust
let identity_response = match client.agent_get(source) {
    Ok(response) => response,
    Err(error) if error.is_pane_not_found() => {
        return remove_missing_source_mapping(state, source);
    }
    Err(error) => return Err(AppError::new("agent", error.to_string())),
};
```

- [x] **Step 6: Probe the source pane directly in the lifecycle worker**

Replace `source_pane_is_gone` with:

```rust
fn source_pane_is_gone(client: &HerdrClient, source_pane: &str) -> bool {
    matches!(
        client.pane_get(source_pane),
        Err(error) if error.is_pane_not_found()
    )
}
```

- [x] **Step 7: Run the focused state, toggle, and runtime suites**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --test toggle_state
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --lib
```

Expected: all tests pass, including confirmed pane cleanup and temporary-agent preservation.

- [x] **Step 8: Review and commit the lifecycle batch**

Invoke `superpowers:requesting-code-review`, address any concrete findings, then commit:

```bash
git add tests/toggle_state.rs src/state.rs src/toggle.rs src/ui/runtime.rs
git commit -m "preserve overlays for live source panes"
```

## Task 4: Update behavior guidance and verify the installed plugin

**Files:**
- Modify: `docs/behavior.md`
- Modify: `docs/troubleshooting.md`
- Modify: `docs/superpowers/plans/2026-08-20-codex-response-item-and-live-pane.md`

**Required skill checkpoints:**
- Tester-oriented checkpoint does not apply to the prose-only documentation step; behavior is already covered by Tasks 1-3.
- Invoke `superpowers:requesting-code-review` for the complete diff.
- Invoke `superpowers:verification-before-completion` before any success claim.

- [x] **Step 1: Document the implemented contracts**

In `docs/behavior.md`, state that Codex visible conversation may come from legacy `event_msg` records or current `response_item/message` records, and that only user messages and assistant `final_answer` content are shown. In the pane/session lifecycle table, state that temporary `agent_not_found` preserves state while confirmed pane loss removes it.

In `docs/troubleshooting.md`, add that an already-registered current Codex pane needs no hardcoded session identifier: Simple Prompts resolves its Herdr session metadata and supports the current transcript layout. Explain that `prefix+m` can still close an existing overlay while agent detection is temporarily unavailable.

- [x] **Step 2: Run formatting, lint, tests, helper tests, and release build**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo fmt --check
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo clippy --locked --all-targets --all-features -- -D warnings
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo test --locked --all-targets --all-features
bash tests/register-existing-sessions.sh
CARGO_TARGET_DIR=/private/tmp/herdr-simple-prompts-target CARGO_NET_OFFLINE=true cargo build --locked --release
```

Expected: every command exits zero with no warnings or failed tests.

- [x] **Step 3: Build the repository-linked release binary**

Run:

```bash
CARGO_TARGET_DIR=/Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts/target CARGO_NET_OFFLINE=true cargo build --locked --release
```

Expected: `target/release/herdr-simple-prompts` is rebuilt from the verified source.

- [x] **Step 4: Reload and perform a sanitized live smoke test**

Run `herdr server reload-config`, open Simple Prompts from the current source pane, and verify without printing transcript contents or identifiers that:

- the visible history contains at least one user message and one final answer;
- `prefix+m` closes the overlay and returns focus to the source;
- reopening restores the history from the beginning.

- [x] **Step 5: Complete final review and commit documentation**

Invoke `superpowers:requesting-code-review`, address any concrete findings, invoke `superpowers:verification-before-completion`, mark only completed plan checkboxes, and commit:

```bash
git add docs/behavior.md docs/troubleshooting.md docs/superpowers/plans/2026-08-20-codex-response-item-and-live-pane.md
git commit -m "document current Codex session recovery"
```

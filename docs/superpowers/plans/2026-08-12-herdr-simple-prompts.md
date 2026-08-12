# Herdr Simple Prompts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a source-only Herdr overlay plugin that presents only user prompts and final Codex/Claude answers while preserving native agent input, working state, images, and large multiline paste.

**Architecture:** A small Rust binary has a short-lived `toggle` mode and a long-lived `ui` mode. It talks to Herdr's newline-delimited JSON Unix socket, follows the native Codex or Claude JSONL transcript through agent-specific adapters, reduces events into normalized turns, and renders a Ratatui overlay whose composer forwards input to the unchanged source agent pane.

**Tech Stack:** Rust 2024, Ratatui, Crossterm, Serde/serde_json, unicode-width, Herdr plugin/socket API v1, GitHub Actions.

---

## File map

The implementation uses focused files with these responsibilities:

```text
herdr-plugin.toml                   Herdr source-build, toggle action, overlay pane
Cargo.toml                          crate metadata and minimal direct dependencies
Cargo.lock                          exact dependency resolution for --locked builds
src/main.rs                         argv dispatch for toggle/ui
src/lib.rs                          public module boundary for integration tests
src/error.rs                        contextual application error type
src/herdr/mod.rs                    typed Herdr facade exports
src/herdr/protocol.rs               evolvable request/response DTOs
src/herdr/client.rs                 newline JSON Unix-socket request client
src/agent/mod.rs                    supported-agent identity and adapter dispatch
src/agent/resolve.rs                native transcript path resolution
src/agent/codex.rs                  Codex JSONL normalization
src/agent/claude.rs                 stateful Claude JSONL normalization
src/agent/follower.rs               initial read and incremental complete-line tailing
src/model.rs                        normalized messages, turns, lifecycle and UI events
src/app.rs                          single reducer and optimistic reconciliation
src/editor.rs                       Unicode-safe multiline editor and paste handling
src/status.rs                       verified native status-line extraction
src/transport.rs                    prompt, interrupt and image forwarding safeguards
src/state.rs                        private atomic draft/overlay registry persistence
src/toggle.rs                       idempotent overlay open/focus/close controller
src/ui/mod.rs                       event loop and component coordination
src/ui/terminal.rs                  raw-mode/bracketed-paste RAII guard
src/ui/render.rs                    history, Working, composer and status rendering
tests/support/mod.rs                isolated temp dirs and fake Herdr socket
tests/fixtures/codex/*.jsonl        synthetic Codex transcript cases
tests/fixtures/claude/*.jsonl       synthetic Claude transcript cases
tests/herdr_client.rs               wire-contract tests
tests/transcript_follower.rs        append/truncate/partial-line tests
tests/toggle_lifecycle.rs           overlay controller tests
tests/ui_pty.rs                     terminal restoration and large-paste smoke tests
.github/workflows/ci.yml            source-only format/lint/test/build matrix
README.md                           install, keybinding, privacy and usage
LICENSE                             MIT license text
.gitignore                          Rust build/editor artifacts
```

## Task 1: Source-only crate and plugin contract

**Files:**
- Create: `Cargo.toml`
- Create: `Cargo.lock`
- Create: `herdr-plugin.toml`
- Create: `src/lib.rs`
- Create: `src/main.rs`
- Create: `src/error.rs`
- Create: `.gitignore`
- Test: `tests/manifest_contract.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before creating implementation files.
- Invoke `superpowers:requesting-code-review` after this task's coding batch.
- Use `superpowers:verification-before-completion` before marking the task complete.

- [ ] **Step 1: Write the failing manifest contract test**

```rust
#[test]
fn manifest_is_source_only_and_registers_toggle_overlay() {
    let manifest = std::fs::read_to_string("herdr-plugin.toml").unwrap();
    assert!(manifest.contains("id = \"herdr.simple-prompts\""));
    assert!(manifest.contains("min_herdr_version = \"0.7.5\""));
    assert!(manifest.contains("command = [\"cargo\", \"build\", \"--locked\", \"--release\"]"));
    assert!(manifest.contains("id = \"toggle\""));
    assert!(manifest.contains("id = \"simple-prompts\""));
    assert!(manifest.contains("placement = \"overlay\""));
    assert!(!manifest.contains("curl"));
    assert!(!manifest.contains("wget"));
}
```

- [ ] **Step 2: Run the test and confirm the repository has no crate yet**

Run: `cargo test --test manifest_contract`

Expected: FAIL because `Cargo.toml` is absent.

- [ ] **Step 3: Add locked minimal dependencies and crate metadata**

Create the package, then use Cargo to select current compatible crate releases and generate the lockfile:

```bash
cargo init --name herdr-simple-prompts --vcs none .
cargo add ratatui --no-default-features --features crossterm
cargo add crossterm serde --features serde/derive
cargo add serde_json unicode-width
```

Keep this metadata in `Cargo.toml`:

```toml
[package]
name = "herdr-simple-prompts"
version = "0.1.0"
edition = "2024"
rust-version = "1.85"
license = "MIT"
publish = false
description = "A prompt and final-answer overlay for Codex and Claude in Herdr"

[profile.release]
strip = true
lto = "thin"
codegen-units = 1
```

The generated `Cargo.lock` must be committed. Do not add an HTTP client, updater,
telemetry crate, async runtime, or shell installer.

- [ ] **Step 4: Add the exact source-only Herdr manifest**

```toml
id = "herdr.simple-prompts"
name = "Simple Prompts"
version = "0.1.0"
min_herdr_version = "0.7.5"
description = "Show only user prompts and final Codex or Claude answers"
platforms = ["linux", "macos"]

[[build]]
command = ["cargo", "build", "--locked", "--release"]

[[actions]]
id = "toggle"
title = "Toggle Simple Prompts"
contexts = ["pane"]
command = ["./target/release/herdr-simple-prompts", "toggle"]

[[panes]]
id = "simple-prompts"
title = "Simple Prompts"
placement = "overlay"
command = ["./target/release/herdr-simple-prompts", "ui"]
```

- [ ] **Step 5: Add explicit argv dispatch and contextual errors**

`src/main.rs` dispatches only known modes:

```rust
use herdr_simple_prompts::{run_toggle, run_ui};

fn main() {
    let result = match std::env::args().nth(1).as_deref() {
        Some("toggle") => run_toggle(),
        Some("ui") => run_ui(),
        _ => Err("usage: herdr-simple-prompts <toggle|ui>".into()),
    };
    if let Err(error) = result {
        eprintln!("herdr-simple-prompts: {error}");
        std::process::exit(1);
    }
}
```

`src/error.rs` defines `AppError { context: &'static str, message: String }`,
`Display`, `Error`, and `From<std::io::Error>`; `src/lib.rs` exports modules and
temporarily returns a descriptive not-yet-wired error from `run_toggle` and
`run_ui` so the crate compiles without hidden behavior.

- [ ] **Step 6: Run the contract and compilation tests**

Run: `cargo test --test manifest_contract && cargo check --all-targets`

Expected: PASS; the binary contains no download path.

- [ ] **Step 7: Commit the scaffold**

```bash
git add Cargo.toml Cargo.lock herdr-plugin.toml src tests/manifest_contract.rs .gitignore
git commit -m "scaffold source-only rust plugin"
```

## Task 2: Herdr socket protocol and synchronous client

**Files:**
- Create: `src/herdr/mod.rs`
- Create: `src/herdr/protocol.rs`
- Create: `src/herdr/client.rs`
- Modify: `src/lib.rs`
- Create: `tests/support/mod.rs`
- Test: `tests/herdr_client.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before production code.
- Invoke `superpowers:requesting-code-review` after the client batch.
- Use `superpowers:verification-before-completion` before task completion.

- [ ] **Step 1: Write failing wire-contract tests against a fake Unix socket**

```rust
#[test]
fn call_sends_one_json_line_and_matches_response_id() {
    let fake = support::FakeHerdr::start(|request| {
        assert_eq!(request["method"], "agent.prompt");
        assert_eq!(request["params"], serde_json::json!({"target":"w1:p1","text":"hello"}));
        serde_json::json!({"id": request["id"], "result": {"type":"agent_prompted"}})
    });
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let result = client.call("agent.prompt", serde_json::json!({"target":"w1:p1","text":"hello"})).unwrap();
    assert_eq!(result["type"], "agent_prompted");
}

#[test]
fn call_preserves_structured_api_error() {
    let fake = support::FakeHerdr::error("not_found", "pane not found");
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let error = client.call("pane.get", serde_json::json!({"pane_id":"missing"})).unwrap_err();
    assert_eq!(error.api_code(), Some("not_found"));
}
```

- [ ] **Step 2: Run the focused test and verify missing module failure**

Run: `cargo test --test herdr_client`

Expected: FAIL because `HerdrClient` and fake socket support are undefined.

- [ ] **Step 3: Implement evolvable protocol DTOs**

Use `serde_json::Value` at the protocol boundary so unknown Herdr fields are
ignored rather than rejected:

```rust
#[derive(serde::Serialize)]
pub struct Request<'a> {
    pub id: &'a str,
    pub method: &'a str,
    pub params: serde_json::Value,
}

#[derive(serde::Deserialize)]
pub struct Response {
    pub id: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<ApiError>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}
```

- [ ] **Step 4: Implement one-request-per-line Unix socket calls**

`HerdrClient` stores the socket path and an `AtomicU64`. Each call opens a fresh
`UnixStream`, writes exactly one serialized request plus `\n`, reads exactly one
response line, checks the response id, and returns either `result` or a typed
`HerdrError::Api`. This avoids sharing a request stream with subscriptions.

Add typed wrappers with these exact JSON params:

```rust
pub fn agent_prompt(&self, target: &str, text: &str) -> Result<Value, HerdrError>;
pub fn agent_send_keys(&self, target: &str, keys: &[&str]) -> Result<Value, HerdrError>;
pub fn pane_get(&self, pane_id: &str) -> Result<Value, HerdrError>;
pub fn pane_read_visible(&self, pane_id: &str, lines: u16) -> Result<Value, HerdrError>;
pub fn pane_send_input(&self, pane_id: &str, text: Option<&str>, keys: &[&str]) -> Result<Value, HerdrError>;
pub fn plugin_pane_open(&self, source: &str) -> Result<String, HerdrError>;
pub fn plugin_pane_focus(&self, pane_id: &str) -> Result<(), HerdrError>;
pub fn plugin_pane_close(&self, pane_id: &str) -> Result<(), HerdrError>;
```

`plugin_pane_open` sends plugin id `herdr.simple-prompts`, entrypoint
`simple-prompts`, placement `overlay`, `target_pane_id`, focus `true`, and env
`HERDR_SIMPLE_PROMPTS_SOURCE_PANE=<source>`, then extracts
`result.plugin_pane.pane.pane_id`.

- [ ] **Step 5: Add fake-server edge cases and pass the suite**

Cover partial response writes, malformed JSON, mismatched ids, EOF, and unknown
response fields. Run: `cargo test --test herdr_client`

Expected: PASS with no hanging sockets.

- [ ] **Step 6: Commit the protocol client**

```bash
git add src/herdr src/lib.rs tests/support tests/herdr_client.rs
git commit -m "add herdr socket client"
```

## Task 3: Agent identity and transcript resolution

**Files:**
- Create: `src/agent/mod.rs`
- Create: `src/agent/resolve.rs`
- Modify: `src/herdr/client.rs`
- Modify: `src/lib.rs`
- Test: `src/agent/resolve.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before production code.
- Invoke `superpowers:requesting-code-review` after identity/resolution work.
- Use `superpowers:verification-before-completion` before task completion.

- [ ] **Step 1: Write failing resolver tests with isolated homes**

```rust
#[test]
fn resolves_codex_session_by_native_id_under_codex_home() {
    let root = TestTree::new();
    let wanted = root.file("codex/sessions/2026/08/12/rollout-x-session-123.jsonl", "{}\n");
    let paths = AgentPaths::new(root.path("home"), Some(root.path("codex")), None);
    assert_eq!(resolve_transcript(AgentKind::Codex, "session-123", &paths).unwrap(), wanted);
}

#[test]
fn resolves_claude_session_by_exact_filename_and_rejects_side_files() {
    let root = TestTree::new();
    root.file("claude/projects/a/not-session-123.jsonl", "{}\n");
    let wanted = root.file("claude/projects/a/session-123.jsonl", "{}\n");
    let paths = AgentPaths::new(root.path("home"), None, Some(root.path("claude")));
    assert_eq!(resolve_transcript(AgentKind::Claude, "session-123", &paths).unwrap(), wanted);
}
```

- [ ] **Step 2: Verify red state**

Run: `cargo test agent::resolve::tests`

Expected: FAIL because the agent types and resolver do not exist.

- [ ] **Step 3: Parse Herdr agent identity without screen guessing**

Define:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentKind { Codex, Claude }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentIdentity {
    pub pane_id: String,
    pub kind: AgentKind,
    pub session_id: String,
    pub cwd: std::path::PathBuf,
    pub status: AgentStatus,
}
```

`HerdrClient::agent_identity(pane_id)` calls `agent.get` with
`{"target": pane_id}` and requires an `agent_session` whose agent is exactly
`codex` or `claude`, kind is `id`, and value is non-empty. Prefer
`foreground_cwd`, then `cwd`. Unsupported agents return an explicit error.

- [ ] **Step 4: Implement bounded transcript discovery**

`AgentPaths::from_env()` uses `CODEX_HOME` before `$HOME/.codex` and
`CLAUDE_CONFIG_DIR` before `$HOME/.claude`. The resolver:

- validates session ids contain only ASCII letters, digits, `-`, and `_`,
- scans only the relevant native sessions/projects root,
- follows no directory symlink outside that root,
- matches Codex filenames containing the complete session id,
- matches Claude's exact `<session-id>.jsonl` filename,
- returns an error on zero or multiple matches.

- [ ] **Step 5: Run resolver and client tests**

Run: `cargo test agent::resolve::tests && cargo test --test herdr_client`

Expected: PASS.

- [ ] **Step 6: Commit identity and resolution**

```bash
git add src/agent src/herdr/client.rs src/lib.rs
git commit -m "resolve native agent transcripts"
```

## Task 4: Normalized turns and optimistic reducer

**Files:**
- Create: `src/model.rs`
- Create: `src/app.rs`
- Modify: `src/lib.rs`
- Test: `src/app.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before reducer code.
- Invoke `superpowers:requesting-code-review` after the reducer batch.
- Use `superpowers:verification-before-completion` before task completion.

- [ ] **Step 1: Write failing reconciliation tests**

```rust
#[test]
fn submitted_prompt_is_visible_then_reconciles_without_duplicate() {
    let mut app = AppState::default();
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        text: "ship it".into(),
        attachments: vec![],
        at_ms: 100,
    });
    assert_eq!(app.turns.len(), 1);
    assert!(matches!(app.turns[0].delivery, Delivery::Optimistic { .. }));

    app.apply(AppEvent::NativeUser(Message::text("native-1", "ship it", Some(120))));
    assert_eq!(app.turns.len(), 1);
    assert_eq!(app.turns[0].prompt.stable_id, "native-1");
    assert_eq!(app.turns[0].delivery, Delivery::Native);
}

#[test]
fn send_failure_restores_draft_and_marks_turn_failed() {
    let mut app = AppState::default();
    app.apply(AppEvent::PromptSubmitted { local_id:"l1".into(), text:"retry".into(), attachments:vec![], at_ms:1 });
    app.apply(AppEvent::SendFailed { local_id:"l1".into(), reason:"pane closed".into() });
    assert_eq!(app.draft.text(), "retry");
    assert!(matches!(app.turns[0].delivery, Delivery::Failed { .. }));
}
```

- [ ] **Step 2: Verify red state**

Run: `cargo test app::tests`

Expected: FAIL because model and reducer types are undefined.

- [ ] **Step 3: Define the normalized model**

```rust
pub struct Attachment { pub id: String, pub display: String, pub native_path: Option<PathBuf> }
pub struct Message { pub stable_id: String, pub text: String, pub attachments: Vec<Attachment>, pub timestamp_ms: Option<u64> }
pub enum Delivery { Native, Optimistic { local_id: String, submitted_at_ms: u64 }, Failed { reason: String } }
pub struct Turn { pub prompt: Message, pub final_answer: Option<Message>, pub delivery: Delivery }
pub enum AgentStatus { Idle, Working, Blocked, Done, Unknown }
```

Define `AppEvent` variants for native user/final answer, prompt submit/failure,
status change, transcript error, socket state, source close/session change,
attachment add/failure, scrolling, and draft editing.

- [ ] **Step 4: Implement deterministic reconciliation**

Match a native user record to the oldest unmatched optimistic turn when:

1. normalized newlines and trailing whitespace match,
2. attachment counts match, and
3. timestamps differ by at most 30 seconds when both exist.

Otherwise append a native turn. Attach final answers to the oldest native turn
without an answer. Keep a failed prompt distinct and restore its exact text and
attachments to the composer.

- [ ] **Step 5: Cover ordering and lifecycle cases**

Add tests for two prompts submitted while working, interrupted unanswered turn,
manual scroll suspending auto-scroll, and reconnect preserving turns/draft.
Run: `cargo test app::tests`

Expected: PASS.

- [ ] **Step 6: Commit the reducer**

```bash
git add src/model.rs src/app.rs src/lib.rs
git commit -m "add normalized conversation reducer"
```

## Task 5: Codex transcript adapter

**Files:**
- Create: `src/agent/codex.rs`
- Modify: `src/agent/mod.rs`
- Create: `tests/fixtures/codex/complete.jsonl`
- Create: `tests/fixtures/codex/filtered.jsonl`
- Test: `src/agent/codex.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before parser code.
- Invoke `superpowers:requesting-code-review` after the adapter batch.
- Use `superpowers:verification-before-completion` before task completion.

- [ ] **Step 1: Add synthetic JSONL and failing parser tests**

The complete fixture contains exact event shapes:

```json
{"timestamp":"2026-08-12T10:00:00Z","type":"event_msg","payload":{"type":"user_message","message":"build it","images":[]}}
{"timestamp":"2026-08-12T10:00:01Z","type":"event_msg","payload":{"type":"agent_message","message":"I am checking files","phase":"commentary"}}
{"timestamp":"2026-08-12T10:00:02Z","type":"event_msg","payload":{"type":"agent_message","message":"Built and verified.","phase":"final_answer"}}
```

Test:

```rust
#[test]
fn emits_only_user_and_final_answer() {
    let events = parse_fixture("tests/fixtures/codex/complete.jsonl");
    assert_eq!(events, vec![
        ConversationEvent::User(Message::text_with_id("codex:1", "build it")),
        ConversationEvent::Final(Message::text_with_id("codex:3", "Built and verified.")),
    ]);
}
```

- [ ] **Step 2: Verify commentary currently cannot be filtered**

Run: `cargo test agent::codex::tests`

Expected: FAIL because the adapter is absent.

- [ ] **Step 3: Implement strict Codex event acceptance**

`CodexAdapter::ingest(line_number, Value)` emits only:

```rust
match (record["type"].as_str(), record["payload"]["type"].as_str()) {
    (Some("event_msg"), Some("user_message")) => parse_user(...),
    (Some("event_msg"), Some("agent_message"))
        if record["payload"]["phase"] == "final_answer" => parse_final(...),
    _ => None,
}
```

Extract string messages and image arrays; reject empty visible content. Stable
ids use a native item id when present and `codex:<line-number>` otherwise.

- [ ] **Step 4: Prove all hidden classes stay hidden**

`filtered.jsonl` includes reasoning, commentary, tool calls, tool results,
developer/system context, approval events, and a record marked as subagent
traffic. Assert the adapter emits no event for every line. Also test one user
message with two image paths becomes two attachments.

Run: `cargo test agent::codex::tests`

Expected: PASS.

- [ ] **Step 5: Commit the Codex adapter**

```bash
git add src/agent/codex.rs src/agent/mod.rs tests/fixtures/codex
git commit -m "parse codex final-answer history"
```

## Task 6: Claude transcript adapter

**Files:**
- Create: `src/agent/claude.rs`
- Modify: `src/agent/mod.rs`
- Create: `tests/fixtures/claude/simple.jsonl`
- Create: `tests/fixtures/claude/tool_cycle.jsonl`
- Create: `tests/fixtures/claude/filtered.jsonl`
- Test: `src/agent/claude.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before parser code.
- Invoke `superpowers:requesting-code-review` after the adapter batch.
- Use `superpowers:verification-before-completion` before task completion.

- [ ] **Step 1: Write failing simple and tool-cycle tests**

```rust
#[test]
fn tool_cycle_commits_only_terminal_visible_text() {
    let mut adapter = ClaudeAdapter::default();
    let events = ingest_fixture(&mut adapter, "tests/fixtures/claude/tool_cycle.jsonl");
    assert_eq!(events, vec![ConversationEvent::User(Message::text_with_id("u1", "fix it"))]);
    assert_eq!(adapter.finalize_pending(), Some(ConversationEvent::Final(
        Message::text_with_id("a3", "Fixed and tested.")
    )));
}
```

The fixture order is real user `u1`, assistant text plus `tool_use` `a1`, user
`tool_result` `u2`, and text-only assistant `a3`.

- [ ] **Step 2: Verify red state**

Run: `cargo test agent::claude::tests`

Expected: FAIL because `ClaudeAdapter` is absent.

- [ ] **Step 3: Implement stateful Claude normalization**

`ClaudeAdapter` tracks the active real user turn, whether a tool cycle occurred,
and the last text-only assistant candidate. It applies these rules:

- require top-level `type` `user` or `assistant`,
- reject `isSidechain == true`,
- treat user content containing `tool_result` as protocol traffic,
- emit a real user prompt immediately,
- ignore `thinking` and `tool_use` blocks,
- never commit assistant text from a message that also contains `tool_use`,
- retain the latest non-empty text-only assistant message as the candidate,
- emit that candidate only from `finalize_pending()` after Herdr leaves working
  or before the next real user prompt is emitted.

- [ ] **Step 4: Add exclusion and interruption fixtures**

Cover thinking, hook/progress/meta records, sidechain records, tool results,
empty text, a simple no-tool final answer, and interruption with no candidate.
Run: `cargo test agent::claude::tests`

Expected: PASS and no reasoning/progress text in normalized output.

- [ ] **Step 5: Commit the Claude adapter**

```bash
git add src/agent/claude.rs src/agent/mod.rs tests/fixtures/claude
git commit -m "parse claude final-answer history"
```

## Task 7: Safe incremental transcript follower

**Files:**
- Create: `src/agent/follower.rs`
- Modify: `src/agent/mod.rs`
- Test: `tests/transcript_follower.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before follower code.
- Invoke `superpowers:requesting-code-review` after the follower batch.
- Use `superpowers:verification-before-completion` before task completion.

- [ ] **Step 1: Write failing append/partial/truncate tests**

```rust
#[test]
fn waits_for_complete_json_line_then_emits_once() {
    let file = support::GrowingFile::new();
    let mut follower = TranscriptFollower::new(file.path(), Box::new(CodexAdapter::default())).unwrap();
    file.append("{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",");
    assert!(follower.poll().unwrap().is_empty());
    file.append("\"message\":\"hello\",\"images\":[]}}\n");
    assert_eq!(follower.poll().unwrap().len(), 1);
    assert!(follower.poll().unwrap().is_empty());
}
```

- [ ] **Step 2: Verify red state**

Run: `cargo test --test transcript_follower`

Expected: FAIL because `TranscriptFollower` is absent.

- [ ] **Step 3: Implement complete-line tailing**

Store path, file identity metadata, byte offset, line number, an incomplete byte
buffer, and `Box<dyn TranscriptAdapter + Send>`. Initial load reads to EOF.
`poll()` reads appended bytes, splits only on `\n`, keeps the final partial line,
and converts invalid complete JSON into a contextual non-fatal parse event.

On file size below offset or file identity replacement, reset to byte zero,
clear partial bytes, reset the adapter, and return `FollowerEvent::Reloaded`
before reparsing. Never busy-loop; the runtime polls at 100 ms.

- [ ] **Step 4: Pass replacement and malformed-tail cases**

Add tests for truncation, atomic replacement, malformed complete line followed by
a valid line, no duplicate initial load, and a two-megabyte user prompt line.
Run: `cargo test --test transcript_follower`

Expected: PASS with the large line preserved exactly.

- [ ] **Step 5: Commit the follower**

```bash
git add src/agent/follower.rs src/agent/mod.rs tests/transcript_follower.rs tests/support
git commit -m "follow native transcript updates"
```

## Task 8: Multiline editor, paste, and attachment recognition

**Files:**
- Create: `src/editor.rs`
- Modify: `src/model.rs`
- Modify: `src/app.rs`
- Test: `src/editor.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before editor code.
- Invoke `superpowers:requesting-code-review` after the input batch.
- Use `superpowers:verification-before-completion` before task completion.

- [ ] **Step 1: Write failing Unicode and large-paste tests**

```rust
#[test]
fn paste_preserves_two_megabytes_and_newlines() {
    let mut editor = Editor::default();
    let pasted = format!("first\n{}\nlast", "я".repeat(1_000_000));
    editor.insert_paste(&pasted);
    assert_eq!(editor.text(), pasted);
    assert!(editor.cursor_byte().is_char_boundary());
}

#[test]
fn shift_enter_and_ctrl_j_insert_newline_while_enter_submits() {
    assert_eq!(map_key(Key::Enter), EditorCommand::Submit);
    assert_eq!(map_key(Key::ShiftEnter), EditorCommand::Newline);
    assert_eq!(map_key(Key::Ctrl('j')), EditorCommand::Newline);
}
```

- [ ] **Step 2: Verify red state**

Run: `cargo test editor::tests`

Expected: FAIL because `Editor` is absent.

- [ ] **Step 3: Implement a char-boundary-safe editor**

`Editor` stores `String`, byte cursor, preferred display column, and selection-free
scroll state. Implement insert char/text, newline, backspace/delete, left/right,
up/down across lines, home/end, clear, replace, and `take_submission`. Every
mutation preserves a valid UTF-8 boundary. Display-column movement uses
`unicode_width::UnicodeWidthChar`.

- [ ] **Step 4: Recognize only Herdr-staged remote image paths**

```rust
pub fn staged_image_path(text: &str) -> Option<PathBuf> {
    let path = PathBuf::from(text.trim());
    let image_ext = matches!(path.extension()?.to_str()?.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp");
    let staged = path.components().any(|part|
        part.as_os_str().to_string_lossy().starts_with("herdr-clipboard-images-"));
    (image_ext && staged && path.is_file()).then_some(path)
}
```

An entire bracketed paste that matches becomes `EditorCommand::ForwardStagedImage`;
all other paste content is inserted verbatim. Test false positives, missing files,
spaces in paths, uppercase extensions, and normal multiline text.

- [ ] **Step 5: Run editor/reducer tests and commit**

Run: `cargo test editor::tests && cargo test app::tests`

Expected: PASS.

```bash
git add src/editor.rs src/model.rs src/app.rs
git commit -m "add native-like multiline composer"
```

## Task 9: Guarded agent transport and truthful status

**Files:**
- Create: `src/transport.rs`
- Create: `src/status.rs`
- Modify: `src/herdr/client.rs`
- Modify: `src/agent/mod.rs`
- Test: `src/transport.rs`
- Test: `src/status.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before transport/status code.
- Invoke `superpowers:requesting-code-review` after the batch.
- Use `superpowers:verification-before-completion` before task completion.

- [ ] **Step 1: Write failing source-revalidation tests**

```rust
#[test]
fn refuses_to_send_after_native_session_changes() {
    let fake = FakeHerdr::with_agent_sequence(vec![agent("codex", "s1"), agent("codex", "s2")]);
    let transport = AgentTransport::new(fake.client(), identity("w1:p1", "s1"));
    let error = transport.submit("do it").unwrap_err();
    assert!(error.to_string().contains("session changed"));
    assert_eq!(fake.count_method("agent.prompt"), 0);
}

#[test]
fn interrupt_uses_agent_send_keys_esc() {
    let fake = FakeHerdr::with_agent(identity_json("w1:p1", "codex", "s1"));
    AgentTransport::new(fake.client(), identity("w1:p1", "s1")).interrupt().unwrap();
    assert_eq!(fake.last_params("agent.send_keys"), json!({"target":"w1:p1","keys":["esc"]}));
}
```

- [ ] **Step 2: Verify red state**

Run: `cargo test transport::tests status::tests`

Expected: FAIL because transport/status modules are absent.

- [ ] **Step 3: Implement validation before every mutation**

`AgentTransport` holds the original `AgentIdentity`. `submit`, `interrupt`,
`forward_local_image_paste`, and `forward_staged_image` first call
`agent_identity(source_pane)` and compare kind plus native session id.

- `submit` calls `agent.prompt` with target and exact text.
- `interrupt` calls `agent.send_keys` with `esc` only while status is working.
- local image paste calls `agent.send_keys` with `ctrl+v`, then polls
  `pane.read` for a new native `[Image #N]` marker for at most 800 ms.
- remote staged image calls `pane.send_input` with the path as `text` and no
  keys, then performs the same marker verification.
- marker verification failure returns an attachment error and never creates a
  successful UI attachment.

- [ ] **Step 4: Implement conservative status extraction**

Define `StatusLine { agent, model, cwd, branch, usage }`, all optional except
agent. Codex and Claude extractors operate only on the final six visible source
rows, strip ANSI, and accept fields only when anchored to known separators and
labels. Unknown layouts return agent/cwd from `AgentIdentity` with other fields
`None`. Add fixtures matching the approved Codex screenshot and a current Claude
footer, plus changed/noisy layouts that must not invent usage.

- [ ] **Step 5: Run focused and regression tests**

Run: `cargo test transport::tests status::tests -- --nocapture && cargo test --test herdr_client`

Expected: PASS.

- [ ] **Step 6: Commit transport and status**

```bash
git add src/transport.rs src/status.rs src/herdr/client.rs src/agent/mod.rs
git commit -m "forward guarded agent input"
```

## Task 10: Private state and idempotent toggle controller

**Files:**
- Create: `src/state.rs`
- Create: `src/toggle.rs`
- Modify: `src/lib.rs`
- Modify: `src/herdr/client.rs`
- Test: `tests/toggle_lifecycle.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before controller code.
- Invoke `superpowers:requesting-code-review` after the controller batch.
- Use `superpowers:verification-before-completion` before task completion.

- [ ] **Step 1: Write failing open/focus/close tests**

```rust
#[test]
fn second_toggle_from_overlay_closes_and_refocuses_source() {
    let fake = FakeHerdr::with_existing_overlay("w1:p9");
    let state = StateStore::at(fake.state_dir());
    state.save_overlay("w1:p1", "w1:p9").unwrap();
    toggle(&fake.client(), &state, "w1:p9").unwrap();
    assert_eq!(fake.methods(), ["pane.get", "plugin.pane.close", "pane.focus"]);
    assert!(state.overlay_for_source("w1:p1").unwrap().is_none());
}
```

- [ ] **Step 2: Verify red state**

Run: `cargo test --test toggle_lifecycle`

Expected: FAIL because state and toggle modules are absent.

- [ ] **Step 3: Implement private atomic state files**

`StateStore` roots at required `HERDR_PLUGIN_STATE_DIR`. Serialize
`registry.json` and `draft-<sanitized-pane-id>.json` to a sibling temporary
file, set Unix mode `0o600`, `sync_all`, and rename atomically. The registry maps
source pane to overlay pane and also supports reverse lookup. Corrupt files are
renamed with `.invalid` and reported; they are never silently overwritten.

- [ ] **Step 4: Implement idempotent toggle decisions**

`run_toggle()` reads `HERDR_SOCKET_PATH`, `HERDR_PLUGIN_STATE_DIR`, and
`HERDR_PANE_ID`. It validates remembered panes with `pane.get`:

1. invoked from registered overlay: close overlay, focus source, remove record;
2. invoked from source with valid overlay: focus overlay;
3. stale record: remove it, validate supported source agent, open overlay, save;
4. unsupported or missing source: return a concise error without opening.

Add `HerdrClient::pane_focus` as raw `pane.focus` with `pane_id`.

- [ ] **Step 5: Pass lifecycle and permission tests**

Cover first open, focus existing, close, stale overlay, source close, two source
panes, corrupt state, and mode `0600`. Run: `cargo test --test toggle_lifecycle`

Expected: PASS.

- [ ] **Step 6: Commit toggle behavior**

```bash
git add src/state.rs src/toggle.rs src/lib.rs src/herdr/client.rs tests/toggle_lifecycle.rs
git commit -m "add simple prompts toggle controller"
```

## Task 11: Terminal guard, rendering, and coordinated UI runtime

**Files:**
- Create: `src/ui/mod.rs`
- Create: `src/ui/terminal.rs`
- Create: `src/ui/render.rs`
- Modify: `src/lib.rs`
- Modify: `src/app.rs`
- Test: `src/ui/render.rs`
- Test: `tests/ui_pty.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before UI code.
- Invoke `superpowers:requesting-code-review` after the meaningful UI batch.
- Use `superpowers:verification-before-completion` before task completion.

- [ ] **Step 1: Write failing deterministic layout tests**

Use Ratatui `TestBackend`:

```rust
#[test]
fn working_prompt_is_above_composer_and_footer() {
    let mut app = AppState::with_turn(user_turn("run tests"));
    app.agent_status = AgentStatus::Working;
    app.working_since = Some(Instant::now() - Duration::from_secs(2));
    let rendered = render_to_string(&app, 80, 24);
    let prompt = rendered.find("run tests").unwrap();
    let working = rendered.find("Working (2s • esc to interrupt)").unwrap();
    let composer = rendered.find("Write a prompt").unwrap();
    assert!(prompt < working && working < composer);
}

#[test]
fn hidden_agent_events_have_no_rendering_surface() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text("u1", "hello", Some(1))));
    app.apply(AppEvent::NativeFinal(Message::text("a1", "done", Some(2))));
    let rendered = render_to_string(&app, 80, 24);
    assert!(rendered.contains("hello"));
    assert!(rendered.contains("done"));
    assert!(!rendered.contains("tool_call"));
    assert!(!rendered.contains("reasoning"));
}
```

- [ ] **Step 2: Verify red state**

Run: `cargo test ui::render::tests`

Expected: FAIL because the UI modules are absent.

- [ ] **Step 3: Implement RAII terminal restoration**

`TerminalGuard::enter()` enables raw mode, alternate screen, bracketed paste,
and hides the cursor. `Drop` always disables bracketed paste, shows the cursor,
leaves alternate screen, and disables raw mode in reverse order. Install a panic
hook that performs best-effort restoration before delegating to the previous
hook.

- [ ] **Step 4: Render the approved hierarchy**

`render(frame, app)` computes bottom-up areas for one-line footer, composer
height clamped to 3..40% of viewport, optional one-line Working row, and remaining
history. Render user prompts with a `›` gutter, final answers with a `•` gutter,
attachment placeholders as `[Image #N]`, send errors inline, and a truthful
footer assembled only from present `StatusLine` fields. Reflow history from the
normalized model; do not store prewrapped lines.

- [ ] **Step 5: Coordinate input, transcript, status and rendering**

`run_ui()` requires `HERDR_SIMPLE_PROMPTS_SOURCE_PANE`, validates identity,
resolves and initially loads the transcript, restores the draft, then runs a
50 ms render/input tick. A follower thread polls JSONL every 100 ms and sends
bounded events. A Herdr observer thread polls agent identity/status and source
visible rows every 200 ms; on reconnect it revalidates the original session.

The UI loop maps:

- Enter to optimistic `PromptSubmitted`, `transport.submit`, then success/failure;
- Shift+Enter and Ctrl+J to newline;
- bracketed text paste to atomic editor insertion;
- staged image paste and Ctrl+V to transport attachment forwarding;
- Esc to interrupt only when working;
- PageUp/PageDown and mouse wheel to history scroll;
- resize to re-render without mutating the editor;
- source close/session replacement to disabled composer plus visible error.

Claude `finalize_pending()` runs when the observer sees a transition out of
working. Draft state is saved after edits with a 250 ms debounce and on exit.

- [ ] **Step 6: Add PTY restoration and paste smoke tests**

The PTY test launches the binary in `ui` mode against a fake Herdr socket and
temporary transcript, sends a bracketed two-megabyte multiline paste, resize,
and controlled quit signal, then asserts terminal restore sequences are emitted
and the fake agent receives the exact text once. Add a panic-in-render test that
still restores raw-mode sequences.

Run: `cargo test ui::render::tests && cargo test --test ui_pty -- --nocapture`

Expected: PASS without terminal leakage or truncated paste.

- [ ] **Step 7: Run the complete suite and commit UI**

Run: `cargo test --all-targets --all-features`

Expected: PASS.

```bash
git add src/ui src/lib.rs src/app.rs tests/ui_pty.rs
git commit -m "add simple prompts overlay ui"
```

## Task 12: Public documentation, CI, and live Herdr verification

**Files:**
- Create: `README.md`
- Create: `LICENSE`
- Create: `.github/workflows/ci.yml`
- Modify: `herdr-plugin.toml`
- Modify: `Cargo.toml`
- Modify: `docs/superpowers/specs/2026-08-12-herdr-simple-prompts-design.md` only if verified behavior differs
- Test: all existing tests plus live smoke commands

**Required skill checkpoints:**
- Documentation-only edits do not require a tester-oriented checkpoint, but any
  manifest or runtime correction discovered here must start with
  `superpowers:test-driven-development`.
- Invoke `superpowers:requesting-code-review` for the full repository.
- Invoke `superpowers:verification-before-completion` before any completion claim.

- [ ] **Step 1: Write the README from the verified contract**

Document these exact sections: purpose and screenshot placeholder description,
supported Herdr/Codex/Claude versions, Rust prerequisite, source-only trust
model, installation, local linking, `prefix+m` config, composer keys, image and
remote image behavior, privacy/no-network guarantee, uninstallation,
troubleshooting, limitations, development commands, and marketplace topic.

Installation and binding blocks are:

```bash
herdr plugin install <github-owner>/herdr_simple_prompts
```

```toml
[[keys.command]]
key = "prefix+m"
type = "plugin_action"
command = "herdr.simple-prompts.toggle"
description = "Toggle Simple Prompts"
```

Explicitly state that installation invokes `cargo build --locked --release`
locally and that the project publishes no executable release artifacts.

- [ ] **Step 2: Add source-only CI**

`.github/workflows/ci.yml` runs on pushes and pull requests on both
`ubuntu-latest` and `macos-latest`, uses the stable Rust toolchain, and runs:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
```

It uploads no artifact and has `contents: read` permissions only.

- [ ] **Step 3: Run local source and manifest gates**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
herdr plugin link .
herdr plugin list --plugin herdr.simple-prompts --json
```

Expected: every Cargo command passes; Herdr lists the plugin enabled, compatible,
and without manifest warnings.

- [ ] **Step 4: Perform the live local Codex smoke matrix**

In a disposable Herdr workspace with current Codex:

1. toggle with `prefix+m`,
2. confirm existing prompt/final history excludes commentary/tools,
3. submit a normal prompt and confirm it appears before completion,
4. paste at least 200 KiB of multiline text and cancel before agent execution if
   it would create unwanted work,
5. paste an image and confirm `[Image #1]` in overlay and native composer,
6. interrupt a working request with Esc,
7. toggle closed and confirm the native pane/session is unchanged.

Record observed versions and outcomes in the final verification report, not in
the repository transcript fixtures.

- [ ] **Step 5: Perform the live local Claude smoke matrix**

Repeat the same flow with current Claude Code, including one tool-using prompt.
Confirm thinking, tool use/results, and progress do not appear, while the final
visible answer appears only after completion.

- [ ] **Step 6: Perform remote-attach image smoke when an SSH target is available**

Attach through Herdr remote mode, paste a local image, confirm the staged remote
path is forwarded as an attachment and not inserted as prompt text, then confirm
temporary staging remains Herdr-owned. If no SSH target is available, report
this single manual check as unverified; do not claim remote image verification.

- [ ] **Step 7: Reconcile documentation with observed behavior**

If any supported key, status field, transcript rule, or image behavior differs,
first add a failing regression test, correct the implementation, rerun all
gates, and then update both README and design spec so they match verified
behavior.

- [ ] **Step 8: Run final security and placeholder scans**

Run:

```bash
rg -n "https?://|curl|wget|reqwest|telemetry|analytics" src Cargo.toml herdr-plugin.toml
rg -n "TODO|TBD|FIXME|todo!|unimplemented!" . --glob '!target/**' --glob '!docs/superpowers/plans/**'
git status --short
```

Expected: no runtime network/download path, no unfinished implementation marker,
and only intended documentation/CI changes before the final commit.

- [ ] **Step 9: Commit the publication-ready repository**

```bash
git add README.md LICENSE .github herdr-plugin.toml Cargo.toml Cargo.lock docs src tests
git commit -m "document and verify simple prompts plugin"
```

## Plan self-review record

- **Spec coverage:** toggle, overlay layout, immediate prompts, Working state,
  native text/large paste/image input, Codex/Claude filtering, source-only build,
  privacy, failure states, persistence, local/remote behavior, tests, CI, and
  publication docs each map to Tasks 1-12.
- **Placeholder scan:** the plan contains no deferred implementation marker or
  unspecified error-handling step. The README screenshot is described rather
  than requiring a binary asset for version 0.1.
- **Type consistency:** `AgentIdentity`, `ConversationEvent`, `Message`, `Turn`,
  `Delivery`, `AppEvent`, `AppState`, `HerdrClient`, `AgentTransport`,
  `TranscriptAdapter`, and `TranscriptFollower` retain the same names and roles
  across tasks.
- **Workflow gates:** every coding task explicitly requires TDD, review, and
  verification checkpoints. The final documentation task requires TDD for any
  behavior correction and full-repository review/verification.

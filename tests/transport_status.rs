mod support;

use herdr_simple_prompts::agent::{AgentIdentity, AgentKind, AgentStatus};
use herdr_simple_prompts::herdr::HerdrClient;
use herdr_simple_prompts::status::extract_status;
use herdr_simple_prompts::transport::AgentTransport;
use serde_json::json;
use std::path::PathBuf;

fn identity(session: &str) -> AgentIdentity {
    AgentIdentity {
        pane_id: "w1:p1".into(),
        kind: AgentKind::Codex,
        session_id: session.into(),
        cwd: PathBuf::from("/repo"),
        status: AgentStatus::Working,
    }
}

fn agent_result(session: &str) -> serde_json::Value {
    agent_result_with_status(session, "working")
}

fn agent_result_with_status(session: &str, status: &str) -> serde_json::Value {
    json!({
        "type": "agent_info",
        "agent": {
            "pane_id": "w1:p1",
            "agent": "codex",
            "agent_status": status,
            "foreground_cwd": "/repo",
            "agent_session": {"source":"herdr:codex","agent":"codex","kind":"id","value":session}
        }
    })
}

fn clear_composer() -> serde_json::Value {
    json!({"read": {"text": concat!(
        "────────\n",
        "• answer\n",
        "────────\n",
        "› \u{1b}[2mWrite a prompt\u{1b}[0m\n",
        "gpt-5.6-sol xhigh · /repo · weekly 75% left",
    )}})
}

fn composer_with(text: &str) -> serde_json::Value {
    json!({"read": {"text": format!(concat!(
        "────────\n",
        "• answer\n",
        "────────\n",
        "› {}\n",
        "gpt-5.6-sol xhigh · /repo · weekly 75% left",
    ), text)}})
}

#[test]
fn refuses_to_send_after_native_session_changes() {
    let fake = support::ScriptedHerdr::start(vec![agent_result("s2")]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    let error = transport.submit("do it", 0).unwrap_err();

    assert!(error.to_string().contains("session changed"));
    assert_eq!(fake.requests().len(), 1);
}

#[test]
fn submit_preserves_full_large_paste_source() {
    let source = "line\n".repeat(1_000);
    let fake = support::ScriptedHerdr::start(vec![
        agent_result("s1"),
        clear_composer(),
        json!({"type": "agent_prompted"}),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    transport.submit(&source, 0).unwrap();

    let requests = fake.requests();
    assert_eq!(requests[1]["method"], "pane.read");
    assert_eq!(requests[1]["params"]["format"], "ansi");
    assert_eq!(requests[2]["method"], "agent.prompt");
    assert_eq!(requests[2]["params"]["text"], source);
    assert!(
        !requests[2]["params"]["text"]
            .as_str()
            .unwrap()
            .contains("Pasted Content")
    );
}

#[test]
fn submit_allows_exact_plugin_owned_image_markers() {
    let fake = support::ScriptedHerdr::start(vec![
        agent_result("s1"),
        composer_with("[Image #1] [Image #2]"),
        json!({"type": "agent_prompted"}),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    transport.submit("describe", 2).unwrap();

    let requests = fake.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2]["method"], "agent.prompt");
}

#[test]
fn submit_rejects_native_text_without_prompting_or_disclosing_it() {
    let native_secret = "private native draft";
    let fake =
        support::ScriptedHerdr::start(vec![agent_result("s1"), composer_with(native_secret)]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    let error = transport.submit("plugin draft", 0).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("native composer contains unsent input")
    );
    assert!(!error.to_string().contains(native_secret));
    assert_eq!(fake.requests().len(), 2);
}

#[test]
fn submit_rejects_attachment_count_mismatch_without_prompting() {
    let fake = support::ScriptedHerdr::start(vec![
        agent_result("s1"),
        composer_with("[Image #1] [Image #2]"),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    let error = transport.submit("plugin draft", 1).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("native composer contains unsent input")
    );
    assert_eq!(fake.requests().len(), 2);
}

#[test]
fn submit_rejects_unknown_or_unreadable_composer_without_prompting() {
    for responses in [
        vec![
            Ok(agent_result("s1")),
            Ok(json!({"read": {"text": "truncated"}})),
        ],
        vec![
            Ok(agent_result("s1")),
            Err(json!({"code":"temporary","message":"screen unavailable"})),
        ],
    ] {
        let fake = support::ScriptedHerdr::start_responses(responses);
        let client = HerdrClient::connect(fake.socket_path()).unwrap();
        let transport = AgentTransport::new(client, identity("s1"));

        let error = transport.submit("plugin draft", 0).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("cannot verify native composer is safe to submit")
        );
        assert_eq!(fake.requests().len(), 2);
    }
}

#[test]
fn interrupt_revalidates_then_sends_escape() {
    let fake =
        support::ScriptedHerdr::start(vec![agent_result("s1"), json!({"type":"agent_keys_sent"})]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    transport.interrupt().unwrap();

    let requests = fake.requests();
    assert_eq!(requests[1]["method"], "agent.send_keys");
    assert_eq!(
        requests[1]["params"],
        json!({"target":"w1:p1","keys":["esc"]})
    );
}

#[test]
fn status_extraction_omits_unproven_fields() {
    let codex = extract_status(
        AgentKind::Codex,
        "gpt-5.6-sol xhigh · ~/projects/demo · weekly 75% left · 203K used",
        PathBuf::from("/projects/demo"),
    );
    assert_eq!(codex.model.as_deref(), Some("gpt-5.6-sol xhigh"));
    assert_eq!(codex.usage.as_deref(), Some("weekly 75% left · 203K used"));

    let unknown = extract_status(
        AgentKind::Claude,
        "totally changed footer",
        PathBuf::from("/repo"),
    );
    assert!(unknown.model.is_none());
    assert!(unknown.usage.is_none());
    assert_eq!(unknown.cwd, PathBuf::from("/repo"));
}

#[test]
fn ansi_reads_revalidate_source_and_use_agent_specific_read_contracts() {
    let fake = support::ScriptedHerdr::start(vec![
        agent_result("s1"),
        json!({"read": {"text": "recent ansi"}}),
        agent_result("s1"),
        json!({"read": {"text": "visible ansi"}}),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    assert_eq!(transport.recent_unwrapped_ansi(240).unwrap(), "recent ansi");
    assert_eq!(transport.visible_source_ansi(8).unwrap(), "visible ansi");

    let requests = fake.requests();
    assert_eq!(requests[0]["method"], "agent.get");
    assert_eq!(requests[1]["method"], "agent.read");
    assert_eq!(requests[1]["params"]["source"], "recent_unwrapped");
    assert_eq!(requests[2]["method"], "agent.get");
    assert_eq!(requests[3]["method"], "pane.read");
    assert_eq!(requests[3]["params"]["format"], "ansi");
}

#[test]
fn ansi_reads_do_not_read_after_native_session_changes() {
    let fake = support::ScriptedHerdr::start(vec![agent_result("s2")]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    let error = transport.recent_unwrapped_ansi(240).unwrap_err();

    assert!(error.to_string().contains("session changed"));
    assert_eq!(fake.requests().len(), 1);
}

#[test]
fn interaction_text_revalidates_then_forwards_exactly_once_to_source_pane() {
    let fake = support::ScriptedHerdr::start(vec![
        agent_result_with_status("s1", "blocked"),
        json!({"type": "pane_input_sent"}),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    transport.forward_interaction_text("yes\nkeep all").unwrap();

    let requests = fake.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["method"], "agent.get");
    assert_eq!(requests[1]["method"], "pane.send_input");
    assert_eq!(
        requests[1]["params"],
        json!({"pane_id":"w1:p1","text":"yes\nkeep all","keys":[]})
    );
}

#[test]
fn interaction_key_revalidates_then_forwards_exactly_once_to_source_pane() {
    let fake = support::ScriptedHerdr::start(vec![
        agent_result_with_status("s1", "blocked"),
        json!({"type": "pane_input_sent"}),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    transport.forward_interaction_key("down").unwrap();

    let requests = fake.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["method"], "agent.get");
    assert_eq!(requests[1]["method"], "pane.send_input");
    assert_eq!(
        requests[1]["params"],
        json!({"pane_id":"w1:p1","keys":["down"]})
    );
}

#[test]
fn interaction_is_not_forwarded_after_native_session_changes() {
    let fake = support::ScriptedHerdr::start(vec![agent_result("s2")]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    let error = transport.forward_interaction_key("enter").unwrap_err();

    assert!(error.to_string().contains("session changed"));
    assert_eq!(fake.requests().len(), 1);
}

#[test]
fn interaction_text_is_not_forwarded_after_agent_leaves_blocked() {
    let fake = support::ScriptedHerdr::start(vec![agent_result_with_status("s1", "working")]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    let error = transport.forward_interaction_text("late text").unwrap_err();

    assert!(error.to_string().contains("no longer blocked"));
    let requests = fake.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "agent.get");
}

#[test]
fn interaction_key_is_not_forwarded_after_agent_leaves_blocked() {
    let fake = support::ScriptedHerdr::start(vec![agent_result_with_status("s1", "done")]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    let error = transport.forward_interaction_key("enter").unwrap_err();

    assert!(error.to_string().contains("no longer blocked"));
    let requests = fake.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "agent.get");
}

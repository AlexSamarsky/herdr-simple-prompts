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
    json!({
        "type": "agent_info",
        "agent": {
            "pane_id": "w1:p1",
            "agent": "codex",
            "agent_status": "working",
            "foreground_cwd": "/repo",
            "agent_session": {"source":"herdr:codex","agent":"codex","kind":"id","value":session}
        }
    })
}

#[test]
fn refuses_to_send_after_native_session_changes() {
    let fake = support::ScriptedHerdr::start(vec![agent_result("s2")]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    let error = transport.submit("do it").unwrap_err();

    assert!(error.to_string().contains("session changed"));
    assert_eq!(fake.requests().len(), 1);
}

#[test]
fn submit_preserves_full_large_paste_source() {
    let source = "line\n".repeat(1_000);
    let fake =
        support::ScriptedHerdr::start(vec![agent_result("s1"), json!({"type": "agent_prompted"})]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    transport.submit(&source).unwrap();

    let requests = fake.requests();
    assert_eq!(requests[1]["method"], "agent.prompt");
    assert_eq!(requests[1]["params"]["text"], source);
    assert!(
        !requests[1]["params"]["text"]
            .as_str()
            .unwrap()
            .contains("Pasted Content")
    );
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

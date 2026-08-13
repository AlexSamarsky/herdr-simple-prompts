mod support;

use herdr_simple_prompts::herdr::HerdrClient;
use std::io::Write;
use std::time::Duration;

#[test]
fn client_can_be_constructed_for_an_unavailable_socket_and_fails_on_call() {
    let socket = std::env::temp_dir().join(format!(
        "herdr-simple-prompts-missing-socket-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&socket);

    let client = HerdrClient::connect(&socket).unwrap();

    assert!(client.call("ping", serde_json::json!({})).is_err());
}

#[test]
fn call_sends_one_json_line_and_matches_response_id() {
    let fake = support::FakeHerdr::start(|request| {
        assert_eq!(request["method"], "agent.prompt");
        assert_eq!(
            request["params"],
            serde_json::json!({"target":"w1:p1","text":"hello"})
        );
        serde_json::json!({"id": request["id"], "result": {"type":"agent_prompted"}})
    });
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    let result = client
        .call(
            "agent.prompt",
            serde_json::json!({"target":"w1:p1","text":"hello"}),
        )
        .unwrap();

    assert_eq!(result["type"], "agent_prompted");
}

#[test]
fn call_preserves_structured_api_error() {
    let fake = support::FakeHerdr::error("not_found", "pane not found");
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    let error = client
        .call("pane.get", serde_json::json!({"pane_id":"missing"}))
        .unwrap_err();

    assert_eq!(error.api_code(), Some("not_found"));
    assert!(!error.is_pane_not_found());
    assert!(error.to_string().contains("pane not found"));
}

#[test]
fn pane_not_found_is_classified_from_the_exact_herdr_api_code() {
    let fake = support::FakeHerdr::error("pane_not_found", "pane not found");
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    let error = client
        .call("pane.get", serde_json::json!({"pane_id":"missing"}))
        .unwrap_err();

    assert!(error.is_pane_not_found());
}

#[test]
fn zoomed_open_targets_the_exact_source_pane_without_workspace_context() {
    let fake = support::FakeHerdr::start(|request| {
        assert_eq!(request["method"], "plugin.pane.open");
        assert_eq!(request["params"]["placement"], "zoomed");
        assert_eq!(request["params"]["target_pane_id"], "w1:p1");
        assert!(request["params"].get("workspace_id").is_none());
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

    let overlay = client.plugin_pane_open_targeted("w1:p1").unwrap();

    assert_eq!(overlay, "w1:p9");
}

#[test]
fn accepts_partial_response_writes_and_unknown_fields() {
    let fake = support::FakeHerdr::start_raw(|request, stream| {
        let id = request["id"].as_str().unwrap();
        write!(stream, "{{\"id\":\"{id}\",").unwrap();
        stream.flush().unwrap();
        writeln!(
            stream,
            "\"result\":{{\"type\":\"pong\",\"future_field\":42}}}}"
        )
        .unwrap();
    });
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    let result = client.call("ping", serde_json::json!({})).unwrap();

    assert_eq!(result["future_field"], 42);
}

#[test]
fn rejects_malformed_json_mismatched_ids_eof_and_oversized_lines() {
    for body in [
        b"not json\n".to_vec(),
        b"{\"id\":\"wrong\",\"result\":{}}\n".to_vec(),
        Vec::new(),
        vec![b'x'; 4 * 1024 * 1024 + 2],
    ] {
        let fake = support::FakeHerdr::start_raw(move |_request, stream| {
            stream.write_all(&body).unwrap();
        });
        let client = HerdrClient::connect(fake.socket_path()).unwrap();
        assert!(client.call("ping", serde_json::json!({})).is_err());
    }
}

#[test]
fn read_timeout_prevents_a_stalled_peer_from_hanging_the_plugin() {
    let fake = support::FakeHerdr::start_raw(|_request, _stream| {
        std::thread::sleep(Duration::from_millis(100));
    });
    let client = HerdrClient::connect(fake.socket_path())
        .unwrap()
        .with_timeout(Duration::from_millis(20));

    let started = std::time::Instant::now();
    assert!(client.call("ping", serde_json::json!({})).is_err());
    assert!(started.elapsed() < Duration::from_millis(90));
}

#[test]
fn typed_ansi_reads_send_exact_contracts_and_extract_text() {
    let fake = support::ScriptedHerdr::start(vec![
        serde_json::json!({"read": {"text": "\u{1b}[32manswer\u{1b}[0m"}}),
        serde_json::json!({"read": {"text": "visible text"}}),
        serde_json::json!({"read": {"text": "\u{1b}[33mvisible\u{1b}[0m"}}),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    assert_eq!(
        client
            .agent_read_recent_unwrapped_ansi("w1:p1", 240)
            .unwrap(),
        "\u{1b}[32manswer\u{1b}[0m"
    );
    assert_eq!(
        client.pane_read_visible_text("w1:p1", 8).unwrap(),
        "visible text"
    );
    assert_eq!(
        client.pane_read_visible_ansi("w1:p1", 8).unwrap(),
        "\u{1b}[33mvisible\u{1b}[0m"
    );

    let requests = fake.requests();
    assert_eq!(requests[0]["method"], "agent.read");
    assert_eq!(
        requests[0]["params"],
        serde_json::json!({
            "target": "w1:p1",
            "source": "recent_unwrapped",
            "lines": 240,
            "format": "ansi",
            "strip_ansi": false
        })
    );
    assert_eq!(requests[1]["method"], "pane.read");
    assert_eq!(
        requests[1]["params"],
        serde_json::json!({
            "pane_id": "w1:p1",
            "source": "visible",
            "lines": 8,
            "format": "text",
            "strip_ansi": true
        })
    );
    assert_eq!(requests[2]["method"], "pane.read");
    assert_eq!(
        requests[2]["params"],
        serde_json::json!({
            "pane_id": "w1:p1",
            "source": "visible",
            "lines": 8,
            "format": "ansi",
            "strip_ansi": false
        })
    );
}

#[test]
fn typed_read_reports_structured_protocol_error_when_text_is_absent() {
    let fake = support::ScriptedHerdr::start(vec![serde_json::json!({"read": {}})]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    let error = client
        .agent_read_recent_unwrapped_ansi("w1:p1", 240)
        .unwrap_err();

    assert!(matches!(
        error,
        herdr_simple_prompts::herdr::HerdrError::Protocol(_)
    ));
    assert!(error.to_string().contains("read text"));
}

#[test]
fn pane_input_uses_exact_text_and_key_contracts() {
    let fake = support::ScriptedHerdr::start(vec![
        serde_json::json!({"type": "pane_input_sent"}),
        serde_json::json!({"type": "pane_input_sent"}),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    client
        .pane_send_input("w1:p1", Some("full\npaste"), &[])
        .unwrap();
    client
        .pane_send_input("w1:p1", None, &["shift+tab"])
        .unwrap();

    let requests = fake.requests();
    assert_eq!(requests[0]["method"], "pane.send_input");
    assert_eq!(
        requests[0]["params"],
        serde_json::json!({"pane_id":"w1:p1","text":"full\npaste","keys":[]})
    );
    assert_eq!(requests[1]["method"], "pane.send_input");
    assert_eq!(
        requests[1]["params"],
        serde_json::json!({"pane_id":"w1:p1","keys":["shift+tab"]})
    );
}

#[test]
fn wait_for_pane_closed_sends_exact_contract_and_accepts_exact_match() {
    let fake = support::ScriptedHerdr::start(vec![serde_json::json!({
        "type": "wait_matched",
        "event": {
            "event": "pane_closed",
            "data": {"pane_id": "w1:p1"}
        }
    })]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    assert!(
        client
            .wait_for_pane_closed("w1:p1", Duration::from_millis(1_000))
            .unwrap()
    );

    let requests = fake.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "events.wait");
    assert_eq!(
        requests[0]["params"],
        serde_json::json!({
            "match_event": {"event": "pane_closed", "pane_id": "w1:p1"},
            "timeout_ms": 1_000
        })
    );
}

#[test]
fn wait_for_pane_closed_treats_only_wait_timeout_as_no_match() {
    let fake = support::ScriptedHerdr::start_responses(vec![Err(serde_json::json!({
        "code": "timeout",
        "message": "event wait timed out"
    }))]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    assert!(
        !client
            .wait_for_pane_closed("w1:p1", Duration::from_millis(1_000))
            .unwrap()
    );
}

#[test]
fn wait_for_pane_closed_rejects_mismatched_success_envelopes() {
    for response in [
        serde_json::json!({
            "type": "wait_matched",
            "event": {"event": "pane_closed", "data": {"pane_id": "w1:p2"}}
        }),
        serde_json::json!({
            "type": "wait_matched",
            "event": {"event": "pane_focused", "data": {"pane_id": "w1:p1"}}
        }),
        serde_json::json!({
            "type": "pane_closed",
            "event": {"event": "pane_closed", "data": {"pane_id": "w1:p1"}}
        }),
    ] {
        let fake = support::ScriptedHerdr::start(vec![response]);
        let client = HerdrClient::connect(fake.socket_path()).unwrap();

        let error = client
            .wait_for_pane_closed("w1:p1", Duration::from_millis(1_000))
            .unwrap_err();

        assert!(matches!(
            error,
            herdr_simple_prompts::herdr::HerdrError::Protocol(_)
        ));
    }
}

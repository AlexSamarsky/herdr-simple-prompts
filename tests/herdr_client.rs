mod support;

use herdr_simple_prompts::herdr::HerdrClient;
use std::io::Write;
use std::time::Duration;

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
    assert!(error.to_string().contains("pane not found"));
}

#[test]
fn overlay_open_uses_active_pane_and_passes_source_only_to_child() {
    let fake = support::FakeHerdr::start(|request| {
        assert_eq!(request["method"], "plugin.pane.open");
        assert_eq!(request["params"]["placement"], "overlay");
        assert!(request["params"].get("target_pane_id").is_none());
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

    let overlay = client.plugin_pane_open("w1:p1").unwrap();

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

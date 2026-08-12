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

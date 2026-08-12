mod support;

use herdr_simple_prompts::herdr::HerdrClient;

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

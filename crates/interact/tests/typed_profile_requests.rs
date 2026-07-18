use std::fs;

#[test]
fn profile_commands_do_not_construct_raw_json_rpc() {
    let source = fs::read_to_string("src/tui/app/submit.rs").unwrap();
    assert!(source.contains("ClientRpcRequest::AgentProfileList"));
    assert!(source.contains("ClientRpcRequest::agent_profile_set"));
    assert!(!source.contains("serde_json::json!({\"jsonrpc\""));
    assert!(!source.contains("\"method\": \"agent.profile.list\""));
    assert!(!source.contains("\"method\": \"agent.profile.set\""));
}

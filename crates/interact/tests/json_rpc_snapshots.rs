use std::path::Path;

use fabric::protocol::client::{ClientRpcRequest, TransientApprovalDecision};
use serde::Serialize;

#[derive(Serialize)]
struct RpcPair {
    name: &'static str,
    request: serde_json::Value,
    response: serde_json::Value,
}

fn pair(
    name: &'static str,
    request: ClientRpcRequest,
    id: u64,
    response: serde_json::Value,
) -> RpcPair {
    RpcPair {
        name,
        request: request.to_json_rpc(Some(id)).unwrap(),
        response,
    }
}

#[test]
fn typed_json_rpc_request_response_contract_snapshot() {
    let workspace = fabric::WorkspacePolicy::from_resolved_roots(
        "/workspace/project".into(),
        vec!["/workspace/shared".into()],
    )
    .unwrap();
    let pairs = vec![
        pair(
            "chat_success",
            ClientRpcRequest::chat("inspect", &workspace),
            1,
            serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"accepted":true}}),
        ),
        pair(
            "profile_list_success",
            ClientRpcRequest::AgentProfileList,
            2,
            serde_json::json!({"jsonrpc":"2.0","id":2,"result":{"profiles":[{"name":"safe"}]}}),
        ),
        pair(
            "profile_set_success",
            ClientRpcRequest::agent_profile_set("safe"),
            3,
            serde_json::json!({"jsonrpc":"2.0","id":3,"result":{"previous":"admin","current":"safe"}}),
        ),
        pair(
            "profile_set_denied",
            ClientRpcRequest::agent_profile_set("admin"),
            4,
            serde_json::json!({"jsonrpc":"2.0","id":4,"error":{"code":-32000,"message":"profile switch denied"}}),
        ),
        pair(
            "approval_response",
            ClientRpcRequest::approval_response(
                "approval-1",
                TransientApprovalDecision::ApproveForSession,
            ),
            5,
            serde_json::json!({"jsonrpc":"2.0","id":5,"result":{"accepted":true}}),
        ),
    ];
    let rendered = serde_json::to_string_pretty(&pairs).unwrap() + "\n";
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots/json_rpc_request_response.snap");
    if std::env::var_os("UPDATE_JSON_RPC_SNAPSHOTS").is_some() {
        std::fs::write(&path, &rendered).unwrap();
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), rendered);
}

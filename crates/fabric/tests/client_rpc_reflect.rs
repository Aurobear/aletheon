use fabric::protocol::client::{ClientRpcRequest, SessionParams};

#[test]
fn reflect_now_for_serializes_the_active_session() {
    let request = ClientRpcRequest::ReflectNowFor(SessionParams {
        session_id: "session-42".to_string(),
    })
    .to_json_rpc(Some(7))
    .expect("request serializes");

    assert_eq!(request["method"], "reflect_now");
    assert_eq!(request["params"]["session_id"], "session-42");
}

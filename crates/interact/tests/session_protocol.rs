use fabric::{ItemId, ItemPayload, ItemRecord, SessionId, TurnId, SESSION_SCHEMA_VERSION};
use interact::tui::session_protocol::*;

#[test]
fn four_session_requests_have_typed_stable_fields() {
    let session_id = SessionId("s1".into());
    let requests = [
        SessionRpcRequest::Resume(ResumeParams {
            session_id: session_id.clone(),
        }),
        SessionRpcRequest::Fork(ForkParams {
            session_id: session_id.clone(),
            through_sequence: 3,
        }),
        SessionRpcRequest::Interrupt(InterruptParams {
            session_id: session_id.clone(),
        }),
        SessionRpcRequest::Replay(ReplayParams {
            session_id,
            after_sequence: Some(2),
        }),
    ];
    let methods = [
        "session.resume",
        "session.fork",
        "session.interrupt",
        "session.replay",
    ];
    for (request, method) in requests.iter().zip(methods) {
        let json = request.to_json(7);
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["method"], method);
        assert_eq!(json["params"]["session_id"], "s1");
    }
}

#[test]
fn item_notification_uses_fabric_shape() {
    let notification = SessionClientNotification::item_appended(ItemRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: ItemId::new(),
        session_id: SessionId("s1".into()),
        turn_id: TurnId::new(),
        sequence: 1,
        created_at_ms: 1,
        payload: ItemPayload::UserMessage {
            content: "hello".into(),
        },
    })
    .to_json();
    assert_eq!(notification["method"], "session.notification");
    assert_eq!(notification["params"]["type"], "item_appended");
    assert_eq!(
        notification["params"]["data"]["item"]["payload"]["type"],
        "user_message"
    );
}

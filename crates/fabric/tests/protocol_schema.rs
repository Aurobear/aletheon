use fabric::protocol::client::{
    client_schema, ClientEvent, ClientMessage, ClientRequest, EventCursor, EventSubscription,
    SnapshotRequest, CLIENT_PROTOCOL_VERSION,
};
use fabric::SessionId;

#[test]
fn schema_is_versioned_and_exposes_snapshot_reconnect_and_incremental_subscription() {
    let schema = client_schema();
    let encoded = serde_json::to_string(&schema).unwrap();
    for contract in [
        "protocol_version",
        "snapshot",
        "reconnected",
        "subscribe",
        "after",
    ] {
        assert!(encoded.contains(contract), "schema missing {contract}");
    }
}

#[test]
fn unknown_versions_fail_and_optional_forward_fields_are_retained() {
    let value = serde_json::json!({
        "protocol_version": 9,
        "payload": {"type":"failed", "data":{"message":"nope"}},
        "future_trace": {"enabled": true}
    });
    let decoded: ClientMessage<ClientEvent> = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.extensions["future_trace"]["enabled"], true);
    assert_eq!(decoded.into_v1().unwrap_err().actual, 9);
}

#[test]
fn typed_requests_round_trip_at_the_supported_version() {
    let session_id = SessionId("session-1".into());
    for request in [
        ClientRequest::Snapshot(SnapshotRequest {
            session_id: session_id.clone(),
        }),
        ClientRequest::Subscribe(EventSubscription {
            session_id,
            after: EventCursor {
                sequence: 41,
                event_id: Some("event-41".into()),
            },
        }),
    ] {
        let wire = ClientMessage::v1(request.clone());
        assert_eq!(wire.protocol_version, CLIENT_PROTOCOL_VERSION);
        let json = serde_json::to_value(&wire).unwrap();
        let decoded: ClientMessage<ClientRequest> = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.into_v1().unwrap(), request);
    }
}

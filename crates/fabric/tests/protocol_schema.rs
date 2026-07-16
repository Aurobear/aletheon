use fabric::protocol::client::{
    client_schema, negotiate_protocol_version, ClientCapabilities, ClientEvent, ClientMessage,
    ClientRequest, EventCursor, EventSubscription, InitializeParams, InitializedResult,
    SnapshotRequest, CLIENT_PROTOCOL_VERSION,
};
use fabric::{
    ConnectionId, LocalOsPrincipal, PrincipalId, SessionId, TurnStop, TurnTerminalStatus,
};

#[test]
fn schema_is_versioned_and_exposes_snapshot_reconnect_and_incremental_subscription() {
    let schema = client_schema();
    let encoded = serde_json::to_string(&schema).unwrap();
    for contract in [
        "protocol_version",
        "initialize",
        "initialize_response",
        "initialized",
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

#[test]
fn initialize_has_version_and_capabilities_but_no_uid() {
    let value = serde_json::to_value(ClientRequest::Initialize(InitializeParams {
        client_version: "0.1.0".into(),
        protocol_versions: vec![1],
        capabilities: ClientCapabilities {
            item_events: true,
            cursors: true,
        },
    }))
    .unwrap();
    assert_eq!(value["type"], "initialize");
    assert_eq!(value["data"]["protocol_versions"], serde_json::json!([1]));
    assert!(value.to_string().find("uid").is_none());
}

#[test]
fn initialized_is_a_distinct_client_message() {
    assert_eq!(
        serde_json::to_value(ClientRequest::Initialized).unwrap()["type"],
        "initialized"
    );
}

#[test]
fn initialize_response_contains_only_the_effective_server_identity() {
    let value = serde_json::to_value(ClientEvent::InitializeResponse(InitializedResult {
        protocol_version: CLIENT_PROTOCOL_VERSION,
        server_capabilities: ClientCapabilities {
            item_events: true,
            cursors: true,
        },
        connection_id: ConnectionId::new(),
        principal_id: PrincipalId::local_uid(1001),
        os_principal: LocalOsPrincipal {
            uid: 1001,
            gid: 1001,
        },
        runtime_version: "0.1.0".into(),
    }))
    .unwrap();
    assert_eq!(value["type"], "initialize_response");
    assert_eq!(value["data"]["protocol_version"], CLIENT_PROTOCOL_VERSION);
    assert_eq!(value["data"]["principal_id"], "local-uid:1001");
    assert_eq!(value["data"]["os_principal"]["uid"], 1001);
}

#[test]
fn protocol_negotiation_selects_the_highest_shared_version() {
    assert_eq!(negotiate_protocol_version(&[0, 1, 2]).unwrap(), 1);
    assert_eq!(
        negotiate_protocol_version(&[]).unwrap_err().expected,
        CLIENT_PROTOCOL_VERSION
    );
    assert_eq!(negotiate_protocol_version(&[2]).unwrap_err().actual, 2);
}

#[test]
fn internal_stops_map_to_one_external_terminal_status() {
    assert_eq!(
        TurnTerminalStatus::from(TurnStop::Completed),
        TurnTerminalStatus::Completed
    );
    assert_eq!(
        TurnTerminalStatus::from(TurnStop::Cancelled),
        TurnTerminalStatus::Interrupted
    );
    assert_eq!(
        TurnTerminalStatus::from(TurnStop::Blocked),
        TurnTerminalStatus::Failed
    );
    assert_eq!(
        TurnTerminalStatus::from(TurnStop::Failed),
        TurnTerminalStatus::Failed
    );
}

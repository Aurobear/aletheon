use fabric::*;

fn item() -> ItemRecord {
    ItemRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: ItemId::new(),
        session_id: SessionId("s1".into()),
        turn_id: TurnId::new(),
        sequence: 1,
        created_at_ms: 42,
        payload: ItemPayload::ToolResult {
            call_id: "c1".into(),
            content: "done".into(),
            is_error: false,
            permit_id: None,
            audit_id: None,
        },
    }
}

#[test]
fn lifecycle_contract_round_trips_with_explicit_version_and_tags() {
    let values = [
        ItemPayload::UserMessage {
            content: "u".into(),
        },
        ItemPayload::AssistantMessage {
            content: "a".into(),
        },
        ItemPayload::ToolCall {
            call_id: "c".into(),
            name: "n".into(),
            input: serde_json::json!({}),
        },
        item().payload,
        ItemPayload::SystemNotice {
            content: "s".into(),
        },
    ];
    for payload in values {
        let json = serde_json::to_value(&payload).unwrap();
        assert!(json["type"].is_string());
        assert_eq!(
            serde_json::from_value::<ItemPayload>(json).unwrap(),
            payload
        );
    }
    let notification = SessionNotification::ItemAppended {
        schema_version: SESSION_SCHEMA_VERSION,
        item: item(),
    };
    let json = serde_json::to_value(&notification).unwrap();
    assert_eq!(json["type"], "item_appended");
    assert_eq!(json["data"]["schema_version"], 1);
    assert_eq!(
        serde_json::from_value::<SessionNotification>(json).unwrap(),
        notification
    );
}

#[test]
fn append_store_is_object_safe() {
    fn accepts(_: Option<&dyn SessionAppendStore>) {}
    accepts(None);
}

#[test]
fn checked_in_schema_matches_exporter_shape() {
    let checked: serde_json::Value =
        serde_json::from_str(include_str!("../../../schemas/session-v1.schema.json")).unwrap();
    let generated = serde_json::to_value(schemars::schema_for!(SessionProtocolV1)).unwrap();
    assert_eq!(checked, generated);
}

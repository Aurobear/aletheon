use fabric::ipc::envelope_v2::Target;
use fabric::{BusConfig, CommunicationBus, EnvelopeV2, EnvelopeV2Delivery, NamespaceId, SchemaId};

fn envelope(schema: &str, value: u64) -> EnvelopeV2 {
    EnvelopeV2::new(
        SchemaId(schema.into()),
        Target("test".into()),
        Target("broadcast".into()),
        EnvelopeV2Delivery::FanOut,
        NamespaceId("test".into()),
        serde_json::json!({"value": value}),
    )
}

#[tokio::test]
async fn canonical_delivery_is_schema_filtered() {
    let bus = CommunicationBus::new();
    let mut turns = bus.subscribe_envelope_v2(SchemaId(SchemaId::TURN_EVENT_V1.into()));
    let mut signals = bus.subscribe_envelope_v2(SchemaId(SchemaId::PROCESS_SIGNAL_V1.into()));
    bus.publish_envelope_v2(envelope(SchemaId::TURN_EVENT_V1, 1))
        .await
        .unwrap();

    assert_eq!(turns.recv().await.unwrap().payload["value"], 1);
    assert!(signals.try_recv().is_err());
}

#[tokio::test]
async fn unknown_schema_is_rejected_and_bounded_channels_report_lag() {
    let bus = CommunicationBus::with_config(BusConfig {
        log_capacity: 1,
        mailbox_buffer: 1,
        topic_buffer: 1,
    });
    assert!(bus
        .publish_envelope_v2(envelope("aletheon.unknown/v9", 0))
        .await
        .unwrap_err()
        .to_string()
        .contains("unsupported schema"));

    let mut receiver = bus.subscribe_envelope_v2(SchemaId(SchemaId::TURN_EVENT_V1.into()));
    bus.publish_envelope_v2(envelope(SchemaId::TURN_EVENT_V1, 1))
        .await
        .unwrap();
    bus.publish_envelope_v2(envelope(SchemaId::TURN_EVENT_V1, 2))
        .await
        .unwrap();
    assert!(matches!(
        receiver.recv().await,
        Err(tokio::sync::broadcast::error::RecvError::Lagged(1))
    ));
    assert_eq!(receiver.recv().await.unwrap().payload["value"], 2);
}

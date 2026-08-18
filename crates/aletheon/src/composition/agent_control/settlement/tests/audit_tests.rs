use super::*;
use runtime::event_spine::{EventPosition, SpineEvent, TreeSequence, UnsequencedEvent};
use std::sync::Mutex as StdMutex;

#[derive(Default)]
struct RecordingSpine {
    events: StdMutex<Vec<UnsequencedEvent>>,
}

impl EventSpine for RecordingSpine {
    fn append(&self, event: UnsequencedEvent) -> anyhow::Result<SpineEvent> {
        self.events.lock().unwrap().push(event.clone());
        Ok(SpineEvent {
            position: EventPosition {
                tree_id: event.tree_id,
                event_id: event.event_id,
                parent: event.parent,
                sequence: TreeSequence(1),
            },
            identity: event.identity,
            schema: event.envelope.schema.clone(),
            visibility: event.visibility,
            envelope: event.envelope,
            payload: event.payload,
        })
    }
}

#[tokio::test]
async fn agent_settlement_evidence_uses_its_domain_schema() {
    let spine = Arc::new(RecordingSpine::default());
    SpineSettlementEvidenceSink::new(
        spine.clone(),
        "root-session".into(),
        "child-agent".into(),
        OperationId::new(),
    )
    .record(SettlementEvidence::Phase(SettlementPhase::Terminal))
    .await
    .unwrap();

    let events = spine.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].envelope.schema.0,
        ::contracts::SchemaId::EVENT_AGENT_SETTLEMENT_V1
    );
    assert_ne!(
        events[0].envelope.schema.0,
        ::contracts::SchemaId::TURN_EVENT_V1
    );
}

#[tokio::test]
async fn u_resume_005_stale_generation_receipt_is_rejected_with_durable_audit_event() {
    let spine = Arc::new(RecordingSpine::default());
    let evidence = Arc::new(SpineSettlementEvidenceSink::new(
        spine.clone(),
        "root-session".into(),
        "child-agent".into(),
        OperationId::new(),
    ));
    let engine = SettlementEngine::new(
        Arc::new(InMemorySettlementReceiptStore::default()),
        Arc::new(FakeResources::allowing()),
        Arc::new(FakeLeases::default()),
        evidence,
    )
    .with_generation("current-generation");
    let mut stale = request();
    stale.generation = "old-generation".into();

    let error = engine.settle(stale, Vec::new()).await.unwrap_err();
    assert!(error.message.contains("stale daemon generation"));
    let events = spine.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].envelope.schema.0,
        ::contracts::SchemaId::EVENT_AGENT_SETTLEMENT_V1
    );
    let runtime::EventPayload::Inline { value } = &events[0].payload else {
        panic!("generation rejection audit must be inline")
    };
    assert_eq!(value["kind"], "agent.settlement.generation_rejected");
    assert_eq!(value["detail"]["expected"], "current-generation");
    assert_eq!(value["detail"]["received"], "old-generation");
}

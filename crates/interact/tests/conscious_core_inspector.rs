use fabric::dasein::SelfVersion;
use fabric::{
    AgoraSpaceId, BroadcastEpoch, CandidateDisposition, ConsciousCoreSnapshot, ContentId,
    InspectorProcessorAck, ProcessorHealth, ProcessorId, SalienceVector,
};

#[test]
fn rendering_is_sanitized_and_states_indicator_limitations_and_degradation() {
    let snapshot = ConsciousCoreSnapshot {
        space: AgoraSpaceId("root".into()),
        epoch: BroadcastEpoch(9),
        dispositions: vec![CandidateDisposition {
            id: ContentId(uuid::Uuid::from_u128(1)),
            source_kind: "external_memory".into(),
            content_schema: "recalled_experience/v1".into(),
            salience: SalienceVector {
                urgency: 0.1,
                goal_relevance: 0.7,
                self_relevance: 0.2,
                novelty: 0.3,
                confidence: 0.5,
                prediction_error: 0.0,
                affect_intensity: 0.0,
                social_relevance: 0.0,
            },
            winner: true,
            coalition_member: false,
            visibility: "private".into(),
        }],
        acknowledgements: vec![InspectorProcessorAck {
            processor: ProcessorId("mnemosyne".into()),
            health: ProcessorHealth::Degraded,
            accepted_count: 1,
            rejected_count: 0,
            degraded_reason: Some("supplemental source timed out".into()),
        }],
        dasein_version: SelfVersion(12),
        indicator_limitations: vec![
            "Functional indicators are not proof of consciousness.".into(),
            "Private payloads are excluded.".into(),
        ],
    };
    snapshot.validate().unwrap();
    let rendered = interact::tui::conscious_core::render_snapshot(&snapshot).join("\n");
    assert!(rendered.contains("winner") && rendered.contains("degraded"));
    assert!(rendered.contains("limitation:"));
    assert!(!rendered.contains("hidden chain of thought"));
}

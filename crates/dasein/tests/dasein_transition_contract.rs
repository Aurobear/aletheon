use dasein::dasein::DaseinModule;
use fabric::dasein::{
    CareActionKind, ExperienceProvenance, ExperienceSource, InterpretedExperience, OutcomeStatus,
    SelfEventId, SelfSignal, SelfTransitionRequest, SelfVersion, Stimmung, TemporalEventKind,
};
use fabric::WallTime;
use std::sync::Arc;

fn module() -> DaseinModule {
    DaseinModule::new(Arc::new(kernel::chronos::TestClock::default())).0
}

fn request(
    event_id: SelfEventId,
    expected_version: u64,
    content: InterpretedExperience,
) -> SelfTransitionRequest {
    SelfTransitionRequest {
        event_id,
        source: ExperienceSource::Runtime,
        observed_at: WallTime(42),
        content,
        provenance: ExperienceProvenance {
            producer: "dasein-transition-test".into(),
            session_id: None,
            turn_id: None,
            source_ref: Some("test-case".into()),
        },
        expected_version: SelfVersion(expected_version),
    }
}

#[tokio::test]
async fn reducer_accepts_once_and_duplicate_returns_same_receipt() {
    let module = module();
    let event_id = SelfEventId::new();
    let transition = request(
        event_id,
        0,
        InterpretedExperience::Lived {
            semantic: "accepted experience".into(),
            action: None,
            perception: None,
        },
    );

    let first = module.transition(transition.clone()).await.unwrap();
    let duplicate = module.transition(transition).await.unwrap();

    assert_eq!(first, duplicate);
    assert_eq!(first.previous_version, SelfVersion(0));
    assert_eq!(first.current_version, SelfVersion(1));
    assert_eq!(module.self_version().await, SelfVersion(1));
    assert_eq!(module.temporality().current_position().0, 1);
    assert_eq!(module.narrative_reference_count().await, 1);
}

#[tokio::test]
async fn reducer_rejects_stale_version_without_mutation() {
    let module = module();
    module
        .transition(request(
            SelfEventId::new(),
            0,
            InterpretedExperience::Lived {
                semantic: "first".into(),
                action: None,
                perception: None,
            },
        ))
        .await
        .unwrap();

    let error = module
        .transition(request(
            SelfEventId::new(),
            0,
            InterpretedExperience::Lived {
                semantic: "stale".into(),
                action: None,
                perception: None,
            },
        ))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("version conflict"));
    assert_eq!(module.self_version().await, SelfVersion(1));
    assert_eq!(module.temporality().current_position().0, 1);
    assert_eq!(module.narrative_reference_count().await, 1);
}

#[tokio::test]
async fn reducer_structured_outcome_controls_mood_without_keywords() {
    let module = module();
    let receipt = module
        .transition(request(
            SelfEventId::new(),
            0,
            InterpretedExperience::Outcome {
                summary: "plain neutral words".into(),
                status: OutcomeStatus::Failed,
            },
        ))
        .await
        .unwrap();

    assert!(matches!(module.mood(), Stimmung::Geknickt { .. }));
    assert!(receipt
        .emitted
        .iter()
        .any(|signal| matches!(signal, SelfSignal::MoodChanged { .. })));
    assert_eq!(module.temporality().current_position().0, 1);
}

#[tokio::test]
async fn reducer_explicitly_handles_reflective_variants() {
    let module = module();
    module
        .transition(request(
            SelfEventId::new(),
            0,
            InterpretedExperience::KnowledgeAsserted {
                assertions: vec!["fixed identity".into()],
                confidence: 0.5,
            },
        ))
        .await
        .unwrap();

    let variants = [
        InterpretedExperience::KnowledgeAsserted {
            assertions: vec!["new understanding".into()],
            confidence: 0.8,
        },
        InterpretedExperience::NegationCompleted {
            target: "fixed identity".into(),
            new_possibilities: vec!["adaptive identity".into()],
        },
        InterpretedExperience::MoodObserved {
            mood: Stimmung::Neugier {
                curiosity_about: "the next test".into(),
            },
            reason: "structured observation".into(),
        },
        InterpretedExperience::TemporalSignal {
            kind: TemporalEventKind::ProtentionSurprised,
            content: "unexpected result".into(),
        },
        InterpretedExperience::ScheduledReflection,
    ];

    for (index, content) in variants.into_iter().enumerate() {
        module
            .transition(request(SelfEventId::new(), index as u64 + 1, content))
            .await
            .unwrap();
    }

    assert_eq!(module.self_version().await, SelfVersion(6));
    assert_eq!(module.temporality().current_position().0, 0);
    assert_eq!(module.narrative_reference_count().await, 6);
    let self_snapshot = module.self_model().to_snapshot();
    assert!(self_snapshot
        .current_assertions
        .iter()
        .any(|assertion| assertion.content == "new understanding"));
    assert!(self_snapshot
        .possibilities
        .iter()
        .any(|possibility| possibility.content == "adaptive identity"));
}

#[tokio::test]
async fn scheduled_reflection_emits_care_decision() {
    // R1 (conscious-core plan): CareStructure::determine_action() must be
    // consumed in the production reflection path and emitted as a signal, not
    // computed and discarded. The emitted signal is what flows into Agora as a
    // candidate, giving "care" a behavioral effect.
    let module = module();
    let receipt = module
        .transition(request(
            SelfEventId::new(),
            0,
            InterpretedExperience::ScheduledReflection,
        ))
        .await
        .unwrap();

    let care_decision = receipt
        .emitted
        .iter()
        .find(|signal| matches!(signal, SelfSignal::CareDecision { .. }));
    assert!(
        care_decision.is_some(),
        "scheduled reflection must emit a CareDecision, got {:?}",
        receipt.emitted
    );
    // With no urgent concerns or chosen projection, the default care action is Wait.
    assert!(matches!(
        care_decision.unwrap(),
        SelfSignal::CareDecision {
            action: CareActionKind::Wait,
            ..
        }
    ));
}

#[tokio::test]
async fn reducer_readiness_change_is_compare_and_set() {
    let module = module();
    module
        .transition(request(
            SelfEventId::new(),
            0,
            InterpretedExperience::WorldEntityObserved {
                entity_id: "compiler".into(),
                what_it_is: "build tool".into(),
                for_the_sake_of: Vec::new(),
                readiness: fabric::dasein::ReadinessState::ReadyToHand,
            },
        ))
        .await
        .unwrap();

    module
        .transition(request(
            SelfEventId::new(),
            1,
            InterpretedExperience::ReadinessChanged {
                entity_id: "compiler".into(),
                old_state: fabric::dasein::ReadinessState::ReadyToHand,
                new_state: fabric::dasein::ReadinessState::PresentAtHand,
            },
        ))
        .await
        .unwrap();

    let error = module
        .transition(request(
            SelfEventId::new(),
            2,
            InterpretedExperience::ReadinessChanged {
                entity_id: "compiler".into(),
                old_state: fabric::dasein::ReadinessState::ReadyToHand,
                new_state: fabric::dasein::ReadinessState::Unavailable,
            },
        ))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("readiness conflict"));
    assert_eq!(module.self_version().await, SelfVersion(2));
    let snapshot = module.world().to_snapshot();
    assert_eq!(snapshot.present_at_hand.len(), 1);
    assert!(snapshot.unavailable.is_empty());
}

#[tokio::test]
async fn bridge_compatibility_event_reaches_reducer() {
    let module = module();
    let receipt = fabric::dasein::DaseinOps::handle_event(
        &module,
        fabric::dasein::DaseinEvent::KnowledgeAsserted {
            assertions: vec!["bridge fact".into()],
            confidence: 0.7,
        },
    )
    .await
    .unwrap();

    assert_eq!(receipt.current_version, SelfVersion(1));
    assert_eq!(module.self_model().assertion_count(), 1);
}

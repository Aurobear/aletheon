//! Integration test for the self-evolution EventBus loop.
//!
//! Uses MockLlmProvider to verify the full event flow without real API calls.
//! Tests cover:
//! 1. Reflection emitted after tool observation
//! 2. Evolution triggered after consecutive failures
//! 3. MutationApprover validates intents via MutationLayer

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;

use aletheon_abi::evolution::*;
use aletheon_brain::r#impl::llm::provider::{LlmProvider, StopReason};
use aletheon_brain::r#impl::llm::scheduler::LlmScheduler;
use aletheon_brain::r#impl::event_handlers::{ToolObservationHandler, ObserverConfig, EvolutionEvent};
use aletheon_brain::testing::mock_llm::MockLlmProvider;
use aletheon_self::core::mutation::MutationLayer;
use aletheon_self::r#impl::mutation::MutationApprover;
use uuid::Uuid;

/// Build a LlmScheduler backed by a MockLlmProvider.
///
/// The mock is registered under the name "mock" and mapped to all LlmPurpose variants.
fn make_scheduler_with_mock(mock: Arc<MockLlmProvider>) -> Arc<LlmScheduler> {
    let mut providers: HashMap<String, Arc<dyn LlmProvider>> = HashMap::new();
    providers.insert("mock".to_string(), mock);

    let mut routing: HashMap<LlmPurpose, String> = HashMap::new();
    routing.insert(LlmPurpose::Reflect, "mock".to_string());
    routing.insert(LlmPurpose::ExtractRules, "mock".to_string());
    routing.insert(LlmPurpose::GenerateMutations, "mock".to_string());
    routing.insert(LlmPurpose::Execute, "mock".to_string());

    Arc::new(LlmScheduler::from_providers(providers, routing))
}

fn make_failure_obs(turn_id: Uuid) -> ToolObservationPayload {
    ToolObservationPayload {
        turn_id,
        tool_name: "bash_exec".to_string(),
        input: serde_json::json!({"command": "test"}),
        output: serde_json::json!("error"),
        duration_ms: 100,
        error: Some("permission denied".to_string()),
        rules_applied: vec![],
    }
}

// ---------------------------------------------------------------------------
// Test 1: Reflection emitted after tool observation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_reflection_emitted_after_tool_observation() -> Result<()> {
    // Mock LLM returns a valid reflection JSON for the first call (reflect),
    // and an empty rules array for the second call (extract_rules, if batch_size=1).
    let mock = Arc::new(MockLlmProvider::new("mock"));
    // Use batch_size=1 so a single observation triggers the full pipeline
    // (reflect + extract_rules). We enqueue 2 responses.
    mock.push_text_response(
        r#"{"assessment": "Failure", "root_cause": "permission denied", "suggested_rule": null, "confidence": 0.3}"#,
        StopReason::EndTurn,
    );
    mock.push_text_response("[]", StopReason::EndTurn); // extract_rules returns empty

    let scheduler = make_scheduler_with_mock(mock.clone());
    let observer = Arc::new(ToolObservationHandler::new(
        scheduler,
        ObserverConfig {
            batch_size: 1,
            consecutive_failure_threshold: 3,
            confidence_drop_threshold: 0.2,
        },
    ));

    let obs = make_failure_obs(Uuid::new_v4());
    let events = observer.handle(&obs).await?;

    // Should have at least a Reflection event
    assert!(
        events.iter().any(|e| matches!(e, EvolutionEvent::Reflection(_))),
        "Expected a Reflection event, got: {:?}",
        events.iter().map(|e| std::mem::discriminant(e)).collect::<Vec<_>>(),
    );

    // Verify the reflection content
    if let Some(EvolutionEvent::Reflection(ref r)) = events.first() {
        assert!(matches!(r.assessment, Assessment::Failure));
        assert_eq!(r.root_cause.as_deref(), Some("permission denied"));
        assert!((r.confidence - 0.3).abs() < f64::EPSILON);
    } else {
        panic!("First event should be Reflection");
    }

    // Verify mock was consumed
    assert_eq!(mock.remaining(), 0);

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 2: Evolution triggered after consecutive failures
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_evolution_triggered_after_consecutive_failures() -> Result<()> {
    let mock = Arc::new(MockLlmProvider::new("mock"));

    // batch_size=3, consecutive_failure_threshold=3
    // We emit 3 observations, each with 1 reflect response + 1 extract_rules response
    // On the 3rd observation, batch fills, extract_rules runs, then evolution check.
    for _ in 0..3 {
        mock.push_text_response(
            r#"{"assessment": "Failure", "root_cause": "timeout", "suggested_rule": null, "confidence": 0.2}"#,
            StopReason::EndTurn,
        );
    }
    // extract_rules response (triggered after 3rd observation)
    mock.push_text_response(
        r#"[{"condition": "when timeout occurs", "action": "retry with backoff", "confidence": 0.7}]"#,
        StopReason::EndTurn,
    );

    let scheduler = make_scheduler_with_mock(mock.clone());
    let observer = Arc::new(ToolObservationHandler::new(
        scheduler,
        ObserverConfig {
            batch_size: 3,
            consecutive_failure_threshold: 3,
            confidence_drop_threshold: 1.0, // disable confidence drop trigger
        },
    ));

    let mut all_events = Vec::new();
    for _ in 0..3 {
        let obs = make_failure_obs(Uuid::new_v4());
        let events = observer.handle(&obs).await?;
        all_events.extend(events);
    }

    // Should have at least one EvolutionTriggered event
    let has_evolution = all_events
        .iter()
        .any(|e| matches!(e, EvolutionEvent::EvolutionTriggered(_)));
    assert!(
        has_evolution,
        "Expected EvolutionTriggered event after 3 consecutive failures; got {} events",
        all_events.len(),
    );

    // Verify the trigger reason
    if let Some(EvolutionEvent::EvolutionTriggered(trigger)) = all_events
        .iter()
        .find(|e| matches!(e, EvolutionEvent::EvolutionTriggered(_)))
    {
        assert_eq!(trigger.trigger_reason, "consecutive_failures");
        assert_eq!(trigger.recent_reflections.len(), 3);
    }

    // Should also have RuleExtracted (batch of 3 reflections)
    let has_rules = all_events
        .iter()
        .any(|e| matches!(e, EvolutionEvent::RuleExtracted(_)));
    assert!(has_rules, "Expected RuleExtracted event from batch of 3");

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3: MutationApprover validates intents via MutationLayer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mutation_approver_validates_intents() -> Result<()> {
    let mutation_layer = Arc::new(MutationLayer::new());
    let approver = MutationApprover::new(mutation_layer.clone());

    // Trigger with "consecutive_failures" reason -> generates safety_weight intent
    let trigger = EvolutionTriggeredPayload {
        trigger_reason: "consecutive_failures".to_string(),
        recent_reflections: vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()],
        current_rules_snapshot: vec![LearnedRule {
            id: Uuid::new_v4(),
            condition: "when timeout occurs".to_string(),
            action: "retry with backoff".to_string(),
            confidence: 0.7,
            source_reflections: vec![],
        }],
    };

    let approved = approver.handle(&trigger)?;

    // Should have 1 approved intent (reversible -> Allow by MutationLayer)
    assert_eq!(approved.len(), 1, "Expected 1 approved intent");
    assert_eq!(approved[0].target, "care.priorities");
    assert!(approved[0].reversible);
    assert!(
        approved[0].change["delta"].as_f64().is_some(),
        "Intent change should have a delta field"
    );

    // Verify MutationLayer recorded the review
    let records = mutation_layer.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].target, "care.priorities");
    assert_eq!(
        records[0].status,
        aletheon_self::core::mutation::MutationStatus::Approved
    );

    // Trigger with unknown reason -> no intents
    let unknown_trigger = EvolutionTriggeredPayload {
        trigger_reason: "unknown".to_string(),
        recent_reflections: vec![],
        current_rules_snapshot: vec![],
    };
    let empty_approved = approver.handle(&unknown_trigger)?;
    assert!(empty_approved.is_empty(), "Unknown trigger should produce no intents");

    Ok(())
}

use executive::application::memory_policy::{
    MemoryAxisEvidence, MemoryNovelty, MemoryPolicyDecisionKind, MemoryPolicyEvaluator,
    MemoryPolicyFacts,
};
use executive::composition::config::MemoryPolicyConfig;
use fabric::protocol::memory::{MemoryObservationKindV1, MemoryRecordKindV1, MemorySensitivityV1};
use mnemosyne::{GovernedMemoryObservation, WorkspaceMemoryKey};

fn observation() -> GovernedMemoryObservation {
    GovernedMemoryObservation {
        observation_id: "observation-1".into(),
        client_session_id: "session-1".into(),
        client_turn_id: Some("turn-1".into()),
        kind: MemoryObservationKindV1::ExplicitNote,
        content: "typed durable constraint".into(),
        content_fingerprint: format!("keyed-sha256:{}", "a".repeat(64)),
        scrub_policy_version: 1,
        scrub_redactions: 0,
        occurred_at: Some("2026-08-01T00:00:00Z".into()),
        source_refs: vec!["test-receipt:1".into()],
        sensitivity: MemorySensitivityV1::Internal,
        explicit_user_action: true,
        principal_id: "principal-a".into(),
        workspace_key: WorkspaceMemoryKey::from_verified("ws:repo:sha256:a").unwrap(),
        connection_kind: "versioned_local_rpc".into(),
        observed_at_ms: 1_785_552_000_000,
    }
}

fn facts() -> MemoryPolicyFacts {
    MemoryPolicyFacts {
        provenance_complete: true,
        scope_verified: true,
        scrub_passed: true,
        control_instruction_detected: false,
        model_only_claim: false,
        approved_core_conflict: false,
        binding_verified: true,
        verification_receipts: 1,
        novelty: MemoryNovelty::New,
        contradiction_unresolved: false,
    }
}

fn uniform_axes(millis: u16) -> MemoryAxisEvidence {
    MemoryAxisEvidence {
        evidence_provenance_millis: millis,
        future_utility_millis: millis,
        stability_millis: millis,
        novelty_dedup_millis: millis,
        scope_fit_millis: millis,
        verification_millis: millis,
        privacy_risk_millis: 0,
        contradiction_risk_millis: 0,
    }
}

#[test]
fn threshold_edges_are_exact_and_remote_requires_80() {
    let evaluator = MemoryPolicyEvaluator::new(MemoryPolicyConfig::default()).unwrap();
    let cases = [
        (540, 54, MemoryPolicyDecisionKind::Reject, false),
        (550, 55, MemoryPolicyDecisionKind::Candidate, false),
        (740, 74, MemoryPolicyDecisionKind::Candidate, false),
        (750, 75, MemoryPolicyDecisionKind::PromoteLocal, false),
        (790, 79, MemoryPolicyDecisionKind::PromoteLocal, false),
        (800, 80, MemoryPolicyDecisionKind::PromoteLocal, true),
    ];
    for (millis, total, kind, remote) in cases {
        let result = evaluator
            .evaluate(&observation(), facts(), uniform_axes(millis))
            .unwrap();
        assert_eq!(result.scorecard.total, total);
        assert_eq!(result.kind, kind);
        assert_eq!(result.remote_eligible, remote);
    }
}

#[test]
fn every_hard_gate_forces_rejection_and_remote_block() {
    let evaluator = MemoryPolicyEvaluator::new(MemoryPolicyConfig::default()).unwrap();
    let mut cases = Vec::new();
    let mut value = facts();
    value.scrub_passed = false;
    cases.push((value, "scrub_failed"));
    let mut value = facts();
    value.provenance_complete = false;
    cases.push((value, "provenance_incomplete"));
    let mut value = facts();
    value.scope_verified = false;
    cases.push((value, "scope_unverified"));
    let mut value = facts();
    value.control_instruction_detected = true;
    cases.push((value, "control_instruction_detected"));
    let mut value = facts();
    value.model_only_claim = true;
    cases.push((value, "model_only_claim"));
    let mut value = facts();
    value.approved_core_conflict = true;
    cases.push((value, "approved_core_conflict"));

    for (facts, reason) in cases {
        let result = evaluator
            .evaluate(&observation(), facts, uniform_axes(1_000))
            .unwrap();
        assert_eq!(result.kind, MemoryPolicyDecisionKind::Reject);
        assert!(result.hard_gate_reasons.contains(&reason.to_string()));
        assert!(!result.remote_eligible);
    }
}

#[test]
fn host_derivation_uses_typed_evidence_and_clamps_risk_to_zero() {
    let evaluator = MemoryPolicyEvaluator::new(MemoryPolicyConfig::default()).unwrap();
    let mut value = observation();
    value.scrub_redactions = 2;
    value.sensitivity = MemorySensitivityV1::Restricted;
    let axes = evaluator.derive_axes(&value, facts());
    assert_eq!(axes.privacy_risk_millis, 1_000);
    let all_risk = MemoryAxisEvidence {
        evidence_provenance_millis: 0,
        future_utility_millis: 0,
        stability_millis: 0,
        novelty_dedup_millis: 0,
        scope_fit_millis: 0,
        verification_millis: 0,
        privacy_risk_millis: 1_000,
        contradiction_risk_millis: 1_000,
    };
    let result = evaluator.evaluate(&value, facts(), all_risk).unwrap();
    assert_eq!(result.scorecard.privacy_risk, -25);
    assert_eq!(result.scorecard.contradiction_risk, -20);
    assert_eq!(result.scorecard.total, 0);
    assert!(!result.remote_eligible);
}

#[test]
fn core_state_is_never_an_automatic_output_and_binding_only_blocks_remote() {
    let evaluator = MemoryPolicyEvaluator::new(MemoryPolicyConfig::default()).unwrap();
    let mut unbound = facts();
    unbound.binding_verified = false;
    let result = evaluator
        .evaluate(&observation(), unbound, uniform_axes(1_000))
        .unwrap();
    assert_eq!(result.kind, MemoryPolicyDecisionKind::PromoteLocal);
    assert_eq!(result.record_kind, MemoryRecordKindV1::SemanticFact);
    assert_ne!(result.record_kind, MemoryRecordKindV1::CoreState);
    assert!(!result.remote_eligible);
    assert!(result
        .remote_block_reasons
        .contains(&"binding_unverified".into()));
    assert_eq!(result.scorecard.policy_version, "memory-policy-v1");
}

#[test]
fn normalized_axis_overflow_is_rejected() {
    let evaluator = MemoryPolicyEvaluator::new(MemoryPolicyConfig::default()).unwrap();
    let mut axes = uniform_axes(1_000);
    axes.future_utility_millis = 1_001;
    assert!(evaluator.evaluate(&observation(), facts(), axes).is_err());
}

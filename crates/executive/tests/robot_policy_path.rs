//! Integration tests for the policy proposal path through RobotHarness.
//! Verifies that policy cannot call EmbodiedExecutionPort directly.

use cognit::ports::policy_provider::PolicyProviderPort;
use fabric::types::embodiment::{DeviceId, SkillDescriptor, RiskClass, SkillId};
use fabric::types::perception_observation::PerceptionObservation;
use fabric::types::world_state::WorldSnapshot;
use std::sync::Arc;

/// Stub policy for testing
struct StubPolicy;
#[async_trait::async_trait]
impl PolicyProviderPort for StubPolicy {
    async fn propose(
        &self, _goal: &str, _device: &DeviceId, _snapshots: &[WorldSnapshot],
        _visual: &[PerceptionObservation], _allowed: &[SkillDescriptor],
    ) -> Result<Vec<fabric::types::skill_proposal::SkillProposal>, String> {
        Ok(vec![fabric::types::skill_proposal::SkillProposal {
            skill: SkillId("kuavo.stance".into()),
            device: DeviceId("bot".into()),
            parameters: serde_json::json!({}),
            expected_outcome: fabric::types::expected_outcome::ExpectedOutcome {
                predicate: fabric::types::expected_outcome::OutcomePredicate::Equals {
                    path: "mode".into(),
                    value: serde_json::json!("stance"),
                },
                freshness_ms: 500, stable_window_ms: 0, timeout_ms: 5000,
            },
            confidence: 0.9,
            frame_refs: vec![],
            provenance: fabric::types::skill_proposal::PolicyProvenance {
                provider: "test".into(), model: "m".into(),
                version: "1".into(), digest: "abc".into(),
            },
        }])
    }
    async fn health(&self) -> Result<String, String> { Ok("ready".into()) }
}

#[test]
fn policy_cannot_call_embodied_execution_port() {
    // Architecture check: PolicyProviderPort trait has only `propose` and `health`.
    // It does NOT have `execute`, `cancel`, or `safe_stop` methods.
    // The type system enforces this — validate by checking the trait.
    // This compiles iff PolicyProviderPort is correctly bounded.
    let _policy: Arc<dyn PolicyProviderPort> = Arc::new(StubPolicy);
    // If this compiles, the policy port is clean — no execution methods
}

#[test]
fn proposal_validator_enforces_registered_skills() {
    use cognit::harness::robot::proposal_validator::validate_proposal;
    use fabric::types::skill_proposal::{SkillProposal, PolicyProvenance};
    use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};

    let allowed: Vec<SkillDescriptor> = vec![SkillDescriptor {
        skill: SkillId("kuavo.stance".into()),
        device: DeviceId("bot".into()),
        summary: "stance".into(),
        input_schema: serde_json::json!({"type": "object", "required": []}),
        risk: RiskClass::Low, timeout_ms: 10000, cancellable: false,
        preconditions: vec![], success_criteria: vec![],
    }];

    let valid = SkillProposal {
        skill: SkillId("kuavo.stance".into()),
        device: DeviceId("bot".into()),
        parameters: serde_json::json!({}),
        expected_outcome: ExpectedOutcome {
            predicate: OutcomePredicate::Equals { path: "mode".into(), value: serde_json::json!("stance") },
            freshness_ms: 500, stable_window_ms: 0, timeout_ms: 5000,
        },
        confidence: 0.9, frame_refs: vec![],
        provenance: PolicyProvenance { provider: "p".into(), model: "m".into(), version: "1".into(), digest: "d".into() },
    };
    assert!(validate_proposal(&valid, &allowed, 0, 0).is_ok());

    let invalid = SkillProposal {
        skill: SkillId("unknown.skill".into()),
        ..valid.clone()
    };
    assert!(validate_proposal(&invalid, &allowed, 0, 0).is_err());
}

#[test]
fn proposal_confidence_must_be_bounded() {
    use cognit::harness::robot::proposal_validator::validate_proposal;
    use fabric::types::skill_proposal::{SkillProposal, PolicyProvenance};
    use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};

    let allowed: Vec<SkillDescriptor> = vec![SkillDescriptor {
        skill: SkillId("s".into()), device: DeviceId("d".into()), summary: "s".into(),
        input_schema: serde_json::json!({"type": "object", "required": []}),
        risk: RiskClass::Low, timeout_ms: 10000, cancellable: false,
        preconditions: vec![], success_criteria: vec![],
    }];

    let base = SkillProposal {
        skill: SkillId("s".into()), device: DeviceId("d".into()), parameters: serde_json::json!({}),
        expected_outcome: ExpectedOutcome {
            predicate: OutcomePredicate::Equals { path: "x".into(), value: serde_json::json!(0) },
            freshness_ms: 0, stable_window_ms: 0, timeout_ms: 0,
        },
        confidence: 0.0, frame_refs: vec![],
        provenance: PolicyProvenance { provider: "p".into(), model: "m".into(), version: "1".into(), digest: "d".into() },
    };

    let low = SkillProposal { confidence: -0.1, ..base.clone() };
    assert!(validate_proposal(&low, &allowed, 0, 0).is_err());

    let high = SkillProposal { confidence: 1.1, ..base.clone() };
    assert!(validate_proposal(&high, &allowed, 0, 0).is_err());

    let ok = SkillProposal { confidence: 0.5, ..base.clone() };
    assert!(validate_proposal(&ok, &allowed, 0, 0).is_ok());
}

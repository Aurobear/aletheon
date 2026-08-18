//! Integration tests for the policy proposal path through RobotHarness.
//! Verifies that policy cannot call EmbodiedExecutionPort directly.

use ::contracts::types::embodiment::{DeviceId, RiskClass, SkillDescriptor, SkillId};
use ::contracts::types::world_state::WorldSnapshot;
use cognit::harness::robot::PerceptionObservation;
use cognit::ports::policy_provider::PolicyProviderPort;
use std::sync::Arc;

fn fresh_snapshot() -> WorldSnapshot {
    WorldSnapshot {
        device: DeviceId("bot".into()),
        schema: "robot.state".into(),
        schema_version: 1,
        sequence: 1,
        payload: serde_json::json!({"mode": "idle", "x": 0}),
        observed_at: ::contracts::MonoTime(1),
        valid_until: None,
        stale: false,
    }
}

/// Stub policy for testing
struct StubPolicy;
#[async_trait::async_trait]
impl PolicyProviderPort for StubPolicy {
    async fn propose(
        &self,
        _goal: &str,
        _device: &DeviceId,
        _snapshots: &[WorldSnapshot],
        _visual: &[PerceptionObservation],
        _allowed: &[SkillDescriptor],
    ) -> Result<
        Vec<::contracts::types::skill_proposal::SkillProposal>,
        cognit::ports::policy_provider::PolicyProviderError,
    > {
        Ok(vec![::contracts::types::skill_proposal::SkillProposal {
            skill: SkillId("kuavo.stance".into()),
            device: DeviceId("bot".into()),
            parameters: serde_json::json!({}),
            goal_alignment: ::contracts::types::skill_proposal::GoalAlignment::Direct,
            expected_outcome: ::contracts::types::expected_outcome::ExpectedOutcome {
                predicate: ::contracts::types::expected_outcome::OutcomePredicate::Equals {
                    path: "mode".into(),
                    value: serde_json::json!("stance"),
                },
                freshness_ms: 500,
                stable_window_ms: 0,
                timeout_ms: 5000,
            },
            confidence: 0.9,
            frame_refs: vec![],
            provenance: ::contracts::types::skill_proposal::PolicyProvenance {
                provider: "test".into(),
                model: "m".into(),
                version: "1".into(),
                protocol_version: "1.0".into(),
                digest: "abc".into(),
            },
        }])
    }
    async fn health(&self) -> Result<String, String> {
        Ok("ready".into())
    }
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
fn policy_boundary_has_no_execution_capability_symbols() {
    let port = include_str!("../../cognit/src/ports/policy_provider.rs");
    let adapter = include_str!("../../cognit/src/adapters/policy/grpc_provider.rs");
    for forbidden in [
        "EmbodimentExecutionPort",
        "EmbodiedExecutionPort",
        "execute_skill(",
        "safe_stop(",
        "cancel(",
    ] {
        assert!(
            !port.contains(forbidden),
            "Policy port contains {forbidden}"
        );
        assert!(
            !adapter.contains(forbidden),
            "Policy adapter contains {forbidden}"
        );
    }
}

#[test]
fn child_agent_runtime_is_pinned_to_linear_cognitive_sessions() {
    let bootstrap = include_str!("../../aletheon/src/composition/daemon_bootstrap/request.rs");
    let robot_bootstrap = include_str!("../../aletheon/src/composition/daemon_bootstrap/robot.rs");
    let native_start = bootstrap
        .find("NativeCognitRuntimeResources {")
        .expect("native Cognit runtime composition must remain explicit");
    let native = &bootstrap[native_start..];
    let native_end = native
        .find("runtime_bindings.push(super::services::RuntimeLauncherBinding")
        .expect("native Cognit runtime composition must remain bounded");
    let native = &native[..native_end];

    assert!(
        bootstrap.contains("build_target_routed_cognition(")
            && robot_bootstrap.contains("TargetRoutedCognitiveSessionFactory::new"),
        "the top-level runtime must retain explicit per-turn Robot routing"
    );
    assert!(
        native.contains("sessions: linear_cognitive_sessions.clone()"),
        "child Agents must use the non-embodied linear session factory"
    );
    assert!(
        !native.contains("sessions: domains.cognition()"),
        "child Agents must not inherit the top-level Robot control surface"
    );
}

#[test]
fn proposal_validator_enforces_registered_skills() {
    use ::contracts::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
    use ::contracts::types::skill_proposal::{PolicyProvenance, SkillProposal};
    use cognit::harness::robot::proposal_validator::validate_proposal;

    let allowed: Vec<SkillDescriptor> = vec![SkillDescriptor {
        skill: SkillId("kuavo.stance".into()),
        device: DeviceId("bot".into()),
        summary: "stance".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        }),
        risk: RiskClass::Low,
        timeout_ms: 10000,
        cancellable: false,
        preconditions: vec![],
        success_criteria: vec![],
    }];

    let valid = SkillProposal {
        skill: SkillId("kuavo.stance".into()),
        device: DeviceId("bot".into()),
        parameters: serde_json::json!({}),
        goal_alignment: ::contracts::types::skill_proposal::GoalAlignment::Direct,
        expected_outcome: ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("stance"),
            },
            freshness_ms: 500,
            stable_window_ms: 0,
            timeout_ms: 5000,
        },
        confidence: 0.9,
        frame_refs: vec![],
        provenance: PolicyProvenance {
            provider: "p".into(),
            model: "m".into(),
            version: "1".into(),
            protocol_version: "1.0".into(),
            digest: "d".into(),
        },
    };
    assert!(validate_proposal(
        &valid,
        &DeviceId("bot".into()),
        &allowed,
        &[fresh_snapshot()]
    )
    .is_ok());

    let invalid = SkillProposal {
        skill: SkillId("unknown.skill".into()),
        ..valid.clone()
    };
    assert!(validate_proposal(
        &invalid,
        &DeviceId("bot".into()),
        &allowed,
        &[fresh_snapshot()]
    )
    .is_err());
}

#[test]
fn proposal_confidence_must_be_bounded() {
    use ::contracts::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
    use ::contracts::types::skill_proposal::{PolicyProvenance, SkillProposal};
    use cognit::harness::robot::proposal_validator::validate_proposal;

    let allowed: Vec<SkillDescriptor> = vec![SkillDescriptor {
        skill: SkillId("s".into()),
        device: DeviceId("d".into()),
        summary: "s".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        }),
        risk: RiskClass::Low,
        timeout_ms: 10000,
        cancellable: false,
        preconditions: vec![],
        success_criteria: vec![],
    }];

    let base = SkillProposal {
        skill: SkillId("s".into()),
        device: DeviceId("d".into()),
        parameters: serde_json::json!({}),
        goal_alignment: ::contracts::types::skill_proposal::GoalAlignment::Direct,
        expected_outcome: ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "x".into(),
                value: serde_json::json!(0),
            },
            freshness_ms: 0,
            stable_window_ms: 0,
            timeout_ms: 1,
        },
        confidence: 0.0,
        frame_refs: vec![],
        provenance: PolicyProvenance {
            provider: "p".into(),
            model: "m".into(),
            version: "1".into(),
            protocol_version: "1.0".into(),
            digest: "d".into(),
        },
    };

    let low = SkillProposal {
        confidence: -0.1,
        ..base.clone()
    };
    assert!(validate_proposal(
        &low,
        &DeviceId("d".into()),
        &allowed,
        &[WorldSnapshot {
            device: DeviceId("d".into()),
            ..fresh_snapshot()
        }]
    )
    .is_err());

    let high = SkillProposal {
        confidence: 1.1,
        ..base.clone()
    };
    assert!(validate_proposal(
        &high,
        &DeviceId("d".into()),
        &allowed,
        &[WorldSnapshot {
            device: DeviceId("d".into()),
            ..fresh_snapshot()
        }]
    )
    .is_err());

    let ok = SkillProposal {
        confidence: 0.5,
        ..base.clone()
    };
    assert!(validate_proposal(
        &ok,
        &DeviceId("d".into()),
        &allowed,
        &[WorldSnapshot {
            device: DeviceId("d".into()),
            ..fresh_snapshot()
        }]
    )
    .is_ok());
}

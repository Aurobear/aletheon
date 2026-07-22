use cognit::harness::robot::proposal_validator::validate_proposal;
use fabric::types::embodiment::{DeviceId, RiskClass, SkillDescriptor, SkillId};
use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
use fabric::types::skill_proposal::{PolicyProvenance, SkillProposal};

fn allowed_skills() -> Vec<SkillDescriptor> {
    vec![SkillDescriptor {
        skill: SkillId("kuavo.stance".into()),
        device: DeviceId("bot".into()),
        summary: "stance".into(),
        input_schema: serde_json::json!({"type": "object", "required": []}),
        risk: RiskClass::Low,
        timeout_ms: 10000,
        cancellable: false,
        preconditions: vec![],
        success_criteria: vec![],
    }]
}

#[test]
fn integration_valid_proposal_passes() {
    let p = SkillProposal {
        skill: SkillId("kuavo.stance".into()),
        device: DeviceId("bot".into()),
        parameters: serde_json::json!({}),
        expected_outcome: ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("stance"),
            },
            freshness_ms: 500,
            stable_window_ms: 0,
            timeout_ms: 5000,
        },
        confidence: 0.8,
        frame_refs: vec![],
        provenance: PolicyProvenance {
            provider: "vla".into(),
            model: "m".into(),
            version: "1".into(),
            digest: "abc".into(),
        },
    };
    assert!(validate_proposal(&p, &allowed_skills(), 1000, 5000).is_ok());
}

#[test]
fn integration_unknown_skill_fails() {
    let p = SkillProposal {
        skill: SkillId("unknown".into()),
        device: DeviceId("bot".into()),
        parameters: serde_json::json!({}),
        expected_outcome: ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "x".into(),
                value: serde_json::json!(0),
            },
            freshness_ms: 0,
            stable_window_ms: 0,
            timeout_ms: 0,
        },
        confidence: 0.5,
        frame_refs: vec![],
        provenance: PolicyProvenance {
            provider: "v".into(),
            model: "m".into(),
            version: "1".into(),
            digest: "d".into(),
        },
    };
    assert!(validate_proposal(&p, &allowed_skills(), 0, 0).is_err());
}

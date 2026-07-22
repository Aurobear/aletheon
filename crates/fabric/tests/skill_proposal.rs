use fabric::types::embodiment::{DeviceId, SkillId};
use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
use fabric::types::skill_proposal::{PolicyProvenance, SkillProposal};

fn make_valid_proposal() -> SkillProposal {
    SkillProposal {
        skill: SkillId("wave".into()),
        device: DeviceId("bot".into()),
        parameters: serde_json::json!({"amplitude": 0.5}),
        expected_outcome: ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("stance"),
            },
            freshness_ms: 500,
            stable_window_ms: 200,
            timeout_ms: 5000,
        },
        confidence: 0.95,
        frame_refs: vec!["artifact://sha256:abc".into()],
        provenance: PolicyProvenance {
            provider: "openvla-v1".into(),
            model: "openvla-7b".into(),
            version: "1.0".into(),
            digest: "sha256:def456".into(),
        },
    }
}

#[test]
fn valid_proposal_passes_validation() {
    let p = make_valid_proposal();
    assert!(p.validate().is_ok());
}

#[test]
fn invalid_confidence_rejected() {
    let mut p = make_valid_proposal();
    p.confidence = 1.5;
    assert!(p.validate().is_err());
}

#[test]
fn negative_confidence_rejected() {
    let mut p = make_valid_proposal();
    p.confidence = -0.1;
    assert!(p.validate().is_err());
}

#[test]
fn too_many_frame_refs_rejected() {
    let mut p = make_valid_proposal();
    p.frame_refs = (0..5).map(|i| format!("artifact://sha256:{}", i)).collect();
    assert!(p.validate().is_err());
}

#[test]
fn empty_provenance_digest_rejected() {
    let mut p = make_valid_proposal();
    p.provenance.digest = "".into();
    assert!(p.validate().is_err());
}

#[test]
fn invalid_expected_outcome_rejected() {
    let mut p = make_valid_proposal();
    p.expected_outcome.predicate = OutcomePredicate::All {
        predicates: vec![],
    };
    assert!(p.validate().is_err());
}

#[test]
fn skill_proposal_serde_roundtrip() {
    let p = make_valid_proposal();
    let json = serde_json::to_string(&p).unwrap();
    let back: SkillProposal = serde_json::from_str(&json).unwrap();
    assert_eq!(p, back);
}

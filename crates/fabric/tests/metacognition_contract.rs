//! Metacognition contract tests — generic experience and evidence serialization.
//!
//! These tests verify that domain adapters can construct, serialize, and
//! deserialize metacognition contracts without importing domain-private types.

use std::collections::BTreeMap;

use fabric::types::metacognition_evidence::{
    EvidenceId, EvidenceItem, EvidenceKind, EvidenceTrust,
};
use fabric::types::metacognition_experience::{
    DomainId, ExperienceEnvelope, ExperienceId, ExperienceOutcome, SubjectId,
    METACOGNITION_SCHEMA_V1,
};

#[test]
fn experience_envelope_roundtrips_through_json() {
    let experience = ExperienceEnvelope {
        schema_version: METACOGNITION_SCHEMA_V1,
        experience_id: ExperienceId("exp-1".into()),
        domain: DomainId::new("synthetic").unwrap(),
        subject: SubjectId("component-a".into()),
        goal_ref: Some("goal-1".into()),
        started_at_ms: 100,
        completed_at_ms: Some(200),
        outcome: ExperienceOutcome::Succeeded,
        correlations: BTreeMap::from([("task".into(), "task-1".into())]),
        evidence: vec![EvidenceId("ev-1".into())],
    };

    let json = serde_json::to_string(&experience).unwrap();
    let roundtripped: ExperienceEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(experience, roundtripped);
}

#[test]
fn domain_id_rejects_empty() {
    assert!(DomainId::new("").is_err());
    assert!(DomainId::new("   ").is_err());
}

#[test]
fn domain_id_rejects_control_characters() {
    assert!(DomainId::new("bad\nid").is_err());
    assert!(DomainId::new("bad\tid").is_err());
}

#[test]
fn domain_id_rejects_invalid_chars() {
    assert!(DomainId::new("bad id").is_err());
    assert!(DomainId::new("bad@id").is_err());
}

#[test]
fn domain_id_accepts_valid_identifiers() {
    assert!(DomainId::new("coding").is_ok());
    assert!(DomainId::new("robot-behavior").is_ok());
    assert!(DomainId::new("research_v2").is_ok());
}

#[test]
fn evidence_item_serialization_is_stable() {
    let item = EvidenceItem {
        schema_version: 1,
        evidence_id: EvidenceId("ev-1".into()),
        experience_id: ExperienceId("exp-1".into()),
        kind: EvidenceKind::ActionResult,
        source: "compiler".into(),
        producer: "rustc".into(),
        captured_at_ms: 300,
        payload: serde_json::json!({"exit_code": 0}),
        sha256: "abc123".into(),
        trust: EvidenceTrust::Authoritative,
        freshness_ms: Some(5000),
        redacted: false,
    };

    let json = serde_json::to_string(&item).unwrap();
    let roundtripped: EvidenceItem = serde_json::from_str(&json).unwrap();
    assert_eq!(item.schema_version, roundtripped.schema_version);
    assert_eq!(item.evidence_id, roundtripped.evidence_id);
    assert_eq!(item.kind, roundtripped.kind);
    assert_eq!(item.trust, roundtripped.trust);
}

#[test]
fn experience_outcome_serializes_as_snake_case() {
    let json = serde_json::to_string(&ExperienceOutcome::TimedOut).unwrap();
    assert_eq!(json, r#""timed_out""#);
}

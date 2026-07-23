//! Metacognition contract tests — generic experience and evidence serialization.
//!
//! These tests verify that domain adapters can construct, serialize, and
//! deserialize metacognition contracts without importing domain-private types.

use std::collections::BTreeMap;

use fabric::types::metacognition_evaluation::{
    DimensionScore, DimensionValue, EvaluationReport, GateResult, RubricId,
};
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

// ---------------------------------------------------------------------------
// Task 8: Rubric and evaluation report contract tests
// ---------------------------------------------------------------------------

#[test]
fn evaluation_report_unknown_dimension_absent_from_weighted_total() {
    let report = EvaluationReport {
        rubric: RubricId("test-rubric".into()),
        rubric_version: 1,
        dimensions: vec![
            DimensionScore {
                name: "goal_attainment".into(),
                value: DimensionValue::Scored(80),
                weight_millis: 500_000, // 0.5
                evidence: vec![EvidenceId("ev-1".into())],
                reasons: vec!["target met".into()],
            },
            DimensionScore {
                name: "safety".into(),
                value: DimensionValue::Unknown,
                weight_millis: 300_000, // 0.3 — excluded from denominator
                evidence: vec![],
                reasons: vec!["no safety evidence available".into()],
            },
            DimensionScore {
                name: "efficiency".into(),
                value: DimensionValue::Scored(90),
                weight_millis: 200_000, // 0.2
                evidence: vec![EvidenceId("ev-2".into())],
                reasons: vec!["within budget".into()],
            },
        ],
        gates: vec![],
        // expected: (80 * 0.5 + 90 * 0.2) / (0.5 + 0.2) = (40 + 18) / 0.7 = 82.857...
        weighted_total_millis: Some(82_857),
        evidence_coverage_millis: 667, // 2/3 ≈ 0.667
        confidence_millis: 500,
        eligible: true,
    };

    // Round-trip through JSON
    let json = serde_json::to_string(&report).unwrap();
    let rt: EvaluationReport = serde_json::from_str(&json).unwrap();
    assert_eq!(report, rt);

    // Unknown dimension is preserved in the report (explicitly, not as zero)
    let safety_dim = rt.dimensions.iter().find(|d| d.name == "safety").unwrap();
    assert_eq!(safety_dim.value, DimensionValue::Unknown);

    // Weighted total is present because there are applicable dimensions
    assert!(rt.weighted_total_millis.is_some());
}

#[test]
fn high_score_with_failed_hard_gate_is_ineligible() {
    let report = EvaluationReport {
        rubric: RubricId("safety-rubric".into()),
        rubric_version: 1,
        dimensions: vec![DimensionScore {
            name: "goal_attainment".into(),
            value: DimensionValue::Scored(95),
            weight_millis: 1_000_000, // 1.0
            evidence: vec![EvidenceId("ev-1".into())],
            reasons: vec!["excellent completion".into()],
        }],
        gates: vec![GateResult {
            name: "safety_invariant".into(),
            passed: false,
            evidence: vec![EvidenceId("ev-gate".into())],
        }],
        weighted_total_millis: Some(95_000), // 95.0 in millis
        evidence_coverage_millis: 1_000,
        confidence_millis: 900,
        eligible: false, // failed gate overrides score
    };

    let json = serde_json::to_string(&report).unwrap();
    let rt: EvaluationReport = serde_json::from_str(&json).unwrap();
    assert_eq!(report, rt);

    // Score is high
    assert_eq!(rt.weighted_total_millis, Some(95_000));
    // But eligibility is false due to failed gate
    assert!(!rt.eligible);
    assert!(rt
        .gates
        .iter()
        .any(|g| g.name == "safety_invariant" && !g.passed));
}

#[test]
fn dimension_value_unknown_serializes_correctly() {
    assert_eq!(
        serde_json::to_string(&DimensionValue::Unknown).unwrap(),
        r#""unknown""#
    );
    assert_eq!(
        serde_json::to_string(&DimensionValue::Scored(42)).unwrap(),
        r#"{"scored":42}"#
    );
}

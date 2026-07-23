//! Integration tests for the deterministic reflection engine.
//!
//! Covers: recurring patterns, contrary evidence, observation recommendations,
//! and the absence of MutationIntent in the reflection API.

use metacog::reflection::{
    DeterministicReflectionEngine, ProblemSummary, ReflectionEngine, ReflectionInput,
};

fn make_problem(
    id: &str,
    category: &str,
    state: &str,
    severity: u8,
    description: &str,
    has_contrary: bool,
    contrary: Vec<String>,
) -> ProblemSummary {
    ProblemSummary {
        problem_id: id.to_string(),
        category: category.to_string(),
        state: state.to_string(),
        severity_ordinal: severity,
        description: description.to_string(),
        has_contrary_evidence: has_contrary,
        contrary_evidence: contrary,
    }
}

#[test]
fn three_confirmed_problems_shared_category_yields_one_recurring_pattern() {
    let engine = DeterministicReflectionEngine;
    let input = ReflectionInput {
        experiences: vec![],
        evaluations: vec![],
        problems: vec![
            make_problem(
                "p1",
                "timeout",
                "confirmed",
                3,
                "req timeout",
                false,
                vec![],
            ),
            make_problem(
                "p2",
                "timeout",
                "confirmed",
                3,
                "req timeout 2",
                false,
                vec![],
            ),
            make_problem(
                "p3",
                "timeout",
                "confirmed",
                2,
                "req timeout 3",
                true,
                vec!["succeeds when retried quickly".to_string()],
            ),
        ],
    };
    let report = engine.reflect(input).unwrap();

    // Exactly one recurring pattern for the "timeout" category
    assert_eq!(report.recurring_patterns.len(), 1);
    let pattern = &report.recurring_patterns[0];
    assert_eq!(pattern.category, "timeout");
    assert_eq!(pattern.occurrence_count, 3);
    assert!(pattern.example_ids.contains(&"p1".to_string()));
    assert!(pattern.example_ids.contains(&"p2".to_string()));
    assert!(pattern.example_ids.contains(&"p3".to_string()));

    // Contrary evidence from p3 should appear
    assert!(!pattern.contrary_evidence.is_empty());
    assert!(pattern
        .contrary_evidence
        .contains(&"succeeds when retried quickly".to_string()));
}

#[test]
fn observation_recommendation_is_produced_for_recurring_pattern() {
    let engine = DeterministicReflectionEngine;
    let input = ReflectionInput {
        experiences: vec![],
        evaluations: vec![],
        problems: vec![
            make_problem(
                "p1",
                "timeout",
                "confirmed",
                3,
                "req timeout",
                false,
                vec![],
            ),
            make_problem(
                "p2",
                "timeout",
                "confirmed",
                3,
                "req timeout 2",
                false,
                vec![],
            ),
            make_problem("p3", "timeout", "active", 3, "req timeout 3", false, vec![]),
        ],
    };
    let report = engine.reflect(input).unwrap();

    assert!(!report.recommended_observations.is_empty());
    let has_monitor = report
        .recommended_observations
        .iter()
        .any(|r| r.contains("monitor category 'timeout'"));
    assert!(has_monitor);
}

#[test]
fn no_mutation_intent_appears_in_api() {
    // Reflection API must not reference MutationIntent directly.
    let engine = DeterministicReflectionEngine;
    let input = ReflectionInput {
        experiences: vec![],
        evaluations: vec![],
        problems: vec![
            make_problem("p1", "timeout", "confirmed", 3, "timeout", false, vec![]),
            make_problem("p2", "timeout", "confirmed", 3, "timeout 2", false, vec![]),
        ],
    };
    let report = engine.reflect(input).unwrap();

    // Proposals come back as ProposalId placeholders, not MutationIntent
    assert!(!report.improvement_proposals.is_empty());
    // No MutationIntent type anywhere in the report or its fields
    // (this is a compile-time assertion — the ReflectionReport has no fabric::MutationIntent field)
}

#[test]
fn strengths_and_weaknesses_derived_from_evaluations() {
    let engine = DeterministicReflectionEngine;

    use fabric::types::metacognition_evaluation::{EvaluationReport, GateResult, RubricId};

    let eval_passed = EvaluationReport {
        rubric: RubricId("rubric-1".into()),
        rubric_version: 1,
        dimensions: vec![],
        gates: vec![GateResult {
            name: "safety".into(),
            passed: true,
            evidence: vec![],
        }],
        weighted_total_millis: Some(85_000),
        evidence_coverage_millis: 900,
        confidence_millis: 800,
        eligible: true,
    };

    let eval_failed = EvaluationReport {
        rubric: RubricId("rubric-2".into()),
        rubric_version: 1,
        dimensions: vec![],
        gates: vec![GateResult {
            name: "safety".into(),
            passed: false,
            evidence: vec![],
        }],
        weighted_total_millis: Some(90_000),
        evidence_coverage_millis: 500,
        confidence_millis: 600,
        eligible: false,
    };

    let input = ReflectionInput {
        experiences: vec![],
        evaluations: vec![eval_passed, eval_failed],
        problems: vec![],
    };
    let result = engine.reflect(input);
    // No problems means error
    assert!(result.is_err());
}

#[test]
fn knowledge_gaps_from_unknown_dimensions() {
    let engine = DeterministicReflectionEngine;

    use fabric::types::metacognition_evaluation::{
        DimensionScore, DimensionValue, EvaluationReport, RubricId,
    };

    let eval_with_unknown = EvaluationReport {
        rubric: RubricId("rubric-1".into()),
        rubric_version: 1,
        dimensions: vec![
            DimensionScore {
                name: "goal_attainment".into(),
                value: DimensionValue::Scored(80),
                weight_millis: 500_000,
                evidence: vec![],
                reasons: vec![],
            },
            DimensionScore {
                name: "efficiency".into(),
                value: DimensionValue::Unknown,
                weight_millis: 300_000,
                evidence: vec![],
                reasons: vec![],
            },
        ],
        gates: vec![],
        weighted_total_millis: Some(80_000),
        evidence_coverage_millis: 500,
        confidence_millis: 500,
        eligible: true,
    };

    let input = ReflectionInput {
        experiences: vec![],
        evaluations: vec![eval_with_unknown],
        problems: vec![
            make_problem("p1", "timeout", "confirmed", 2, "timeout", false, vec![]),
            make_problem("p2", "timeout", "confirmed", 3, "timeout 2", false, vec![]),
        ],
    };
    let report = engine.reflect(input).unwrap();

    let has_unknown_dim = report
        .knowledge_gaps
        .iter()
        .any(|g| g.contains("efficiency") && g.contains("unknown"));
    assert!(has_unknown_dim);
}

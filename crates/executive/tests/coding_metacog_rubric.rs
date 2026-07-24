//! Coding metacognition rubric tests — deterministic fixtures for each dimension.
//!
//! Verifies that the coding rubric v1 has correct structure, that completion-only
//! messages produce low coverage, and that every dimension is independently testable.

use executive::application::coding_metacog_rubric::{coding_rubric_v1, Rubric, CODING_RUBRIC_V1};

use fabric::types::metacognition_evaluation::{
    DimensionScore, DimensionValue, GateResult, RubricId,
};

// ---------------------------------------------------------------------------
// Version and structure
// ---------------------------------------------------------------------------

#[test]
fn rubric_v1_has_correct_version() {
    let rubric = coding_rubric_v1();
    assert_eq!(rubric.version, CODING_RUBRIC_V1);
    assert_eq!(CODING_RUBRIC_V1, 1);
}

#[test]
fn rubric_v1_has_expected_dimensions() {
    let rubric = coding_rubric_v1();
    let names: Vec<&str> = rubric.dimensions.iter().map(|d| d.name.as_str()).collect();

    assert!(names.contains(&"requirement_coverage"));
    assert!(names.contains(&"correctness"));
    assert!(names.contains(&"scope_discipline"));
    assert!(names.contains(&"maintainability"));
    assert!(names.contains(&"verification_sufficiency"));
    assert!(names.contains(&"regression_risk"));
    assert_eq!(names.len(), 6);
}

#[test]
fn rubric_v1_has_expected_gates() {
    let rubric = coding_rubric_v1();
    let names: Vec<&str> = rubric.gates.iter().map(|g| g.name.as_str()).collect();

    assert!(names.contains(&"verification_evidence"));
    assert!(names.contains(&"change_within_scope"));
    assert_eq!(names.len(), 2);
}

// ---------------------------------------------------------------------------
// Weight sum
// ---------------------------------------------------------------------------

#[test]
fn dimension_weights_sum_to_one() {
    let rubric = coding_rubric_v1();
    let total: u32 = rubric.dimensions.iter().map(|d| d.weight_millis).sum();
    assert_eq!(
        total, 1_000_000,
        "weights must sum to 1.0 in fixed-point millis"
    );
}

#[test]
fn each_dimension_has_positive_weight() {
    let rubric = coding_rubric_v1();
    for dim in &rubric.dimensions {
        assert!(
            dim.weight_millis > 0,
            "dimension '{}' must have positive weight",
            dim.name
        );
    }
}

// ---------------------------------------------------------------------------
// Empty report behaviour
// ---------------------------------------------------------------------------

#[test]
fn build_empty_report_produces_all_unknown_dimensions() {
    let rubric = coding_rubric_v1();
    let report = rubric.build_empty_report();

    assert_eq!(report.dimensions.len(), 6);
    for dim in &report.dimensions {
        assert!(
            matches!(dim.value, DimensionValue::Unknown),
            "dimension '{}' should be Unknown",
            dim.name
        );
    }
}

#[test]
fn build_empty_report_produces_all_passing_gates() {
    let rubric = coding_rubric_v1();
    let report = rubric.build_empty_report();

    assert_eq!(report.gates.len(), 2);
    for gate in &report.gates {
        assert!(gate.passed, "gate '{}' should start as passed", gate.name);
    }
}

#[test]
fn build_empty_report_is_not_eligible() {
    let rubric = coding_rubric_v1();
    let report = rubric.build_empty_report();

    assert!(!report.eligible);
    assert_eq!(report.weighted_total_millis, None);
    assert_eq!(report.evidence_coverage_millis, 0);
    assert_eq!(report.confidence_millis, 0);
}

#[test]
fn build_empty_report_has_correct_rubric_id() {
    let rubric = coding_rubric_v1();
    let report = rubric.build_empty_report();

    assert_eq!(report.rubric, RubricId("coding-v1".to_string()));
    assert_eq!(report.rubric_version, CODING_RUBRIC_V1);
}

// ---------------------------------------------------------------------------
// Completion-only message produces low coverage (not a fabricated high score)
// ---------------------------------------------------------------------------

#[test]
fn completion_only_message_produces_zero_evidence_coverage() {
    let rubric = coding_rubric_v1();
    let report = rubric.build_empty_report();

    // With no evidence attached, coverage must be zero
    assert_eq!(report.evidence_coverage_millis, 0);

    // Zero coverage is below the minimum threshold (400 millis = 40%)
    let threshold = rubric.min_evidence_coverage_millis;
    assert!(
        report.evidence_coverage_millis < threshold,
        "completion-only message (0 evidence) must be below coverage threshold of {}",
        threshold
    );
}

#[test]
fn completion_only_message_is_not_eligible() {
    let rubric = coding_rubric_v1();
    let report = rubric.build_empty_report();

    // Even with all gates passing (default), zero coverage + no
    // weighted total means the report is not eligible.
    assert!(!report.eligible);
}

// ---------------------------------------------------------------------------
// Individual dimension coverage
// ---------------------------------------------------------------------------

/// Verifies that requirement_coverage dimension can be set to a scored value.
#[test]
fn requirement_coverage_dimension_is_scorable() {
    let rubric = coding_rubric_v1();
    let mut report = rubric.build_empty_report();

    let dim = report
        .dimensions
        .iter_mut()
        .find(|d| d.name == "requirement_coverage")
        .expect("must have requirement_coverage dimension");

    dim.value = DimensionValue::Scored(85);
    dim.reasons
        .push("all stated requirements addressed".to_string());
    dim.evidence
        .push(fabric::types::metacognition_evidence::EvidenceId(
            "ev-req-1".to_string(),
        ));

    match dim.value {
        DimensionValue::Scored(score) => assert_eq!(score, 85),
        DimensionValue::Unknown => panic!("should be scored"),
    }
}

/// Verifies that correctness dimension can be set to a scored value.
#[test]
fn correctness_dimension_is_scorable() {
    let rubric = coding_rubric_v1();
    let mut report = rubric.build_empty_report();

    let dim = report
        .dimensions
        .iter_mut()
        .find(|d| d.name == "correctness")
        .expect("must have correctness dimension");

    dim.value = DimensionValue::Scored(100);
    dim.reasons.push("all tests pass".to_string());

    match dim.value {
        DimensionValue::Scored(score) => assert_eq!(score, 100),
        DimensionValue::Unknown => panic!("should be scored"),
    }
}

/// Verifies that scope_discipline dimension is independently evaluable.
#[test]
fn scope_discipline_dimension_is_scorable() {
    let rubric = coding_rubric_v1();
    let mut report = rubric.build_empty_report();

    let dim = report
        .dimensions
        .iter_mut()
        .find(|d| d.name == "scope_discipline")
        .expect("must have scope_discipline dimension");

    dim.value = DimensionValue::Scored(90);
    dim.reasons.push("no forbidden paths touched".to_string());

    match dim.value {
        DimensionValue::Scored(score) => assert_eq!(score, 90),
        DimensionValue::Unknown => panic!("should be scored"),
    }
}

/// Verifies that maintainability dimension is independently evaluable.
#[test]
fn maintainability_dimension_is_scorable() {
    let rubric = coding_rubric_v1();
    let mut report = rubric.build_empty_report();

    let dim = report
        .dimensions
        .iter_mut()
        .find(|d| d.name == "maintainability")
        .expect("must have maintainability dimension");

    dim.value = DimensionValue::Scored(70);
    dim.reasons
        .push("code is clean but could use more comments".to_string());

    match dim.value {
        DimensionValue::Scored(score) => assert_eq!(score, 70),
        DimensionValue::Unknown => panic!("should be scored"),
    }
}

/// Verifies that verification_sufficiency dimension is independently evaluable.
#[test]
fn verification_sufficiency_dimension_is_scorable() {
    let rubric = coding_rubric_v1();
    let mut report = rubric.build_empty_report();

    let dim = report
        .dimensions
        .iter_mut()
        .find(|d| d.name == "verification_sufficiency")
        .expect("must have verification_sufficiency dimension");

    dim.value = DimensionValue::Scored(80);
    dim.reasons.push("test coverage is adequate".to_string());

    match dim.value {
        DimensionValue::Scored(score) => assert_eq!(score, 80),
        DimensionValue::Unknown => panic!("should be scored"),
    }
}

/// Verifies that regression_risk dimension is independently evaluable.
#[test]
fn regression_risk_dimension_is_scorable() {
    let rubric = coding_rubric_v1();
    let mut report = rubric.build_empty_report();

    let dim = report
        .dimensions
        .iter_mut()
        .find(|d| d.name == "regression_risk")
        .expect("must have regression_risk dimension");

    // Lower score = higher risk
    dim.value = DimensionValue::Scored(60);
    dim.reasons
        .push("changed a core function shared by many callers".to_string());

    match dim.value {
        DimensionValue::Scored(score) => assert_eq!(score, 60),
        DimensionValue::Unknown => panic!("should be scored"),
    }
}

// ---------------------------------------------------------------------------
// Gate toggle
// ---------------------------------------------------------------------------

#[test]
fn verification_evidence_gate_can_be_failed() {
    let rubric = coding_rubric_v1();
    let mut report = rubric.build_empty_report();

    let gate = report
        .gates
        .iter_mut()
        .find(|g| g.name == "verification_evidence")
        .expect("must have verification_evidence gate");

    gate.passed = false;
    gate.evidence
        .push(fabric::types::metacognition_evidence::EvidenceId(
            "ev-none".to_string(),
        ));

    assert!(!gate.passed);
}

#[test]
fn change_within_scope_gate_can_be_failed() {
    let rubric = coding_rubric_v1();
    let mut report = rubric.build_empty_report();

    let gate = report
        .gates
        .iter_mut()
        .find(|g| g.name == "change_within_scope")
        .expect("must have change_within_scope gate");

    gate.passed = false;
    gate.evidence
        .push(fabric::types::metacognition_evidence::EvidenceId(
            "ev-scope".to_string(),
        ));

    assert!(!gate.passed);
}

// ---------------------------------------------------------------------------
// Minimum evidence coverage threshold
// ---------------------------------------------------------------------------

#[test]
fn min_evidence_coverage_threshold_is_40_percent() {
    let rubric = coding_rubric_v1();
    // 400_000 millis = 0.4 = 40%
    assert_eq!(rubric.min_evidence_coverage_millis, 400);
}

// ---------------------------------------------------------------------------
// Dimension names are domain-specific but use only generic types
// ---------------------------------------------------------------------------

#[test]
fn rubric_uses_only_fabric_contract_types() {
    let rubric = coding_rubric_v1();
    let report = rubric.build_empty_report();

    // Prove that the rubric and report types are generic fabric types
    let _: &Rubric = &rubric;
    let _: &fabric::types::metacognition_evaluation::EvaluationReport = &report;

    // Dimension scores are generic
    for dim in &report.dimensions {
        let _: &DimensionScore = dim;
    }

    // Gate results are generic
    for gate in &report.gates {
        let _: &GateResult = gate;
    }
}

//! Deterministic evaluation engine integration tests.
//!
//! Covers: all-applicable, one-unknown, zero-applicable, failed hard gate,
//! missing evidence, weight overflow rejection.

use metacog::evaluation::{
    DeterministicEvaluator, DimensionScore, DimensionValue, GateResult, Rubric, RubricDimension,
    RubricGate, RubricId,
};

fn make_rubric() -> Rubric {
    Rubric {
        id: "test".into(),
        version: 1,
        dimensions: vec![
            RubricDimension {
                name: "goal".into(),
                weight_millis: 500_000,
                mandatory: true,
            },
            RubricDimension {
                name: "safety".into(),
                weight_millis: 300_000,
                mandatory: false,
            },
            RubricDimension {
                name: "efficiency".into(),
                weight_millis: 200_000,
                mandatory: false,
            },
        ],
        gates: vec![RubricGate {
            name: "invariant".into(),
            description: "system invariant holds".into(),
        }],
    }
}

// ---------------------------------------------------------------------------
// Test 1: all-applicable
// ---------------------------------------------------------------------------

#[test]
fn all_applicable_dimensions() {
    let engine = DeterministicEvaluator::new();
    let rubric = make_rubric();

    let scores = vec![
        DimensionScore {
            name: "goal".into(),
            value: DimensionValue::Scored(80),
            weight_millis: 500_000,
            evidence: vec![],
            reasons: vec!["ok".into()],
        },
        DimensionScore {
            name: "safety".into(),
            value: DimensionValue::Scored(100),
            weight_millis: 300_000,
            evidence: vec![],
            reasons: vec!["safe".into()],
        },
        DimensionScore {
            name: "efficiency".into(),
            value: DimensionValue::Scored(90),
            weight_millis: 200_000,
            evidence: vec![],
            reasons: vec!["fast".into()],
        },
    ];

    let gates = vec![GateResult {
        name: "invariant".into(),
        passed: true,
        evidence: vec![],
    }];

    let report = engine.evaluate(&rubric, scores, gates).unwrap();

    // (80*500000 + 100*300000 + 90*200000) / (500000+300000+200000)
    // = (40000000 + 30000000 + 18000000) / 1000000
    // = 88000000 / 1000000 = 88.0 → 88_000 millis
    assert_eq!(report.weighted_total_millis, Some(88_000));
    assert_eq!(report.evidence_coverage_millis, 1000);
    assert_eq!(report.confidence_millis, 1000);
    assert!(report.eligible);
    assert_eq!(report.rubric, RubricId("test".into()));
    assert_eq!(report.rubric_version, 1);
}

// ---------------------------------------------------------------------------
// Test 2: one-unknown
// ---------------------------------------------------------------------------

#[test]
fn one_unknown_dimension() {
    let engine = DeterministicEvaluator::new();
    let rubric = make_rubric();

    let scores = vec![
        DimensionScore {
            name: "goal".into(),
            value: DimensionValue::Scored(100),
            weight_millis: 500_000,
            evidence: vec![],
            reasons: vec!["done".into()],
        },
        DimensionScore {
            name: "safety".into(),
            value: DimensionValue::Unknown,
            weight_millis: 300_000,
            evidence: vec![],
            reasons: vec!["no data".into()],
        },
        DimensionScore {
            name: "efficiency".into(),
            value: DimensionValue::Scored(50),
            weight_millis: 200_000,
            evidence: vec![],
            reasons: vec!["slow".into()],
        },
    ];

    let gates = vec![GateResult {
        name: "invariant".into(),
        passed: true,
        evidence: vec![],
    }];

    let report = engine.evaluate(&rubric, scores, gates).unwrap();

    // Safety is unknown, excluded from denominator
    // (100*500000 + 50*200000) / (500000+200000)
    // = (50000000 + 10000000) / 700000 = 60000000 / 700000 ≈ 85.714...
    let wt = report.weighted_total_millis.unwrap();
    assert!((85710..85720).contains(&wt), "got {}", wt);

    // 2 applicable out of 3 → 666
    assert_eq!(report.evidence_coverage_millis, 666);
    assert_eq!(report.confidence_millis, 666);

    // Mandatory (goal) is applicable, gate passes, weighted exists
    assert!(report.eligible);
}

// ---------------------------------------------------------------------------
// Test 3: zero-applicable
// ---------------------------------------------------------------------------

#[test]
fn zero_applicable_dimensions() {
    let engine = DeterministicEvaluator::new();
    let rubric = make_rubric();

    let scores = vec![
        DimensionScore {
            name: "goal".into(),
            value: DimensionValue::Unknown,
            weight_millis: 500_000,
            evidence: vec![],
            reasons: vec!["none".into()],
        },
        DimensionScore {
            name: "safety".into(),
            value: DimensionValue::Unknown,
            weight_millis: 300_000,
            evidence: vec![],
            reasons: vec!["none".into()],
        },
        DimensionScore {
            name: "efficiency".into(),
            value: DimensionValue::Unknown,
            weight_millis: 200_000,
            evidence: vec![],
            reasons: vec!["none".into()],
        },
    ];

    let gates = vec![GateResult {
        name: "invariant".into(),
        passed: true,
        evidence: vec![],
    }];

    let report = engine.evaluate(&rubric, scores, gates).unwrap();

    assert_eq!(report.weighted_total_millis, None);
    assert_eq!(report.evidence_coverage_millis, 0);
    assert_eq!(report.confidence_millis, 0);
    // Gates pass but no weighted total → not eligible
    assert!(!report.eligible);
}

// ---------------------------------------------------------------------------
// Test 4: failed hard gate
// ---------------------------------------------------------------------------

#[test]
fn failed_hard_gate_with_high_score() {
    let engine = DeterministicEvaluator::new();

    // Custom rubric where only goal has weight, with a gate
    let rubric = Rubric {
        id: "test".into(),
        version: 1,
        dimensions: vec![
            RubricDimension {
                name: "goal".into(),
                weight_millis: 1_000_000,
                mandatory: true,
            },
            RubricDimension {
                name: "safety".into(),
                weight_millis: 0,
                mandatory: false,
            },
            RubricDimension {
                name: "efficiency".into(),
                weight_millis: 0,
                mandatory: false,
            },
        ],
        gates: vec![RubricGate {
            name: "invariant".into(),
            description: "system invariant holds".into(),
        }],
    };

    let scores = vec![
        DimensionScore {
            name: "goal".into(),
            value: DimensionValue::Scored(95),
            weight_millis: 1_000_000,
            evidence: vec![],
            reasons: vec!["great".into()],
        },
        DimensionScore {
            name: "safety".into(),
            value: DimensionValue::Scored(100),
            weight_millis: 0,
            evidence: vec![],
            reasons: vec!["safe".into()],
        },
        DimensionScore {
            name: "efficiency".into(),
            value: DimensionValue::Scored(100),
            weight_millis: 0,
            evidence: vec![],
            reasons: vec!["fast".into()],
        },
    ];

    let gates = vec![GateResult {
        name: "invariant".into(),
        passed: false,
        evidence: vec![],
    }];

    let report = engine.evaluate(&rubric, scores, gates).unwrap();

    // Score is 95 but gate failed
    assert_eq!(report.weighted_total_millis, Some(95_000));
    assert!(!report.eligible);
    assert!(!report.gates.iter().any(|g| g.passed));
}

// ---------------------------------------------------------------------------
// Test 5: missing evidence
// ---------------------------------------------------------------------------

#[test]
fn missing_evidence_dimension_becomes_unknown() {
    let engine = DeterministicEvaluator::new();
    let rubric = make_rubric();

    // When there is no evidence for a dimension, it should be scored as Unknown
    let scores = vec![
        DimensionScore {
            name: "goal".into(),
            value: DimensionValue::Scored(70),
            weight_millis: 500_000,
            evidence: vec![],
            reasons: vec!["partial".into()],
        },
        DimensionScore {
            name: "safety".into(),
            value: DimensionValue::Unknown,
            weight_millis: 300_000,
            evidence: vec![],
            reasons: vec!["no safety log found".into()],
        },
        DimensionScore {
            name: "efficiency".into(),
            value: DimensionValue::Scored(80),
            weight_millis: 200_000,
            evidence: vec![],
            reasons: vec!["ok".into()],
        },
    ];

    let gates = vec![GateResult {
        name: "invariant".into(),
        passed: true,
        evidence: vec![],
    }];

    let report = engine.evaluate(&rubric, scores, gates).unwrap();

    let wt = report.weighted_total_millis.unwrap();
    // (70*500000 + 80*200000) / 700000 = (35000000 + 16000000) / 700000 ≈ 72857
    assert!((72850..72860).contains(&wt), "got {}", wt);

    assert_eq!(report.evidence_coverage_millis, 666);
    assert!(report.eligible);
}

// ---------------------------------------------------------------------------
// Test 6: weight overflow rejection
// ---------------------------------------------------------------------------

#[test]
fn weight_overflow_rejected() {
    let engine = DeterministicEvaluator::new();

    // Use u32::MAX weight: score (100) * weight (u32::MAX) > u64
    let rubric = Rubric {
        id: "overflow".into(),
        version: 1,
        dimensions: vec![RubricDimension {
            name: "huge".into(),
            weight_millis: u32::MAX,
            mandatory: false,
        }],
        gates: vec![],
    };

    let scores = vec![DimensionScore {
        name: "huge".into(),
        value: DimensionValue::Scored(100),
        weight_millis: u32::MAX,
        evidence: vec![],
        reasons: vec!["overflow".into()],
    }];

    let result = engine.evaluate(&rubric, scores, vec![]);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("overflow"),
        "expected overflow, got: {}",
        err_msg
    );
}

// ---------------------------------------------------------------------------
// Additional edge case tests
// ---------------------------------------------------------------------------

#[test]
fn unknown_dimension_not_in_rubric_is_rejected() {
    let engine = DeterministicEvaluator::new();
    let rubric = make_rubric();

    let scores = vec![DimensionScore {
        name: "not_in_rubric".into(),
        value: DimensionValue::Scored(50),
        weight_millis: 100_000,
        evidence: vec![],
        reasons: vec![],
    }];

    let result = engine.evaluate(&rubric, scores, vec![]);
    assert!(result.is_err());
}

#[test]
fn rubric_dimension_missing_score_is_rejected() {
    let engine = DeterministicEvaluator::new();
    let rubric = make_rubric();

    let scores = vec![DimensionScore {
        name: "goal".into(),
        value: DimensionValue::Scored(50),
        weight_millis: 500_000,
        evidence: vec![],
        reasons: vec![],
    }];

    let result = engine.evaluate(
        &rubric,
        scores,
        vec![GateResult {
            name: "invariant".into(),
            passed: true,
            evidence: vec![],
        }],
    );
    assert!(result.is_err());
}

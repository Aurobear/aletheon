use fabric::types::expected_outcome::{ExpectedOutcome, OutcomeContractError, OutcomePredicate};

#[test]
fn valid_equals_predicate() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Equals {
            path: "mode".into(),
            value: serde_json::json!("stance"),
        },
        freshness_ms: 500,
        stable_window_ms: 200,
        timeout_ms: 5000,
    };
    assert!(eo.validate().is_ok());
}

#[test]
fn empty_path_rejected() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Equals {
            path: "".into(),
            value: serde_json::json!("x"),
        },
        freshness_ms: 500,
        stable_window_ms: 200,
        timeout_ms: 5000,
    };
    assert!(matches!(
        eo.validate(),
        Err(OutcomeContractError::EmptyPath)
    ));
}

#[test]
fn nan_rejected_in_range() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Range {
            path: "x".into(),
            min: Some(f64::NAN),
            max: None,
        },
        freshness_ms: 500,
        stable_window_ms: 200,
        timeout_ms: 5000,
    };
    assert!(matches!(eo.validate(), Err(OutcomeContractError::NaNValue)));
}

#[test]
fn infinity_rejected_in_range() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Range {
            path: "x".into(),
            min: None,
            max: Some(f64::INFINITY),
        },
        freshness_ms: 500,
        stable_window_ms: 200,
        timeout_ms: 5000,
    };
    assert!(matches!(
        eo.validate(),
        Err(OutcomeContractError::InfinityValue)
    ));
}

#[test]
fn depth_nine_rejected() {
    let mut inner = OutcomePredicate::Equals {
        path: "x".into(),
        value: serde_json::json!(1),
    };
    for _ in 0..9 {
        inner = OutcomePredicate::All {
            predicates: vec![inner],
        };
    }
    let eo = ExpectedOutcome {
        predicate: inner,
        freshness_ms: 500,
        stable_window_ms: 200,
        timeout_ms: 5000,
    };
    assert!(matches!(
        eo.validate(),
        Err(OutcomeContractError::DepthExceeded(_))
    ));
}

#[test]
fn over_64_nodes_rejected() {
    let mut preds = Vec::new();
    for i in 0..65 {
        preds.push(OutcomePredicate::Equals {
            path: format!("field_{i}"),
            value: serde_json::json!(i),
        });
    }
    let all = OutcomePredicate::All { predicates: preds };
    let eo = ExpectedOutcome {
        predicate: all,
        freshness_ms: 500,
        stable_window_ms: 200,
        timeout_ms: 5000,
    };
    assert!(matches!(
        eo.validate(),
        Err(OutcomeContractError::NodeCountExceeded(_))
    ));
}

#[test]
fn empty_all_rejected() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::All { predicates: vec![] },
        freshness_ms: 500,
        stable_window_ms: 200,
        timeout_ms: 5000,
    };
    assert!(matches!(eo.validate(), Err(OutcomeContractError::EmptyAll)));
}

#[test]
fn range_predicate_serde_roundtrip() {
    let pred = OutcomePredicate::Range {
        path: "x".into(),
        min: Some(0.0),
        max: Some(10.0),
    };
    let json = serde_json::to_string(&pred).unwrap();
    let back: OutcomePredicate = serde_json::from_str(&json).unwrap();
    assert_eq!(pred, back);
}

#[test]
fn change_predicate_serde_roundtrip() {
    let pred = OutcomePredicate::Change {
        path: "x".into(),
        min_delta: Some(0.5),
        max_delta: None,
    };
    let json = serde_json::to_string(&pred).unwrap();
    let back: OutcomePredicate = serde_json::from_str(&json).unwrap();
    assert_eq!(pred, back);
}

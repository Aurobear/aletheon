//! Deterministic expected-outcome predicates for embodied skill verification.
//!
//! Predicates are evaluated against JSON observation payloads via dot-path
//! traversal. No scripting, regex, JSONPath, or natural-language judgment.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpectedOutcome {
    pub predicate: OutcomePredicate,
    /// Observation must be fresher than this many milliseconds.
    pub freshness_ms: u64,
    /// The predicate must hold for this many consecutive matching observations.
    pub stable_window_ms: u64,
    /// Maximum wait for a matching observation before timing out.
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutcomePredicate {
    Equals {
        path: String,
        value: serde_json::Value,
    },
    NotEquals {
        path: String,
        value: serde_json::Value,
    },
    Range {
        path: String,
        /// None = unbounded below.
        min: Option<f64>,
        /// None = unbounded above.
        max: Option<f64>,
    },
    Change {
        path: String,
        /// Minimum required delta.
        min_delta: Option<f64>,
        /// Maximum allowed delta.
        max_delta: Option<f64>,
    },
    All {
        predicates: Vec<OutcomePredicate>,
    },
    Any {
        predicates: Vec<OutcomePredicate>,
    },
}

/// Validation errors for expected outcomes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutcomeContractError {
    #[error("empty and-point path")]
    EmptyPath,
    #[error("predicate depth {0} exceeds maximum 8")]
    DepthExceeded(usize),
    #[error("predicate node count {0} exceeds maximum 64")]
    NodeCountExceeded(usize),
    #[error("NaN value in predicate")]
    NaNValue,
    #[error("infinity value in predicate")]
    InfinityValue,
    #[error("empty All predicate list")]
    EmptyAll,
    #[error("empty Any predicate list")]
    EmptyAny,
}

pub const MAX_PREDICATE_DEPTH: usize = 8;
pub const MAX_PREDICATE_NODES: usize = 64;

impl ExpectedOutcome {
    /// Validate depth, node count, NaN/Infinity, and empty lists.
    pub fn validate(&self) -> Result<(), OutcomeContractError> {
        validate_predicate(&self.predicate)?;
        Ok(())
    }
}

fn validate_predicate(p: &OutcomePredicate) -> Result<(usize, usize), OutcomeContractError> {
    match p {
        OutcomePredicate::Equals { path, value } | OutcomePredicate::NotEquals { path, value } => {
            if path.is_empty() {
                return Err(OutcomeContractError::EmptyPath);
            }
            check_numeric(value)?;
            Ok((1, 1))
        }
        OutcomePredicate::Range { path, min, max } => {
            if path.is_empty() {
                return Err(OutcomeContractError::EmptyPath);
            }
            if let Some(v) = min {
                if v.is_nan() {
                    return Err(OutcomeContractError::NaNValue);
                }
                if v.is_infinite() {
                    return Err(OutcomeContractError::InfinityValue);
                }
            }
            if let Some(v) = max {
                if v.is_nan() {
                    return Err(OutcomeContractError::NaNValue);
                }
                if v.is_infinite() {
                    return Err(OutcomeContractError::InfinityValue);
                }
            }
            Ok((1, 1))
        }
        OutcomePredicate::Change {
            path,
            min_delta,
            max_delta,
        } => {
            if path.is_empty() {
                return Err(OutcomeContractError::EmptyPath);
            }
            if let Some(v) = min_delta {
                if v.is_nan() {
                    return Err(OutcomeContractError::NaNValue);
                }
                if v.is_infinite() {
                    return Err(OutcomeContractError::InfinityValue);
                }
            }
            if let Some(v) = max_delta {
                if v.is_nan() {
                    return Err(OutcomeContractError::NaNValue);
                }
                if v.is_infinite() {
                    return Err(OutcomeContractError::InfinityValue);
                }
            }
            Ok((1, 1))
        }
        OutcomePredicate::All { predicates } => {
            if predicates.is_empty() {
                return Err(OutcomeContractError::EmptyAll);
            }
            let mut max_depth = 0usize;
            let mut total_nodes = 0usize;
            for child in predicates {
                let (d, n) = validate_predicate(child)?;
                max_depth = max_depth.max(d);
                total_nodes += n;
            }
            let depth = max_depth + 1;
            let nodes = total_nodes + 1;
            if depth > MAX_PREDICATE_DEPTH {
                return Err(OutcomeContractError::DepthExceeded(depth));
            }
            if nodes > MAX_PREDICATE_NODES {
                return Err(OutcomeContractError::NodeCountExceeded(nodes));
            }
            Ok((depth, nodes))
        }
        OutcomePredicate::Any { predicates } => {
            if predicates.is_empty() {
                return Err(OutcomeContractError::EmptyAny);
            }
            let mut max_depth = 0usize;
            let mut total_nodes = 0usize;
            for child in predicates {
                let (d, n) = validate_predicate(child)?;
                max_depth = max_depth.max(d);
                total_nodes += n;
            }
            let depth = max_depth + 1;
            let nodes = total_nodes + 1;
            if depth > MAX_PREDICATE_DEPTH {
                return Err(OutcomeContractError::DepthExceeded(depth));
            }
            if nodes > MAX_PREDICATE_NODES {
                return Err(OutcomeContractError::NodeCountExceeded(nodes));
            }
            Ok((depth, nodes))
        }
    }
}

fn check_numeric(value: &serde_json::Value) -> Result<(), OutcomeContractError> {
    if let Some(n) = value.as_f64() {
        if n.is_nan() {
            return Err(OutcomeContractError::NaNValue);
        }
        if n.is_infinite() {
            return Err(OutcomeContractError::InfinityValue);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
                path: format!("field_{}", i),
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
}

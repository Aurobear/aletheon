//! Deterministic expected-outcome predicates for embodied skill verification.
//!
//! Predicates are evaluated against JSON observation payloads via dot-path
//! traversal. No scripting, regex, JSONPath, or natural-language judgment.

use crate::types::world_state::WorldSnapshot;
use crate::MonoTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

/// Outcome of evaluating an expected outcome against an observation snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutcomeMatch {
    /// Predicate satisfied within the freshness window.
    Match,
    /// Predicate not satisfied.
    Mismatch { reason: String },
    /// Snapshot is stale or older than the freshness window — unusable evidence.
    Stale,
}

/// Dot-path traversal into a JSON value (`a.b.c`). Returns `None` for an
/// unknown or non-object path segment. Empty segments are rejected.
pub fn get_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for key in path.split('.') {
        if key.is_empty() {
            return None;
        }
        current = current.get(key)?;
    }
    Some(current)
}

/// Evaluate a predicate against a payload. `before` is required only for
/// `Change` predicates (delta between before and after payloads).
pub fn evaluate_predicate(
    predicate: &OutcomePredicate,
    payload: &Value,
    before: Option<&Value>,
) -> bool {
    match predicate {
        OutcomePredicate::Equals { path, value } => {
            get_path(payload, path).map(|v| v == value).unwrap_or(false)
        }
        OutcomePredicate::NotEquals { path, value } => {
            get_path(payload, path).map(|v| v != value).unwrap_or(true)
        }
        OutcomePredicate::Range { path, min, max } => get_path(payload, path)
            .and_then(|v| v.as_f64())
            .map(|n| {
                let within_low = min.map(|lo| n >= lo).unwrap_or(true);
                let within_high = max.map(|hi| n <= hi).unwrap_or(true);
                within_low && within_high
            })
            .unwrap_or(false),
        OutcomePredicate::Change {
            path,
            min_delta,
            max_delta,
        } => {
            let after = get_path(payload, path).and_then(|v| v.as_f64());
            let before = before
                .and_then(|b| get_path(b, path))
                .and_then(|v| v.as_f64());
            match (before, after) {
                (Some(a), Some(b)) => {
                    let delta = b - a;
                    let meets_min = min_delta.map(|lo| delta >= lo).unwrap_or(true);
                    let meets_max = max_delta.map(|hi| delta <= hi).unwrap_or(true);
                    meets_min && meets_max
                }
                _ => false,
            }
        }
        OutcomePredicate::All { predicates } => predicates
            .iter()
            .all(|p| evaluate_predicate(p, payload, before)),
        OutcomePredicate::Any { predicates } => predicates
            .iter()
            .any(|p| evaluate_predicate(p, payload, before)),
    }
}

/// Evaluate an expected outcome against a snapshot, honoring staleness and
/// the freshness window. `before` is required only for `Change` predicates.
pub fn evaluate_expected(
    expected: &ExpectedOutcome,
    snapshot: &WorldSnapshot,
    before: Option<&WorldSnapshot>,
    now: MonoTime,
) -> OutcomeMatch {
    if snapshot.stale {
        return OutcomeMatch::Stale;
    }
    let age_ms = now.0.saturating_sub(snapshot.observed_at.0);
    if age_ms > expected.freshness_ms {
        return OutcomeMatch::Stale;
    }
    if evaluate_predicate(
        &expected.predicate,
        &snapshot.payload,
        before.map(|b| &b.payload),
    ) {
        OutcomeMatch::Match
    } else {
        OutcomeMatch::Mismatch {
            reason: "expected outcome predicate not satisfied".into(),
        }
    }
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

    fn snapshot(seq: u64, observed_at: u64, stale: bool, payload: Value) -> WorldSnapshot {
        WorldSnapshot {
            device: crate::types::embodiment::DeviceId("bot".into()),
            schema: "robot.state/v1".into(),
            sequence: seq,
            payload,
            observed_at: MonoTime(observed_at),
            stale,
        }
    }

    #[test]
    fn get_path_traverses_nested_objects() {
        let payload = serde_json::json!({"robot": {"base": {"height_m": 0.8}}});
        assert_eq!(
            get_path(&payload, "robot.base.height_m").and_then(|v| v.as_f64()),
            Some(0.8)
        );
        assert!(get_path(&payload, "robot.missing").is_none());
        assert!(get_path(&payload, "robot.base.height_m.extra").is_none());
        assert!(get_path(&payload, "robot..base").is_none());
    }

    #[test]
    fn evaluate_equals_range_and_all() {
        let payload = serde_json::json!({"mode": "stance", "fall_detected": false, "base": {"height_m": 0.8}});
        let expected = ExpectedOutcome {
            predicate: OutcomePredicate::All {
                predicates: vec![
                    OutcomePredicate::Equals {
                        path: "mode".into(),
                        value: serde_json::json!("stance"),
                    },
                    OutcomePredicate::Equals {
                        path: "fall_detected".into(),
                        value: serde_json::json!(false),
                    },
                    OutcomePredicate::Range {
                        path: "base.height_m".into(),
                        min: Some(0.75),
                        max: Some(0.95),
                    },
                ],
            },
            freshness_ms: 500,
            stable_window_ms: 0,
            timeout_ms: 10_000,
        };
        let snap = snapshot(1, 0, false, payload);
        assert_eq!(evaluate_expected(&expected, &snap, None, MonoTime(100)), OutcomeMatch::Match);
    }

    #[test]
    fn evaluate_mismatch_and_stale() {
        let expected = ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("walk"),
            },
            freshness_ms: 500,
            stable_window_ms: 0,
            timeout_ms: 10_000,
        };
        let snap = snapshot(1, 0, false, serde_json::json!({"mode": "stance"}));
        assert!(matches!(
            evaluate_expected(&expected, &snap, None, MonoTime(100)),
            OutcomeMatch::Mismatch { .. }
        ));
        // Snapshot older than freshness window.
        assert!(matches!(
            evaluate_expected(&expected, &snap, None, MonoTime(1_000)),
            OutcomeMatch::Stale
        ));
        // Snapshot explicitly stale.
        let stale = snapshot(1, 0, true, serde_json::json!({"mode": "walk"}));
        assert!(matches!(
            evaluate_expected(&expected, &stale, None, MonoTime(0)),
            OutcomeMatch::Stale
        ));
    }

    #[test]
    fn evaluate_change_uses_before_delta() {
        let expected = ExpectedOutcome {
            predicate: OutcomePredicate::Change {
                path: "base.height_m".into(),
                min_delta: Some(0.1),
                max_delta: None,
            },
            freshness_ms: 500,
            stable_window_ms: 0,
            timeout_ms: 10_000,
        };
        let before = snapshot(0, 0, false, serde_json::json!({"base": {"height_m": 0.5}}));
        let after = snapshot(1, 1, false, serde_json::json!({"base": {"height_m": 0.7}}));
        assert_eq!(
            evaluate_expected(&expected, &after, Some(&before), MonoTime(50)),
            OutcomeMatch::Match
        );
        let small = snapshot(1, 1, false, serde_json::json!({"base": {"height_m": 0.52}}));
        assert!(matches!(
            evaluate_expected(&expected, &small, Some(&before), MonoTime(50)),
            OutcomeMatch::Mismatch { .. }
        ));
    }
}

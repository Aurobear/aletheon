//! Deterministic outcome verification — evaluates OutcomePredicate against WorldSnapshot.
//! No scripting, regex, JSONPath, or natural-language judgment. Dot-path only.

use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
use fabric::types::outcome_verification::{VerificationDecision, VerificationReport};
use fabric::types::world_state::WorldSnapshot;

/// Evaluates a predicate tree against a before and after snapshot.
/// Returns a VerificationReport with a tagged decision.
pub fn evaluate(
    expected: &ExpectedOutcome,
    before: Option<&WorldSnapshot>,
    after: Option<&WorldSnapshot>,
    attempt_number: u32,
) -> VerificationReport {
    let after = match after {
        Some(s) if !s.stale => s,
        Some(_) => {
            return VerificationReport {
                decision: VerificationDecision::Unknown,
                evaluated_sequence: 0,
                observed_paths: vec![],
                reasons: vec!["after snapshot is stale".into()],
                evidence: vec![],
            };
        }
        None => {
            return VerificationReport {
                decision: VerificationDecision::Unknown,
                evaluated_sequence: 0,
                observed_paths: vec![],
                reasons: vec!["no after snapshot available".into()],
                evidence: vec![],
            };
        }
    };

    // Check for safety faults in the payload
    if let Some(fault) = after
        .payload
        .get("fault")
        .or_else(|| after.payload.get("safety_fault"))
    {
        let reason = format!("safety fault detected: {}", fault);
        return VerificationReport {
            decision: VerificationDecision::Unsafe,
            evaluated_sequence: after.sequence,
            observed_paths: vec!["fault".into()],
            reasons: vec![reason],
            evidence: vec![],
        };
    }

    let before_payload = before.map(|b| &b.payload);

    match eval_predicate(&expected.predicate, &after.payload, before_payload) {
        Ok(matched) => {
            if matched {
                VerificationReport {
                    decision: VerificationDecision::Matched,
                    evaluated_sequence: after.sequence,
                    observed_paths: collect_paths(&expected.predicate),
                    reasons: vec!["predicate satisfied".into()],
                    evidence: vec![],
                }
            } else {
                // Threshold: first failure → retryable; after retry → replannable
                let decision = if attempt_number < 2 {
                    VerificationDecision::RetryableMismatch
                } else {
                    VerificationDecision::ReplannableMismatch
                };
                VerificationReport {
                    decision,
                    evaluated_sequence: after.sequence,
                    observed_paths: collect_paths(&expected.predicate),
                    reasons: vec![format!(
                        "predicate not satisfied on attempt {}",
                        attempt_number
                    )],
                    evidence: vec![],
                }
            }
        }
        Err(err) => VerificationReport {
            decision: VerificationDecision::Unknown,
            evaluated_sequence: after.sequence,
            observed_paths: collect_paths(&expected.predicate),
            reasons: vec![err],
            evidence: vec![],
        },
    }
}

/// Evaluates a predicate against current state, with optional previous state for Change predicates.
fn eval_predicate(
    pred: &OutcomePredicate,
    current: &serde_json::Value,
    before: Option<&serde_json::Value>,
) -> Result<bool, String> {
    match pred {
        OutcomePredicate::Equals { path, value } => {
            let observed = dot_get(current, path)?;
            Ok(&observed == value)
        }
        OutcomePredicate::NotEquals { path, value } => {
            let observed = dot_get(current, path)?;
            Ok(&observed != value)
        }
        OutcomePredicate::Range { path, min, max } => {
            let observed = dot_get(current, path)?;
            let n = observed
                .as_f64()
                .ok_or_else(|| format!("path '{}' is not numeric: {}", path, observed))?;
            if let Some(min_val) = min {
                if n < *min_val {
                    return Ok(false);
                }
            }
            if let Some(max_val) = max {
                if n > *max_val {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        OutcomePredicate::Change {
            path,
            min_delta,
            max_delta,
        } => {
            let current_val = dot_get(current, path)?;
            let current_n = current_val.as_f64().ok_or_else(|| {
                format!("path '{}' current is not numeric: {}", path, current_val)
            })?;

            let before_val = match before {
                Some(b) => dot_get(b, path)?,
                None => {
                    return Err(format!(
                        "Change predicate requires before snapshot for path '{}'",
                        path
                    ))
                }
            };
            let before_n = before_val
                .as_f64()
                .ok_or_else(|| format!("path '{}' before is not numeric: {}", path, before_val))?;

            let delta = current_n - before_n;

            if let Some(min) = min_delta {
                if delta < *min {
                    return Ok(false);
                }
            }
            if let Some(max) = max_delta {
                if delta > *max {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        OutcomePredicate::All { predicates } => {
            for child in predicates {
                if !eval_predicate(child, current, before)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        OutcomePredicate::Any { predicates } => {
            let mut reasons = Vec::new();
            for child in predicates {
                match eval_predicate(child, current, before) {
                    Ok(true) => return Ok(true),
                    Err(e) => reasons.push(e),
                    _ => {}
                }
            }
            if reasons.len() == predicates.len() {
                Err(format!("Any: all predicates errored: {:?}", reasons))
            } else {
                Ok(false)
            }
        }
    }
}

/// Traverse a JSON object by dot-separated path. Arrays and escaped expressions are unsupported.
fn dot_get<'a>(value: &'a serde_json::Value, path: &str) -> Result<serde_json::Value, String> {
    let mut current = value;
    for segment in path.split('.') {
        match current {
            serde_json::Value::Object(map) => {
                current = map
                    .get(segment)
                    .ok_or_else(|| format!("path segment '{}' not found in {}", segment, path))?;
            }
            _ => {
                return Err(format!(
                    "path segment '{}' is not an object in path '{}'",
                    segment, path
                ))
            }
        }
    }
    Ok(current.clone())
}

/// Collect all dot-paths referenced in a predicate tree.
fn collect_paths(pred: &OutcomePredicate) -> Vec<String> {
    match pred {
        OutcomePredicate::Equals { path, .. }
        | OutcomePredicate::NotEquals { path, .. }
        | OutcomePredicate::Range { path, .. }
        | OutcomePredicate::Change { path, .. } => {
            vec![path.clone()]
        }
        OutcomePredicate::All { predicates } | OutcomePredicate::Any { predicates } => {
            let mut paths = Vec::new();
            for child in predicates {
                paths.extend(collect_paths(child));
            }
            paths.sort();
            paths.dedup();
            paths
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(payload: serde_json::Value, seq: u64, stale: bool) -> WorldSnapshot {
        WorldSnapshot {
            device: fabric::types::embodiment::DeviceId("bot".into()),
            schema: "test".into(),
            sequence: seq,
            payload,
            observed_at: fabric::MonoTime(seq),
            stale,
        }
    }

    #[test]
    fn equals_matches_when_value_equal() {
        let eo = ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("stance"),
            },
            freshness_ms: 500,
            stable_window_ms: 200,
            timeout_ms: 5000,
        };
        let after = snapshot(serde_json::json!({"mode": "stance"}), 1, false);
        let report = evaluate(&eo, None, Some(&after), 1);
        assert_eq!(report.decision, VerificationDecision::Matched);
    }

    #[test]
    fn equals_mismatch_on_wrong_value() {
        let eo = ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("stance"),
            },
            freshness_ms: 500,
            stable_window_ms: 200,
            timeout_ms: 5000,
        };
        let after = snapshot(serde_json::json!({"mode": "walking"}), 1, false);
        let report = evaluate(&eo, None, Some(&after), 1);
        assert_eq!(report.decision, VerificationDecision::RetryableMismatch);
    }

    #[test]
    fn mismatch_after_retry_becomes_replannable() {
        let eo = ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "x".into(),
                value: serde_json::json!(1),
            },
            freshness_ms: 500,
            stable_window_ms: 200,
            timeout_ms: 5000,
        };
        let after = snapshot(serde_json::json!({"x": 2}), 1, false);
        // Attempt 2 (third try) → ReplannableMismatch
        let report = evaluate(&eo, None, Some(&after), 2);
        assert_eq!(report.decision, VerificationDecision::ReplannableMismatch);
    }

    #[test]
    fn range_inclusive_bounds() {
        let eo = ExpectedOutcome {
            predicate: OutcomePredicate::Range {
                path: "x".into(),
                min: Some(0.0),
                max: Some(10.0),
            },
            freshness_ms: 500,
            stable_window_ms: 200,
            timeout_ms: 5000,
        };
        let after = snapshot(serde_json::json!({"x": 5.0}), 1, false);
        assert_eq!(
            evaluate(&eo, None, Some(&after), 1).decision,
            VerificationDecision::Matched
        );

        // boundary
        let after2 = snapshot(serde_json::json!({"x": 0.0}), 1, false);
        assert_eq!(
            evaluate(&eo, None, Some(&after2), 1).decision,
            VerificationDecision::Matched
        );
    }

    #[test]
    fn range_out_of_bounds() {
        let eo = ExpectedOutcome {
            predicate: OutcomePredicate::Range {
                path: "x".into(),
                min: Some(0.0),
                max: Some(10.0),
            },
            freshness_ms: 500,
            stable_window_ms: 200,
            timeout_ms: 5000,
        };
        let after = snapshot(serde_json::json!({"x": 15.0}), 1, false);
        assert_eq!(
            evaluate(&eo, None, Some(&after), 1).decision,
            VerificationDecision::RetryableMismatch
        );
    }

    #[test]
    fn change_delta_detected() {
        let eo = ExpectedOutcome {
            predicate: OutcomePredicate::Change {
                path: "x".into(),
                min_delta: Some(1.0),
                max_delta: None,
            },
            freshness_ms: 500,
            stable_window_ms: 200,
            timeout_ms: 5000,
        };
        let before = snapshot(serde_json::json!({"x": 0.0}), 0, false);
        let after = snapshot(serde_json::json!({"x": 3.0}), 1, false);
        assert_eq!(
            evaluate(&eo, Some(&before), Some(&after), 1).decision,
            VerificationDecision::Matched
        );
    }

    #[test]
    fn change_no_delta_is_mismatch() {
        let eo = ExpectedOutcome {
            predicate: OutcomePredicate::Change {
                path: "x".into(),
                min_delta: Some(1.0),
                max_delta: None,
            },
            freshness_ms: 500,
            stable_window_ms: 200,
            timeout_ms: 5000,
        };
        let before = snapshot(serde_json::json!({"x": 0.0}), 0, false);
        let after = snapshot(serde_json::json!({"x": 0.5}), 1, false);
        assert_eq!(
            evaluate(&eo, Some(&before), Some(&after), 1).decision,
            VerificationDecision::RetryableMismatch
        );
    }

    #[test]
    fn all_nested_predicates() {
        let eo = ExpectedOutcome {
            predicate: OutcomePredicate::All {
                predicates: vec![
                    OutcomePredicate::Equals {
                        path: "mode".into(),
                        value: serde_json::json!("stance"),
                    },
                    OutcomePredicate::Range {
                        path: "x".into(),
                        min: Some(0.0),
                        max: Some(5.0),
                    },
                ],
            },
            freshness_ms: 500,
            stable_window_ms: 200,
            timeout_ms: 5000,
        };
        let after = snapshot(serde_json::json!({"mode": "stance", "x": 2.0}), 1, false);
        assert_eq!(
            evaluate(&eo, None, Some(&after), 1).decision,
            VerificationDecision::Matched
        );
    }

    #[test]
    fn any_predicate_one_matches() {
        let eo = ExpectedOutcome {
            predicate: OutcomePredicate::Any {
                predicates: vec![
                    OutcomePredicate::Equals {
                        path: "mode".into(),
                        value: serde_json::json!("stance"),
                    },
                    OutcomePredicate::Equals {
                        path: "mode".into(),
                        value: serde_json::json!("walk"),
                    },
                ],
            },
            freshness_ms: 500,
            stable_window_ms: 200,
            timeout_ms: 5000,
        };
        let after = snapshot(serde_json::json!({"mode": "walk"}), 1, false);
        assert_eq!(
            evaluate(&eo, None, Some(&after), 1).decision,
            VerificationDecision::Matched
        );
    }

    #[test]
    fn stale_snapshot_is_unknown() {
        let eo = ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "x".into(),
                value: serde_json::json!(1),
            },
            freshness_ms: 500,
            stable_window_ms: 200,
            timeout_ms: 5000,
        };
        let after = snapshot(serde_json::json!({"x": 1}), 1, true);
        assert_eq!(
            evaluate(&eo, None, Some(&after), 1).decision,
            VerificationDecision::Unknown
        );
    }

    #[test]
    fn fault_field_maps_to_unsafe() {
        let eo = ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("stance"),
            },
            freshness_ms: 500,
            stable_window_ms: 200,
            timeout_ms: 5000,
        };
        let after = snapshot(serde_json::json!({"fault": "motor_stall"}), 1, false);
        assert_eq!(
            evaluate(&eo, None, Some(&after), 1).decision,
            VerificationDecision::Unsafe
        );
    }

    #[test]
    fn missing_path_is_unknown() {
        let eo = ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "nonexistent".into(),
                value: serde_json::json!(1),
            },
            freshness_ms: 500,
            stable_window_ms: 200,
            timeout_ms: 5000,
        };
        let after = snapshot(serde_json::json!({"x": 1}), 1, false);
        assert_eq!(
            evaluate(&eo, None, Some(&after), 1).decision,
            VerificationDecision::Unknown
        );
    }

    #[test]
    fn type_mismatch_path_is_unknown() {
        let eo = ExpectedOutcome {
            predicate: OutcomePredicate::Range {
                path: "x".into(),
                min: Some(0.0),
                max: None,
            },
            freshness_ms: 500,
            stable_window_ms: 200,
            timeout_ms: 5000,
        };
        let after = snapshot(serde_json::json!({"x": "not_a_number"}), 1, false);
        assert_eq!(
            evaluate(&eo, None, Some(&after), 1).decision,
            VerificationDecision::Unknown
        );
    }
}

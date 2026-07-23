//! Evaluator — validates candidate genomes before migration.
//!
//! Two levels of evaluation:
//! 1. Genome-level: checks safety weight, immutable rules, adjustment magnitude
//! 2. ABI-level: evaluates RuntimeCandidate after sandbox testing

use crate::core::types::EvaluationResult;
use crate::genome::model::{Genome, GenomeMeta};
use anyhow::Result;
use fabric::{meta::Recommendation, Evaluation, RuntimeCandidate, TestResult};

/// Safety threshold below which a candidate is rejected.
const SAFETY_THRESHOLD: f64 = 0.8;
/// Maximum allowed adjustment magnitude per change.
const MAX_ADJUSTMENT_MAGNITUDE: f64 = 0.2;

/// Evaluates runtimes and genomes based on safety and stability criteria.
pub struct Evaluator {
    /// Minimum safety weight that must be maintained.
    safety_floor: f64,
    /// Maximum weight adjustment allowed per step.
    max_adjustment_per_step: f64,
}

impl Evaluator {
    pub fn new() -> Self {
        Self {
            safety_floor: SAFETY_THRESHOLD,
            max_adjustment_per_step: 0.1,
        }
    }

    pub fn with_config(safety_floor: f64, max_adjustment_per_step: f64) -> Self {
        Self {
            safety_floor,
            max_adjustment_per_step,
        }
    }

    /// Evaluate a candidate genome against the current genome.
    ///
    /// Checks:
    /// 1. Safety weight >= safety_floor
    /// 2. No immutable boundary rules (priority >= 200) removed
    /// 3. Care weight adjustments <= max_adjustment_per_step
    pub fn evaluate_candidate(&self, candidate: &Genome, current: &Genome) -> EvaluationResult {
        let mut reasons = Vec::new();
        let mut passed = true;

        // Check 1: safety weight >= safety_floor
        let safety_weight = candidate
            .care
            .priorities
            .iter()
            .find(|p| p.topic == "safety" || p.topic == "user_safety")
            .map(|p| p.weight)
            .unwrap_or(0.0);

        if safety_weight < self.safety_floor {
            passed = false;
            reasons.push(format!(
                "Safety weight {} is below minimum {}",
                safety_weight, self.safety_floor
            ));
        }

        // Check 2: no immutable rules violated (priority >= 200 treated as immutable)
        let candidate_rule_ids: Vec<&str> = candidate
            .boundary
            .rules
            .iter()
            .map(|r| r.id.as_str())
            .collect();

        for rule in &current.boundary.rules {
            if rule.priority >= 200 && !candidate_rule_ids.contains(&rule.id.as_str()) {
                passed = false;
                reasons.push(format!(
                    "Immutable boundary rule '{}' (priority {}) was removed",
                    rule.id, rule.priority
                ));
            }
        }

        // Check 3: adjustment magnitude
        let current_care: std::collections::HashMap<&str, f64> = current
            .care
            .priorities
            .iter()
            .map(|p| (p.topic.as_str(), p.weight))
            .collect();
        let candidate_care: std::collections::HashMap<&str, f64> = candidate
            .care
            .priorities
            .iter()
            .map(|p| (p.topic.as_str(), p.weight))
            .collect();

        for (topic, &new_weight) in &candidate_care {
            if let Some(&old_weight) = current_care.get(topic) {
                let delta = (new_weight - old_weight).abs();
                if delta > self.max_adjustment_per_step {
                    passed = false;
                    reasons.push(format!(
                        "Care weight adjustment for '{}' is {:.3}, exceeds max {:.3}",
                        topic, delta, self.max_adjustment_per_step
                    ));
                }
            }
        }

        EvaluationResult { passed, reasons }
    }

    /// Evaluate a candidate genome against a GenomeMeta with evolution config.
    ///
    /// Uses the evolution config's safety_floor and max_adjustment values.
    pub fn evaluate_with_meta(
        &self,
        candidate: &Genome,
        current: &Genome,
        meta: &GenomeMeta,
    ) -> EvaluationResult {
        let mut reasons = Vec::new();
        let mut passed = true;

        // Check safety floor from evolution config
        let safety_weight = candidate
            .care
            .priorities
            .iter()
            .find(|p| p.topic == "safety" || p.topic == "user_safety")
            .map(|p| p.weight)
            .unwrap_or(0.0);

        if safety_weight < meta.evolution.safety_floor {
            passed = false;
            reasons.push(format!(
                "Safety weight {} is below evolution safety_floor {}",
                safety_weight, meta.evolution.safety_floor
            ));
        }

        // Check immutable rules from care_ext
        for rule in &meta.care_ext.boundary_rules {
            if rule.immutable {
                let current_match = current
                    .boundary
                    .rules
                    .iter()
                    .find(|r| r.id == rule.pattern || r.condition.contains(&rule.pattern));
                let candidate_match = candidate
                    .boundary
                    .rules
                    .iter()
                    .find(|r| r.id == rule.pattern || r.condition.contains(&rule.pattern));

                match (current_match, candidate_match) {
                    (Some(cur), Some(cand)) if cur.action != cand.action => {
                        passed = false;
                        reasons.push(format!(
                            "Immutable rule '{}' action changed from '{}' to '{}'",
                            rule.pattern, cur.action, cand.action
                        ));
                    }
                    (Some(_), None) => {
                        passed = false;
                        reasons.push(format!(
                            "Immutable rule '{}' was removed from candidate",
                            rule.pattern
                        ));
                    }
                    _ => {}
                }
            }
        }

        // Check adjustment magnitude against evolution config
        let current_care: std::collections::HashMap<&str, f64> = current
            .care
            .priorities
            .iter()
            .map(|p| (p.topic.as_str(), p.weight))
            .collect();
        let candidate_care: std::collections::HashMap<&str, f64> = candidate
            .care
            .priorities
            .iter()
            .map(|p| (p.topic.as_str(), p.weight))
            .collect();

        for (topic, &new_weight) in &candidate_care {
            if let Some(&old_weight) = current_care.get(topic) {
                let delta = (new_weight - old_weight).abs();
                if delta > meta.evolution.max_adjustment {
                    passed = false;
                    reasons.push(format!(
                        "Care weight adjustment for '{}' is {:.3}, exceeds evolution max {:.3}",
                        topic, delta, meta.evolution.max_adjustment
                    ));
                }
            }
        }

        EvaluationResult { passed, reasons }
    }

    /// Evaluate an ABI RuntimeCandidate after sandbox testing.
    ///
    /// Used by the morphogenesis pipeline.
    pub async fn evaluate(
        &self,
        candidate: &RuntimeCandidate,
        test: &TestResult,
    ) -> Result<Evaluation> {
        let mut strengths = Vec::new();
        let mut weaknesses = Vec::new();

        // Safety score: derived from test pass rate
        let safety_score = if test.tests_run == 0 {
            0.0
        } else {
            test.tests_passed as f64 / test.tests_run as f64
        };

        // Check for immutable rule violations
        let immutable_violated = test.failures.iter().any(|f| {
            let lower = f.to_lowercase();
            lower.contains("immutable") || lower.contains("identity") || lower.contains("core")
        });

        if immutable_violated {
            weaknesses.push("Immutable rule violation detected in test failures".to_string());
        }

        // Adjustment magnitude: number of changes normalized (cap at 5 for max)
        let adjustment_magnitude = (candidate.changes.len() as f64 / 5.0).min(1.0);
        if adjustment_magnitude > MAX_ADJUSTMENT_MAGNITUDE {
            weaknesses.push(format!(
                "Adjustment magnitude {adjustment_magnitude:.2} exceeds limit {MAX_ADJUSTMENT_MAGNITUDE:.2}"
            ));
        }

        // Strengths
        if safety_score >= SAFETY_THRESHOLD {
            strengths.push(format!("Safety score {safety_score:.2} meets threshold"));
        }
        if test.tests_passed > 0 {
            strengths.push(format!("{} tests passed", test.tests_passed));
        }
        if test.elapsed_ms < 5000 {
            strengths.push(format!("Completed in {}ms", test.elapsed_ms));
        }

        // Weaknesses from test failures
        if !test.passed {
            weaknesses.push(format!("{} test(s) failed", test.tests_failed));
        }
        if safety_score < SAFETY_THRESHOLD {
            weaknesses.push(format!(
                "Safety score {safety_score:.2} below threshold {SAFETY_THRESHOLD:.2}"
            ));
        }

        // Overall score: weighted combination
        let mut score = safety_score * 0.6;
        if !immutable_violated {
            score += 0.2;
        } else {
            score -= 0.3;
        }
        score += (1.0 - adjustment_magnitude) * 0.2;
        let score = score.clamp(0.0, 1.0);

        // Recommendation
        let recommendation = if immutable_violated {
            Recommendation::Reject
        } else if score >= SAFETY_THRESHOLD && adjustment_magnitude <= MAX_ADJUSTMENT_MAGNITUDE {
            Recommendation::Adopt
        } else if score >= 0.5 {
            Recommendation::NeedsMoreTesting
        } else {
            Recommendation::Reject
        };

        Ok(Evaluation {
            score,
            strengths,
            weaknesses,
            recommendation,
        })
    }
}

impl Default for Evaluator {
    fn default() -> Self {
        Self::new()
    }
}

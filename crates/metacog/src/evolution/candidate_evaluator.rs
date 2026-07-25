//! Evaluator — validates candidate genomes before migration.
//!
//! Two levels of evaluation:
//! 1. Genome-level: checks safety weight, immutable rules, adjustment magnitude
//! 2. ABI-level: evaluates RuntimeCandidate after sandbox testing

use anyhow::Result;
use fabric::{meta::Recommendation, Evaluation, RuntimeCandidate, TestResult};

/// Safety threshold below which a candidate is rejected.
const SAFETY_THRESHOLD: f64 = 0.8;
/// Maximum allowed adjustment magnitude per change.
const MAX_ADJUSTMENT_MAGNITUDE: f64 = 0.2;

/// Evaluates runtime candidates based on safety and stability criteria.
pub struct Evaluator;

impl Evaluator {
    pub fn new() -> Self {
        Self
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

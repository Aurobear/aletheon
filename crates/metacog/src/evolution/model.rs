//! Evolution data model — evaluator specs, metrics, and evaluation results.

use serde::{Deserialize, Serialize};

/// Evaluator specification — not in ABI yet, defined here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatorSpec {
    pub metrics: Vec<EvaluatorMetric>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatorMetric {
    pub name: String,
    pub weight: f64,
}

/// Result of evaluating a candidate genome.
#[derive(Debug, Clone)]
pub struct EvaluationResult {
    pub passed: bool,
    pub reasons: Vec<String>,
}

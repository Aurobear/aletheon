//! Versioned evaluation rubric definition.
//!
//! A rubric describes which dimensions and gates an evaluator must
//! assess when evaluating an experience.

use serde::{Deserialize, Serialize};

/// A versioned evaluation rubric.
///
/// A rubric defines the dimensions and hard gates that an evaluator
/// must apply when scoring an experience.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rubric {
    /// Stable rubric identifier.
    pub id: String,
    /// Monotonically increasing rubric version.
    pub version: u32,
    /// The dimensions that must be scored.
    pub dimensions: Vec<RubricDimension>,
    /// Mandatory hard gates.
    pub gates: Vec<RubricGate>,
}

/// A single dimension within a rubric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RubricDimension {
    /// Human-readable dimension name.
    pub name: String,
    /// Fixed-point weight: 1_000_000 = 1.0.
    pub weight_millis: u32,
    /// Whether this dimension is mandatory (must have evidence to be applicable).
    pub mandatory: bool,
}

/// A single hard gate within a rubric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RubricGate {
    /// Human-readable gate name.
    pub name: String,
    /// Description of the invariant being checked.
    pub description: String,
}

/// Canonical deterministic rubric for coding-v2 evaluation.
pub fn coding_v2_rubric() -> Rubric {
    Rubric {
        id: "coding-v2".into(),
        version: 2,
        dimensions: vec![
            dimension("requirement_coverage", 200_000, false),
            dimension("correctness", 250_000, true),
            dimension("scope_discipline", 150_000, true),
            dimension("maintainability", 100_000, false),
            dimension("verification_sufficiency", 200_000, true),
            dimension("regression_safety", 100_000, true),
        ],
        gates: vec![
            gate("required_verification_passed"),
            gate("change_within_scope"),
        ],
    }
}

fn dimension(name: &str, weight_millis: u32, mandatory: bool) -> RubricDimension {
    RubricDimension {
        name: name.into(),
        weight_millis,
        mandatory,
    }
}

fn gate(name: &str) -> RubricGate {
    RubricGate {
        name: name.into(),
        description: format!("coding-v2 hard gate: {name}"),
    }
}

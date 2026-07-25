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

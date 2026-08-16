//! CognitCore trait — like Linux kernel's CFS scheduler.
//!
//! CognitCore is the cognitive computation layer. It doesn't decide
//! "should I?" (that's SelfField). It decides "how do I?" and
//! "what should I do?"

use serde::{Deserialize, Serialize};

use ::contracts::Action;
use ::contracts::Context;

/// A plan — the output of CognitCore's thinking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: uuid::Uuid,
    pub steps: Vec<PlanStep>,
    pub estimated_cost: CostEstimate,
    pub risk_level: dasein::AwarenessRiskLevel,
    pub reasoning: String,
    pub alternatives: Vec<Plan>,
}

/// A single step in a plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: uuid::Uuid,
    pub action: Action,
    pub depends_on: Vec<uuid::Uuid>,
    pub expected_outcome: String,
    pub rollback_action: Option<Action>,
}

/// Cost estimate for a plan.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostEstimate {
    pub estimated_tokens: u32,
    pub estimated_time_ms: u64,
    pub estimated_tool_calls: usize,
}

/// Result of executing a plan.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub plan_id: uuid::Uuid,
    pub success: bool,
    pub steps_completed: usize,
    pub steps_total: usize,
    pub output: String,
    pub error: Option<String>,
    pub elapsed_ms: u64,
}

/// Reflection on an execution — CognitCore's self-assessment.
#[derive(Debug, Clone)]
pub struct Reflection {
    pub what_worked: Vec<String>,
    pub what_failed: Vec<String>,
    pub what_to_improve: Vec<String>,
    pub confidence: f64, // 0.0 to 1.0
}

pub use ::contracts::reflection::{ReflectionEntry, ReflectionOutcome, ReflectionTrigger};

/// Critique of a plan — CognitCore's self-criticism.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Critique {
    pub dimension: CriticismDimension,
    pub severity: CriticismSeverity,
    pub description: String,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CriticismDimension {
    Correctness,
    Completeness,
    Risk,
    Efficiency,
    Consistency,
    Reversibility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CriticismSeverity {
    Info,
    Warning,
    Error,
    Fatal,
}

/// A learned rule — extracted from experience.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnedRule {
    pub id: String,
    pub pattern: String,
    pub action: String,
    pub confidence: f64,
    pub examples: Vec<String>,
}

pub use ::contracts::reflection::{BehaviorAdjustment, EvolutionLogEntry};

/// An observation about the world.
#[derive(Debug, Clone)]
pub struct Observation {
    pub what: String,
    pub source: String,
    pub data: serde_json::Value,
}

/// An experience — a completed action with its outcome.
#[derive(Debug, Clone)]
pub struct Experience {
    pub action: Action,
    pub result: ::contracts::ActionResult,
    pub context: Context,
}

//! CognitCore trait — like Linux kernel's CFS scheduler.
//!
//! CognitCore is the cognitive computation layer. It doesn't decide
//! "should I?" (that's SelfField). It decides "how do I?" and
//! "what should I do?"

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::Action;
use crate::Context;
use crate::Subsystem;

/// A plan — the output of CognitCore's thinking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: uuid::Uuid,
    pub steps: Vec<PlanStep>,
    pub estimated_cost: CostEstimate,
    pub risk_level: crate::RiskLevel,
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

/// What triggered a reflection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReflectionTrigger {
    TaskComplete,
    Impasse,
    Manual,
}

impl std::fmt::Display for ReflectionTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TaskComplete => write!(f, "task_complete"),
            Self::Impasse => write!(f, "impasse"),
            Self::Manual => write!(f, "manual"),
        }
    }
}

/// Outcome of a reflected-upon task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReflectionOutcome {
    Success,
    Partial,
    Failure,
}

impl std::fmt::Display for ReflectionOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success => write!(f, "success"),
            Self::Partial => write!(f, "partial"),
            Self::Failure => write!(f, "failure"),
        }
    }
}

/// A structured reflection entry — the persistent form of self-reflection.
///
/// Stored in Episodic Memory and queryable via /reflect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub trigger: ReflectionTrigger,
    pub task_summary: String,
    pub outcome: ReflectionOutcome,
    pub what_worked: Vec<String>,
    pub what_failed: Vec<String>,
    pub learned: Vec<String>,
    pub behavior_changes: Vec<String>,
    pub confidence: f64,
}

impl ReflectionEntry {
    /// Serialize to JSON bytes for storage in MemoryEntry.content.
    pub fn to_json_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Deserialize from JSON bytes.
    pub fn from_json_bytes(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }

    /// Human-readable summary for display.
    pub fn summary(&self) -> String {
        let icon = match self.outcome {
            ReflectionOutcome::Success => "✅",
            ReflectionOutcome::Partial => "⚠️",
            ReflectionOutcome::Failure => "❌",
        };
        let trigger = match self.trigger {
            ReflectionTrigger::TaskComplete => "",
            ReflectionTrigger::Impasse => " [impasse]",
            ReflectionTrigger::Manual => " [manual]",
        };
        format!(
            "[{}] {}{} {}",
            self.timestamp.format("%Y-%m-%d %H:%M"),
            icon,
            trigger,
            self.task_summary
        )
    }

    /// Detailed display for /reflect output.
    pub fn detail(&self) -> String {
        let mut lines = Vec::new();
        lines.push(self.summary());

        if !self.learned.is_empty() {
            lines.push("  学到:".to_string());
            for l in &self.learned {
                lines.push(format!("    · {}", l));
            }
        }
        if !self.behavior_changes.is_empty() {
            lines.push("  行为调整:".to_string());
            for c in &self.behavior_changes {
                lines.push(format!("    · {}", c));
            }
        }
        lines.join("\n")
    }
}

use chrono::{DateTime, Utc};

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

/// A single behavior adjustment proposed by the evolution engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorAdjustment {
    /// What is being adjusted (e.g., "care.safety.weight", "boundary.rule.rm *").
    pub target: String,
    /// Previous value, if numeric.
    pub old_value: Option<f64>,
    /// New value, if numeric.
    pub new_value: Option<f64>,
    /// Human-readable reason for the adjustment.
    pub reason: String,
}

/// An evolution log entry — records a behavior evolution event.
///
/// Produced by the ExperienceSummarizer when it detects patterns
/// in accumulated reflections and proposes adjustments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionLogEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    /// What triggered this evolution (e.g., "periodic_review", "threshold_reached").
    pub trigger: String,
    /// IDs of the reflection entries that formed the basis.
    pub basis: Vec<String>,
    /// Detected behavioral patterns.
    pub patterns_detected: Vec<String>,
    /// Proposed behavior adjustments.
    pub adjustments: Vec<BehaviorAdjustment>,
}

impl EvolutionLogEntry {
    /// Serialize to JSON bytes for storage.
    pub fn to_json_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Deserialize from JSON bytes.
    pub fn from_json_bytes(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

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
    pub result: crate::ActionResult,
    pub context: Context,
}

/// CognitCore trait — the cognitive scheduler.
///
/// Like CFS decides how to schedule processes, CognitCore decides
/// how to approach problems: what to think about, how to plan,
/// how to critique, and how to learn.
#[async_trait]
pub trait CognitOps: Subsystem {
    /// Think about an intent and produce a plan.
    async fn think(&self, intent: &crate::Intent, ctx: &Context) -> Result<Plan>;

    /// Reflect on an execution result.
    async fn reflect(&self, execution: &ExecutionResult) -> Result<Reflection>;

    /// Critique a plan before execution.
    async fn critique(&self, plan: &Plan) -> Result<Vec<Critique>>;

    /// Learn from experience — extract reusable rules.
    async fn learn(&self, experience: &Experience) -> Result<Vec<LearnedRule>>;

    /// Update world model with new observation.
    async fn update_world(&self, observation: &Observation) -> Result<()>;
}

//! SelfField trait — like Linux kernel's LSM / SELinux.
//!
//! SelfField is the policy engine. It reviews intents, enforces boundaries,
//! resolves conflicts, and maintains identity continuity.
//! It is not a module — it's a field. But it has a trait interface.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::context::Context;
use crate::subsystem::Subsystem;

/// Verdict from SelfField review — like SELinux allow/deny.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Verdict {
    /// Allow the action without modification.
    Allow,
    /// Allow with modification (SelfField rewrote the intent).
    AllowWithModification { modification: serde_json::Value },
    /// Deny the action.
    Deny { reason: String },
    /// Require user confirmation before proceeding.
    RequireConfirmation { reason: String, risk_level: RiskLevel },
    /// Must run in sandbox first.
    SandboxFirst { reason: String },
    /// Delay execution until condition is met.
    Delay { reason: String, until: String },
}

/// Risk level for actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RiskLevel {
    None = 0,
    Low = 1,
    Medium = 2,
    High = 3,
    Critical = 4,
}

/// An intent — something that wants to happen.
///
/// Could be a user request, a BrainCore plan, or a MetaRuntime mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    /// What wants to happen.
    pub action: String,
    /// Parameters.
    pub parameters: serde_json::Value,
    /// Source of the intent (user, brain, meta, etc.).
    pub source: IntentSource,
    /// Human-readable description.
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IntentSource {
    User,
    Brain,
    Body,
    Memory,
    Meta,
    External,
}

/// Identity — current self model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub name: String,
    pub description: String,
    pub version: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_mutation: Option<chrono::DateTime<chrono::Utc>>,
}

/// A care — something the agent values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Care {
    pub topic: String,
    pub weight: f64, // 0.0 to 1.0
    pub description: String,
}

/// A conflict between multiple opinions.
#[derive(Debug, Clone)]
pub struct Conflict {
    pub source_a: ConflictSource,
    pub source_b: ConflictSource,
    pub context: Context,
}

/// Source of a conflicting opinion.
#[derive(Debug, Clone)]
pub enum ConflictSource {
    User { intent: String },
    Brain { proposal: String, confidence: f64 },
    Body { objection: String, risk: RiskLevel },
    Memory { evidence: String },
    Self_ { concern: String },
}

/// Resolution of a conflict.
#[derive(Debug, Clone)]
pub enum Resolution {
    AcceptA { reason: String },
    AcceptB { reason: String },
    Compromise { action: String, reason: String },
    EscalateToUser { question: String },
    SandboxExperiment { plan: String },
}

/// A mutation intent — how the agent wants to change itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationIntent {
    pub target: String,       // What to change (e.g., "boundary_rules", "care_priorities")
    pub change: serde_json::Value, // The proposed change
    pub reason: String,       // Why
    pub reversible: bool,     // Can this be undone?
}

/// SelfField trait — the LSM policy engine.
///
/// SelfField reviews intents, enforces boundaries, resolves conflicts,
/// and maintains identity continuity. It is the "should I?" layer.
#[async_trait]
pub trait SelfFieldOps: Subsystem {
    /// Review an intent. Returns a verdict.
    ///
    /// This is the core operation — like an LSM hook. Every significant
    /// action passes through here before execution.
    async fn review(&self, intent: &Intent, ctx: &Context) -> Result<Verdict>;

    /// Get current identity.
    async fn identity(&self) -> Result<Identity>;

    /// Get current cares.
    async fn cares(&self) -> Result<Vec<Care>>;

    /// Record a narrative — why a decision was made.
    ///
    /// This is how the agent explains itself. Not just logging —
    /// it's the continuity of self-narrative.
    async fn narrate(&self, event: &str, reason: &str) -> Result<()>;

    /// Resolve an internal conflict.
    async fn resolve_conflict(&self, conflict: &Conflict) -> Result<Resolution>;

    /// Review a mutation intent — should the agent change itself?
    async fn review_mutation(&self, mutation: &MutationIntent) -> Result<Verdict>;
}

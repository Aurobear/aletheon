use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::brain::ExecutionResult;
use crate::context::Context;
use crate::self_field::Intent;
use crate::subsystem::{Subsystem, SubsystemHealth};

/// Runtime orchestration trait — like init/systemd
/// Manages agent lifecycle, ReAct loop, scheduling, boot sequencing
#[async_trait]
pub trait RuntimeOps: Subsystem {
    /// Orchestrate a full cognitive cycle: think → execute → reflect
    async fn orchestrate(&self, intent: &Intent, ctx: &Context) -> Result<ExecutionResult>;

    /// List registered agents
    fn agents(&self) -> Vec<AgentInfo>;

    /// Schedule a recurring/one-shot task
    async fn schedule(&self, task: ScheduledTask) -> Result<()>;

    /// Health check all managed subsystems
    async fn health_all(&self) -> Vec<SubsystemHealth>;

    /// Process one ReAct iteration (think + maybe execute)
    async fn step(&self, ctx: &Context) -> Result<StepResult>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub status: AgentStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentStatus {
    Idle,
    Running,
    Failed { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub schedule: ScheduleKind,
    pub action: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScheduleKind {
    Once { at: chrono::DateTime<chrono::Utc> },
    Cron { expression: String },
    Interval { seconds: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub completed: bool,
    pub output: Option<String>,
    pub tool_calls: usize,
    pub continue_reason: Option<String>,
}

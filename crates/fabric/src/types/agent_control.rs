use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::attempt::{AttemptEvidence, AttemptUsage, RuntimeId};
use super::operation::{OperationId, ProcessId};
use super::process::{AgentId, AgentProfileId};
use super::space::AgoraSpaceId;
use super::workspace::{BroadcastEpoch, ContentId};

pub const MAX_AGENT_TASK_BYTES: usize = 64 * 1024;
pub const MAX_AGENT_MESSAGE_BYTES: usize = 64 * 1024;
pub const MAX_AGENT_OUTPUT_BYTES: usize = 1024 * 1024;
pub const MAX_CONTEXT_ITEMS: usize = 64;
pub const MAX_EVIDENCE_ITEMS: usize = 128;
pub const MAX_ARTIFACTS: usize = 128;
pub const MAX_LIST_ITEMS: usize = 1000;
pub const MAX_AGENT_BROADCAST_REFS: usize = 64;

/// Immutable receipt for workspace content explicitly made available to a
/// child Agent. The triple is required so a content ID cannot be replayed from
/// a different space or broadcast epoch.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentBroadcastRef {
    pub space: AgoraSpaceId,
    pub epoch: BroadcastEpoch,
    pub content_id: ContentId,
}

impl AgentBroadcastRef {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        ensure_text(&self.space.0, 1024, "broadcast space ID")?;
        if self.epoch.0 == 0 {
            return Err(AgentControlError::invalid(
                "broadcast epoch must be nonzero",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: AgentProfileId,
    pub system_prompt: String,
    pub model: String,
    pub allowed_tools: Vec<String>,
    pub max_iterations: usize,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_tool_calls: u32,
    pub max_elapsed_ms: u64,
}

impl AgentProfile {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        ensure_text(&self.id.0, 512, "profile ID")?;
        ensure_text(
            &self.system_prompt,
            MAX_AGENT_MESSAGE_BYTES,
            "profile system prompt",
        )?;
        ensure_text(&self.model, 512, "profile model")?;
        ensure_count(self.allowed_tools.len(), 256, "profile tools")?;
        for tool in &self.allowed_tools {
            ensure_text(tool, 512, "profile tool")?;
        }
        if self.max_iterations == 0
            || self.max_input_tokens == 0
            || self.max_output_tokens == 0
            || self.max_tool_calls == 0
            || self.max_elapsed_ms == 0
        {
            return Err(AgentControlError::invalid(
                "Agent profile limits must be nonzero",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AgentContextFork {
    None,
    LastTurns { count: u16 },
    SelectedProjection { items: Vec<String> },
}

impl Default for AgentContextFork {
    fn default() -> Self {
        Self::SelectedProjection { items: Vec::new() }
    }
}

impl AgentContextFork {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        match self {
            Self::None => Ok(()),
            Self::LastTurns { count } if *count == 0 || *count > 100 => Err(
                AgentControlError::invalid("last-turn count must be between 1 and 100"),
            ),
            Self::LastTurns { .. } => Ok(()),
            Self::SelectedProjection { items } => {
                ensure_count(items.len(), MAX_CONTEXT_ITEMS, "context items")?;
                for item in items {
                    ensure_text(item, MAX_AGENT_MESSAGE_BYTES, "context item")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentBudget {
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_tool_calls: u32,
    pub max_elapsed_ms: u64,
    pub max_cost_usd: Option<f64>,
    pub max_depth: u16,
}

impl AgentBudget {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        if self.max_input_tokens == 0
            || self.max_output_tokens == 0
            || self.max_tool_calls == 0
            || self.max_elapsed_ms == 0
            || self.max_depth == 0
        {
            return Err(AgentControlError::invalid(
                "Agent budget values must be nonzero",
            ));
        }
        if self
            .max_cost_usd
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        {
            return Err(AgentControlError::invalid("Agent cost budget is invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSpawnRequest {
    pub root_agent_id: AgentId,
    pub parent_agent_id: Option<AgentId>,
    pub parent_process_id: Option<ProcessId>,
    pub profile_id: AgentProfileId,
    pub runtime_id: RuntimeId,
    pub task: String,
    pub context: AgentContextFork,
    #[serde(default)]
    pub broadcast_refs: Vec<AgentBroadcastRef>,
    pub allowed_tools: Vec<String>,
    pub budget: AgentBudget,
}

impl AgentSpawnRequest {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        ensure_text(&self.profile_id.0, 512, "profile ID")?;
        ensure_text(&self.runtime_id.0, 512, "runtime ID")?;
        ensure_text(&self.task, MAX_AGENT_TASK_BYTES, "Agent task")?;
        ensure_count(self.allowed_tools.len(), 256, "allowed tools")?;
        for tool in &self.allowed_tools {
            ensure_text(tool, 512, "allowed tool")?;
        }
        self.context.validate()?;
        ensure_count(
            self.broadcast_refs.len(),
            MAX_AGENT_BROADCAST_REFS,
            "broadcast references",
        )?;
        for reference in &self.broadcast_refs {
            reference.validate()?;
        }
        let mut unique = self.broadcast_refs.clone();
        unique.sort_by(|left, right| {
            (&left.space.0, left.epoch, left.content_id).cmp(&(
                &right.space.0,
                right.epoch,
                right.content_id,
            ))
        });
        unique.dedup();
        if unique.len() != self.broadcast_refs.len() {
            return Err(AgentControlError::invalid(
                "broadcast references contain duplicates",
            ));
        }
        self.budget.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWaitRequest {
    pub caller_root_agent_id: AgentId,
    pub agent_id: AgentId,
    pub timeout_ms: u64,
}

impl AgentWaitRequest {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        if self.timeout_ms == 0 {
            return Err(AgentControlError::invalid(
                "Agent wait timeout must be nonzero",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSendRequest {
    pub caller_root_agent_id: AgentId,
    pub agent_id: AgentId,
    pub message: String,
    pub start_turn: bool,
}

impl AgentSendRequest {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        ensure_text(&self.message, MAX_AGENT_MESSAGE_BYTES, "Agent message")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentListRequest {
    pub caller_root_agent_id: AgentId,
    pub status: Option<AgentRunStatus>,
    pub limit: usize,
}

impl AgentListRequest {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        if self.limit == 0 || self.limit > MAX_LIST_ITEMS {
            return Err(AgentControlError::invalid("Agent list limit is invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Queued,
    Running,
    Waiting,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

impl AgentRunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHandle {
    pub agent_id: AgentId,
    pub root_agent_id: AgentId,
    pub parent_agent_id: Option<AgentId>,
    pub process_id: ProcessId,
    pub operation_id: OperationId,
    pub runtime_id: RuntimeId,
    pub profile_id: AgentProfileId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentControlMessage {
    pub sequence: u64,
    pub from: AgentId,
    pub to: AgentId,
    pub content: String,
}

impl AgentControlMessage {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        ensure_text(&self.content, MAX_AGENT_MESSAGE_BYTES, "Agent message")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentArtifact {
    pub kind: String,
    pub reference: String,
    pub content_hash: String,
}

impl AgentArtifact {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        ensure_text(&self.kind, 512, "artifact kind")?;
        ensure_text(&self.reference, 4096, "artifact reference")?;
        ensure_text(&self.content_hash, 512, "artifact content hash")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentResult {
    pub output: String,
    pub usage: AttemptUsage,
    pub evidence: Vec<AttemptEvidence>,
    pub artifacts: Vec<AgentArtifact>,
}

impl AgentResult {
    pub fn validate(&self) -> Result<(), AgentControlError> {
        if self.output.len() > MAX_AGENT_OUTPUT_BYTES {
            return Err(AgentControlError::invalid(
                "Agent output exceeds byte limit",
            ));
        }
        ensure_count(self.evidence.len(), MAX_EVIDENCE_ITEMS, "Agent evidence")?;
        ensure_count(self.artifacts.len(), MAX_ARTIFACTS, "Agent artifacts")?;
        for artifact in &self.artifacts {
            artifact.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub handle: AgentHandle,
    pub status: AgentRunStatus,
    pub result: Option<AgentResult>,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentControlErrorKind {
    InvalidRequest,
    NotFound,
    Forbidden,
    Capacity,
    Conflict,
    Timeout,
    Terminal,
    Persistence,
    Runtime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("Agent control {kind:?}: {message}")]
pub struct AgentControlError {
    pub kind: AgentControlErrorKind,
    pub message: String,
}

impl AgentControlError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            kind: AgentControlErrorKind::InvalidRequest,
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait AgentControlPort: Send + Sync {
    async fn spawn(&self, request: AgentSpawnRequest) -> Result<AgentHandle, AgentControlError>;
    async fn wait(&self, request: AgentWaitRequest) -> Result<AgentSnapshot, AgentControlError>;
    async fn send(
        &self,
        request: AgentSendRequest,
    ) -> Result<AgentControlMessage, AgentControlError>;
    async fn cancel(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError>;
    async fn inspect(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError>;
    async fn list(
        &self,
        request: AgentListRequest,
    ) -> Result<Vec<AgentSnapshot>, AgentControlError>;
}

fn ensure_text(value: &str, max: usize, label: &str) -> Result<(), AgentControlError> {
    if value.trim().is_empty() || value.len() > max {
        return Err(AgentControlError::invalid(format!(
            "{label} is empty or exceeds byte limit"
        )));
    }
    Ok(())
}

fn ensure_count(value: usize, max: usize, label: &str) -> Result<(), AgentControlError> {
    if value > max {
        return Err(AgentControlError::invalid(format!(
            "{label} count exceeds limit"
        )));
    }
    Ok(())
}

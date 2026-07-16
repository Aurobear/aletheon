//! Turn service contracts shared by executive adapters and cognitive sessions.

use crate::types::admission::{
    AuditEventId, BudgetRequest, CapabilityScope, LeaseRequest, PrincipalId, RiskLevel,
    SandboxRequirement, UsageReport,
};
use crate::types::llm_types::{LlmProvider, ToolDefinition};
use crate::types::local_authority::{ConnectionId, ThreadId, WorkspacePolicy};
use crate::types::message::Message;
use crate::types::operation::{MonoDeadlineMillis, OperationId, ProcessId};
use crate::types::session::TurnId;
use crate::types::turn::{TurnEvent, TurnRequest};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecallRequest {
    pub session_id: String,
    pub input: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecallSet {
    pub snippets: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaseinView {
    pub text: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgoraView {
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityCall {
    pub operation_id: OperationId,
    pub process_id: ProcessId,
    pub name: String,
    pub input: serde_json::Value,
    pub call_id: String,
    pub deadline: Option<MonoDeadlineMillis>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityAuthority {
    pub agent: Option<crate::AgentToolContext>,
    pub principal: PrincipalId,
    pub action: String,
    pub requested_scope: CapabilityScope,
    pub risk: RiskLevel,
    pub budget: Option<BudgetRequest>,
    pub lease: Option<LeaseRequest>,
    pub sandbox: SandboxRequirement,
    pub connection_id: ConnectionId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub workspace: WorkspacePolicy,
    // Compatibility projection for non-approval consumers.
    pub session_id: String,
    pub working_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct InvocationControl {
    pub cancel: CancellationToken,
}

impl Default for InvocationControl {
    fn default() -> Self {
        Self {
            cancel: CancellationToken::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CapabilityRequest {
    pub call: CapabilityCall,
    pub authority: CapabilityAuthority,
    pub control: InvocationControl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityResult {
    pub call_id: String,
    pub output: String,
    pub is_error: bool,
    pub usage: UsageReport,
    pub audit_id: Option<AuditEventId>,
}

#[async_trait]
pub trait TurnEventSink: Send + Sync {
    async fn emit(&self, event: TurnEvent);
}

#[async_trait]
pub trait TurnServices: Send + Sync {
    async fn recall(&self, req: RecallRequest) -> Result<RecallSet>;
    async fn dasein_view(&self, process: ProcessId) -> Result<DaseinView>;
    async fn agora_view(&self, session_id: &str) -> Result<AgoraView>;
    async fn invoke(&self, call: CapabilityCall) -> CapabilityResult;

    fn llm_provider(&self) -> Option<&dyn LlmProvider> {
        None
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        Vec::new()
    }

    fn seed_messages(&self, _request: &TurnRequest) -> Vec<Message> {
        Vec::new()
    }
}

pub struct NoopTurnEventSink;

#[async_trait]
impl TurnEventSink for NoopTurnEventSink {
    async fn emit(&self, _event: TurnEvent) {}
}

#[derive(Default)]
pub struct StubTurnServices;

#[async_trait]
impl TurnServices for StubTurnServices {
    async fn recall(&self, _req: RecallRequest) -> Result<RecallSet> {
        Ok(RecallSet::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> Result<DaseinView> {
        Ok(DaseinView::default())
    }

    async fn agora_view(&self, _session_id: &str) -> Result<AgoraView> {
        Ok(AgoraView::default())
    }

    async fn invoke(&self, req: CapabilityCall) -> CapabilityResult {
        CapabilityResult {
            call_id: req.call_id,
            output: format!("tool {} is unavailable in stub", req.name),
            is_error: true,
            usage: UsageReport::default(),
            audit_id: None,
        }
    }
}

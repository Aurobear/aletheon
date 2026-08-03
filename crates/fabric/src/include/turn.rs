//! Turn service contracts shared by executive adapters and cognitive sessions.

use crate::types::admission::{
    AuditEventId, BudgetRequest, CapabilityScope, LeaseRequest, PrincipalId, RiskLevel,
    SandboxRequirement, UsageReport,
};
use crate::types::conscious_arbitration::CapabilityBatchPlan;
use crate::types::llm_types::{LlmProvider, ToolDefinition};
use crate::types::local_authority::{ConnectionId, ThreadId, WorkspacePolicy};
use crate::types::message::Message;
use crate::types::operation::{MonoDeadlineMillis, OperationId, ProcessId};
use crate::types::session::TurnId;
use crate::types::time::MonoTime;
use crate::types::turn::{TurnEvent, TurnRequest};
use anyhow::Result;
use async_trait::async_trait;
use schemars::JsonSchema;
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

/// Host-authored, typed requirements for a cognitive turn. Implementations
/// derive these from explicit client metadata or workflow policy, never by
/// matching prompt phrases in the execution loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum TurnRequirement {
    InvokeAgentRuntime {
        runtime_id: String,
    },
    InvokeCapability {
        name: String,
    },
    ObserveTerminal {
        operation_id: OperationId,
    },
    RunRoleGraph {
        workspace_scope: Vec<String>,
        allowed_capabilities: Vec<crate::AgentRuntimeCapability>,
        expected_evidence: Vec<String>,
    },
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
    #[serde(default)]
    pub permission_mode: crate::permission::HostPermissionMode,
}

#[derive(Debug, Clone)]
pub struct InvocationControl {
    pub cancel: CancellationToken,
    pub turn_event_sender: Option<crate::ipc::TurnEventSender>,
}

impl Default for InvocationControl {
    fn default() -> Self {
        Self {
            cancel: CancellationToken::new(),
            turn_event_sender: None,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_delta: Option<crate::PatchDelta>,
}

/// Authoritative terminal evidence projected by the host after a capability
/// invocation reaches a terminal state. Submission/progress events never
/// construct this receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CapabilityTerminalReceipt {
    pub invocation_id: String,
    pub operation_id: OperationId,
    #[schemars(with = "uuid::Uuid")]
    pub process_id: ProcessId,
    pub capability: String,
    pub status: CapabilityTerminalStatus,
    #[schemars(with = "u64")]
    pub started_at: MonoTime,
    #[schemars(with = "u64")]
    pub finished_at: MonoTime,
    pub exit_code: Option<i32>,
    pub error_class: Option<CapabilityErrorClass>,
    pub artifact_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub output_ref: Option<String>,
    pub truncated: bool,
    pub retry_disposition: CapabilityRetryDisposition,
    #[schemars(with = "Option<uuid::Uuid>")]
    pub audit_id: Option<AuditEventId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityTerminalStatus {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityErrorClass {
    Implementation,
    Environment,
    Dependency,
    Permission,
    Provider,
    Timeout,
    ConcurrentModification,
    InvalidRequest,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityRetryDisposition {
    Never,
    AfterBackoff,
    AfterCorrection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityReceiptDetails {
    pub status: Option<CapabilityTerminalStatus>,
    pub exit_code: Option<i32>,
    pub error_class: Option<CapabilityErrorClass>,
    pub artifact_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub output_ref: Option<String>,
    pub truncated: bool,
    pub retry_disposition: CapabilityRetryDisposition,
}

impl Default for CapabilityReceiptDetails {
    fn default() -> Self {
        Self {
            status: None,
            exit_code: None,
            error_class: None,
            artifact_ids: Vec::new(),
            evidence_ids: Vec::new(),
            output_ref: None,
            truncated: false,
            retry_disposition: CapabilityRetryDisposition::Never,
        }
    }
}

impl CapabilityTerminalReceipt {
    pub fn from_terminal_result(
        call: &CapabilityCall,
        result: &CapabilityResult,
        started_at: MonoTime,
        finished_at: MonoTime,
        details: CapabilityReceiptDetails,
    ) -> Self {
        let status = details.status.unwrap_or(if result.is_error {
            CapabilityTerminalStatus::Failed
        } else {
            CapabilityTerminalStatus::Succeeded
        });
        Self {
            invocation_id: call.call_id.clone(),
            operation_id: call.operation_id,
            process_id: call.process_id,
            capability: call.name.clone(),
            status,
            started_at,
            finished_at,
            exit_code: details.exit_code.or(result.usage.exit_code),
            error_class: details.error_class,
            artifact_ids: details.artifact_ids,
            evidence_ids: details.evidence_ids,
            output_ref: details.output_ref,
            truncated: details.truncated,
            retry_disposition: details.retry_disposition,
            audit_id: result.audit_id,
        }
    }

    pub fn proves_success(&self) -> bool {
        self.status == CapabilityTerminalStatus::Succeeded
    }
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

    /// Persist or project an authoritative terminal receipt. The default is a
    /// compatibility no-op; callers invoke it only after terminal observation.
    async fn record_capability_receipt(&self, _receipt: CapabilityTerminalReceipt) {}

    /// Persist the exact bounded fragments selected for one provider request.
    async fn record_model_context_projection(
        &self,
        _receipt: crate::model_projection::ModelContextProjectionReceipt,
    ) {
    }

    /// Persist the terminal outcome paired with a model context projection.
    async fn record_inference_receipt(
        &self,
        _receipt: crate::types::inference_receipt::InferenceTerminalReceipt,
    ) {
    }

    fn turn_requirements(&self, request: &TurnRequest) -> Vec<TurnRequirement> {
        request.requirements.clone()
    }

    async fn plan_capability_batch(
        &self,
        calls: Vec<CapabilityCall>,
    ) -> anyhow::Result<CapabilityBatchPlan> {
        Ok(CapabilityBatchPlan::identity(&calls))
    }

    /// Drain mid-turn user interjections at a Cognit-declared safe point.
    /// Implementations return independent messages in FIFO order. The default
    /// preserves legacy behavior for services without G3 queue wiring.
    async fn drain_interjections(&self) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }

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
            patch_delta: None,
        }
    }
}

#[cfg(test)]
mod terminal_receipt_tests {
    use super::*;

    #[test]
    fn receipt_binds_terminal_result_to_exact_invocation() {
        let call = CapabilityCall {
            operation_id: OperationId::new(),
            process_id: ProcessId::new(),
            name: "validation_run".into(),
            input: serde_json::json!({"validation_kind": "test"}),
            call_id: "call-7".into(),
            deadline: None,
        };
        let result = CapabilityResult {
            call_id: call.call_id.clone(),
            output: "passed".into(),
            is_error: false,
            usage: UsageReport {
                exit_code: Some(0),
                ..UsageReport::default()
            },
            audit_id: None,
            patch_delta: None,
        };

        let receipt = CapabilityTerminalReceipt::from_terminal_result(
            &call,
            &result,
            MonoTime(10),
            MonoTime(20),
            CapabilityReceiptDetails {
                evidence_ids: vec!["evidence-7".into()],
                ..CapabilityReceiptDetails::default()
            },
        );

        assert!(receipt.proves_success());
        assert_eq!(receipt.operation_id, call.operation_id);
        assert_eq!(receipt.process_id, call.process_id);
        assert_eq!(receipt.invocation_id, "call-7");
        assert_eq!(receipt.exit_code, Some(0));
        assert_eq!(receipt.evidence_ids, vec!["evidence-7"]);
    }

    #[test]
    fn explicit_timeout_cannot_be_misreported_as_success() {
        let call = CapabilityCall {
            operation_id: OperationId::new(),
            process_id: ProcessId::new(),
            name: "exec_command".into(),
            input: serde_json::Value::Null,
            call_id: "timed-out".into(),
            deadline: None,
        };
        let result = CapabilityResult {
            call_id: call.call_id.clone(),
            output: "timeout".into(),
            is_error: true,
            usage: UsageReport::default(),
            audit_id: None,
            patch_delta: None,
        };
        let receipt = CapabilityTerminalReceipt::from_terminal_result(
            &call,
            &result,
            MonoTime(1),
            MonoTime(2),
            CapabilityReceiptDetails {
                status: Some(CapabilityTerminalStatus::TimedOut),
                error_class: Some(CapabilityErrorClass::Timeout),
                retry_disposition: CapabilityRetryDisposition::AfterCorrection,
                ..CapabilityReceiptDetails::default()
            },
        );
        assert!(!receipt.proves_success());
        assert_eq!(receipt.status, CapabilityTerminalStatus::TimedOut);
    }
}

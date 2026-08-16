//! Port abstractions that decouple the channel layer from concrete
//! executive-side stores.
//!
//! [`ApprovalApplicationPort`] lets `router.rs` and the channel capability
//! adapters depend only on application vocabulary instead of
//! the concrete executive approval repository. Delivery receipts are kept on
//! a separate projection port because they belong to the channel boundary,
//! not to the approval use case.

use crate::channel::{ConversationId, MessageContent, OutboundMessage};
use ::contracts::contract::command::SubmitPromptIntent;
use ::contracts::permission::HostPermissionMode;
use ::contracts::{
    ApprovalCategory, ApprovalId, ApprovalSnapshot, AttemptId, GoalId, GoalSnapshot, PrincipalId,
    WorkspacePolicy,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Bounded proactive Goal notification. Raw provider output and errors are
/// deliberately absent from this channel-facing projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalProgressKind {
    Succeeded,
    RetryBackoff,
    Escalated,
    AwaitingHuman,
    Failed,
    Cancelled,
}

impl GoalProgressKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::RetryBackoff => "retry_backoff",
            Self::Escalated => "escalated",
            Self::AwaitingHuman => "awaiting_human",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalProgress {
    pub goal_id: GoalId,
    pub attempt_id: AttemptId,
    pub kind: GoalProgressKind,
}

impl GoalProgress {
    fn text(&self) -> String {
        let status = match self.kind {
            GoalProgressKind::Succeeded => "completed successfully",
            GoalProgressKind::RetryBackoff => "will retry after bounded backoff",
            GoalProgressKind::Escalated => "escalated to reviewer",
            GoalProgressKind::AwaitingHuman => "is awaiting human input",
            GoalProgressKind::Failed => "failed",
            GoalProgressKind::Cancelled => "was cancelled",
        };
        format!(
            "Goal {} attempt {} {status}.",
            self.goal_id.0, self.attempt_id.0
        )
    }

    fn correlation_id(&self) -> String {
        format!(
            "goal:{}:attempt:{}:{}",
            self.goal_id.0,
            self.attempt_id.0,
            self.kind.as_str()
        )
    }

    pub(crate) fn outbound(&self, conversation_id: ConversationId) -> OutboundMessage {
        OutboundMessage {
            conversation_id,
            content: MessageContent::Text { text: self.text() },
            actions: vec![],
            reply_to: None,
            correlation_id: self.correlation_id(),
        }
    }
}

/// Typed Application command port for channel-originated turns. The channel
/// layer supplies a Fabric command DTO; it does not construct or own Runtime
/// sessions, persistence, or provider state.
#[async_trait]
pub trait ChannelTurnApplicationPort: Send + Sync {
    async fn execute(&self, request: &ChannelTurnRequest) -> anyhow::Result<String>;
}

#[async_trait]
pub trait ChannelChatCommandPort: Send + Sync {
    async fn execute(&self, request: &ChannelChatRequest) -> anyhow::Result<String>;
}

/// Channel-originated prompt command. The Application adapter owns conversion
/// to the daemon's full `ClientIntent`; Gateway only supplies authenticated
/// identity, correlation, and the typed prompt payload.
#[derive(Debug, Clone)]
pub struct ChannelTurnRequest {
    pub principal: PrincipalId,
    pub correlation_id: String,
    pub prompt: SubmitPromptIntent,
}

#[derive(Debug, Clone)]
pub struct ChannelChatRequest {
    pub principal: PrincipalId,
    pub correlation_id: String,
    pub text: String,
}

/// Neutral channel-to-turn adapter. Optional read preflight is injected as a
/// typed port; no provider or repository type crosses this boundary.
pub struct ChannelChatAdapter {
    turn: Arc<dyn ChannelTurnApplicationPort>,
    read: Option<Arc<dyn ChannelReadApplicationPort>>,
    workspace: WorkspacePolicy,
}

impl ChannelChatAdapter {
    pub fn new(turn: Arc<dyn ChannelTurnApplicationPort>, workspace: WorkspacePolicy) -> Self {
        Self {
            turn,
            read: None,
            workspace,
        }
    }

    pub fn with_read_port(mut self, read: Arc<dyn ChannelReadApplicationPort>) -> Self {
        self.read = Some(read);
        self
    }
}

/// Result of an optional Application-owned external-read preflight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelReadDecision {
    Reply(String),
    Rewrite(String),
    Passthrough,
}

#[async_trait]
pub trait ChannelReadApplicationPort: Send + Sync {
    async fn preprocess(&self, principal: &str, text: &str) -> anyhow::Result<ChannelReadDecision>;
}

#[async_trait]
impl ChannelChatCommandPort for ChannelChatAdapter {
    async fn execute(&self, request: &ChannelChatRequest) -> anyhow::Result<String> {
        let query = if let Some(read) = &self.read {
            match read
                .preprocess(request.principal.0.as_str(), &request.text)
                .await?
            {
                ChannelReadDecision::Reply(text) => return Ok(text),
                ChannelReadDecision::Rewrite(text) => text,
                ChannelReadDecision::Passthrough => request.text.clone(),
            }
        } else {
            request.text.clone()
        };

        self.turn
            .execute(&ChannelTurnRequest {
                principal: request.principal.clone(),
                correlation_id: request.correlation_id.clone(),
                prompt: SubmitPromptIntent {
                    content: query,
                    session_id: None,
                    workspace: self.workspace.clone(),
                    requirements: Vec::new(),
                    task_kind: None,
                    permission_mode: HostPermissionMode::Safe,
                    execution_target: ::contracts::ExecutionTargetSelection::default(),
                },
            })
            .await
    }
}

#[async_trait]
pub trait ExternalAccountDirectory: Send + Sync {
    async fn active_account_labels(&self, principal: &str) -> anyhow::Result<Vec<String>>;
}

/// Typed Application command/query port for owner-scoped Goal operations.
/// Gateway routes syntax to this port but does not own Goal state transitions.
#[async_trait]
pub trait ChannelGoalApplicationPort: Send + Sync {
    async fn create_draft(&self, owner: &str, intent: &str) -> anyhow::Result<GoalSnapshot>;
    async fn list(&self, owner: &str) -> anyhow::Result<Vec<GoalSnapshot>>;
    async fn show(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot>;
    async fn pause(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot>;
    async fn resume(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot>;
    async fn cancel(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot>;
}

/// Typed channel route for Goal syntax. Parsing/formatting and Goal state
/// transitions live in the Application adapter; Gateway only supplies the
/// authenticated owner and bounded command text.
#[async_trait]
pub trait ChannelGoalCommandPort: Send + Sync {
    async fn execute(
        &self,
        owner: &str,
        command: &str,
        args: &str,
        now_ms: i64,
    ) -> anyhow::Result<String>;
}

/// Application-side post-commit approval side effect and draft-revision seam.
#[async_trait]
pub trait ApprovalResolver: Send + Sync {
    async fn execute_resolved(
        &self,
        approval: &::contracts::ApprovalSnapshot,
        action: &str,
        now_ms: i64,
    ) -> anyhow::Result<()>;

    async fn revise_draft(
        &self,
        _owner: &str,
        _goal_id: ::contracts::GoalId,
        _intent: &str,
        _now_ms: i64,
    ) -> anyhow::Result<::contracts::ApprovalSnapshot> {
        anyhow::bail!("revision is not supported by this approval resolver")
    }
}

#[derive(Default)]
pub struct ApprovalResolverRegistry {
    by_category: Mutex<HashMap<ApprovalCategory, Arc<dyn ApprovalResolver>>>,
    default: Mutex<Option<Arc<dyn ApprovalResolver>>>,
}

impl ApprovalResolverRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, category: ApprovalCategory, resolver: Arc<dyn ApprovalResolver>) {
        self.by_category.lock().unwrap().insert(category, resolver);
    }

    pub fn set_default(&self, resolver: Arc<dyn ApprovalResolver>) {
        *self.default.lock().unwrap() = Some(resolver);
    }

    pub fn resolve(&self, category: ApprovalCategory) -> Option<Arc<dyn ApprovalResolver>> {
        self.by_category
            .lock()
            .unwrap()
            .get(&category)
            .cloned()
            .or_else(|| self.default.lock().unwrap().clone())
    }

    pub fn resolve_category_only(
        &self,
        category: ApprovalCategory,
    ) -> Option<Arc<dyn ApprovalResolver>> {
        self.by_category.lock().unwrap().get(&category).cloned()
    }
}

/// Approval decision accepted by the channel-facing application port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelApprovalDecision {
    Approve,
    Reject { reason: Option<String> },
}

/// Durable approval operations needed by the channel dispatcher and the
/// approval-callback handler. Implemented in executive by adapting the
/// concrete `ApprovalRepository`.
pub trait ApprovalApplicationPort: Send + Sync {
    fn get(&self, id: ApprovalId) -> anyhow::Result<Option<ApprovalSnapshot>>;

    #[allow(clippy::too_many_arguments)]
    fn resolve(
        &self,
        id: ApprovalId,
        expected_version: u64,
        principal: PrincipalId,
        channel: String,
        decision: ChannelApprovalDecision,
        now_ms: i64,
    ) -> anyhow::Result<ApprovalSnapshot>;

    fn list_pending(
        &self,
        principal: &PrincipalId,
        now_ms: i64,
    ) -> anyhow::Result<Vec<ApprovalSnapshot>>;
}

/// Typed callback command for an approval action received from a channel.
/// Resolution and post-commit side effects belong to the Application adapter.
#[async_trait]
pub trait ChannelApprovalCallbackPort: Send + Sync {
    async fn execute(
        &self,
        principal: &str,
        channel: &str,
        action_data: &str,
        now_ms: i64,
    ) -> anyhow::Result<String>;
}

/// Gateway-owned delivery projection for approval notifications.
///
/// This is intentionally separate from [`ApprovalApplicationPort`]: delivery
/// is an outbox/projection concern and must not make the application approval
/// use case repository-shaped.
pub trait ChannelApprovalDeliveryPort: Send + Sync {
    fn record_delivery_pending(
        &self,
        approval_id: ApprovalId,
        channel: &str,
        conversation_id: &str,
        correlation_id: &str,
        now_ms: i64,
    ) -> anyhow::Result<()>;

    fn record_delivery_sent(
        &self,
        correlation_id: &str,
        provider_message_id: &str,
        now_ms: i64,
    ) -> anyhow::Result<()>;

    fn record_delivery_failed(
        &self,
        correlation_id: &str,
        error: &str,
        now_ms: i64,
    ) -> anyhow::Result<()>;
}

/// Gateway-owned persistence port for channel projections.
///
/// The SQLite implementation lives in `adapters-sqlite`; dispatch and route
/// code depends on this narrow surface rather than a connection or SQL schema.
pub trait ChannelProjectionStore: Send {
    fn insert_inbound(
        &mut self,
        message: &crate::channel::InboundMessage,
    ) -> anyhow::Result<ChannelInsertOutcome>;
    fn resolve_principal(&self, channel: &str, external: &str) -> anyhow::Result<Option<String>>;
    fn complete_inbound(
        &mut self,
        channel: &str,
        message_id: &str,
        next_cursor: &str,
        outbound: &crate::channel::OutboundMessage,
    ) -> anyhow::Result<()>;
    fn reject_inbound(
        &mut self,
        channel: &str,
        message_id: &str,
        next_cursor: &str,
    ) -> anyhow::Result<()>;
    fn fail_inbound(&self, channel: &str, message_id: &str, error: &str) -> anyhow::Result<()>;
    fn enqueue_outbound(
        &self,
        channel: &str,
        outbound: &crate::channel::OutboundMessage,
    ) -> anyhow::Result<bool>;
    fn mark_outbound_sent(&self, correlation_id: &str, provider_id: &str) -> anyhow::Result<()>;
    fn mark_outbound_failed(&self, correlation_id: &str, error: &str) -> anyhow::Result<()>;
    fn pending_inbound(
        &self,
        channel: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<crate::channel::InboundMessage>>;
    fn pending_outbox(
        &self,
        channel: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<crate::channel::OutboundMessage>>;
}

/// Result of inserting an inbound provider message into the durable channel
/// projection. The adapter crate maps its physical SQLite result into this
/// Gateway-owned vocabulary; Gateway does not depend on SQLite types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelInsertOutcome {
    Inserted,
    Duplicate,
}

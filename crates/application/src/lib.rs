//! Aletheon narrow Application contracts and wired use cases.
//!
//! This crate owns pure use-case types, typed errors, and use cases that are
//! wired into production. Host orchestration for Turn, Goal, and Agent Control
//! lives under `aletheon::wiring::application`; this crate does **not** own a
//! concrete repository or mint Runtime IDs. It depends only on `contracts` and
//! `runtime`, with no concrete host adapter.

pub mod approval;
pub mod cache;
pub mod capability_benchmark;
pub mod checkpoint_projection;
pub mod command_dispatcher;
pub mod daemon_lifecycle;
pub mod data_governance;
pub mod error;
pub mod evaluation_projection;
pub mod extension;
pub mod external_identity;
pub mod goal_attempt;
pub mod goal_draft;
pub mod goal_frame;
pub mod goal_projection;
pub mod goal_retry;
pub mod governed_review;
pub mod health;
pub mod message_history;
pub mod objective;
pub mod session_input;
pub mod settlement;
pub mod thread_authority;
pub mod turn_control;
pub mod use_case;
pub mod workflow;
pub mod workspace_checkpoint;
pub mod workspace_identity;
pub mod workspace_trust;

pub use approval::{
    resolve_decision, ApprovalError, ApprovalRecord, ApprovalScope, ApprovalStore,
    ApprovingPrincipal, DecisionRequestId, OpaqueApprovalGrant,
};
pub use cache::{decide_cache, CacheDecision, CacheKey, CacheLayer, PromptConstructionProfile};
pub use capability_benchmark::CapabilityRollupProjectionSink;
pub use checkpoint_projection::{CheckpointSettlement, TURN_CHECKPOINT_PROJECTION_SCHEMA_V1};
pub use command_dispatcher::{
    prompt_completion_from_rpc_result, CommandDispatcher, CommandOutput, CommandUseCases,
};
pub use daemon_lifecycle::{DaemonInstallMode, InstallModeFacts};
pub use data_governance::*;
pub use error::ApplicationError;
pub use evaluation_projection::{
    EvaluationProjection, EvaluationProjectionContext, EvaluationProjectionMetrics,
    EvaluationProjectionRecord, EvaluationProjectionReport, EvaluationProjectionSink,
};
pub use extension::{
    ExtensionFlags, ExtensionId, ExtensionPort, ExtensionRegistration, InMemoryExtensionRegistry,
};
pub use external_identity::{
    CapabilityGrant, ExternalCapabilityId, ExternalIdentity, ExternalIdentityContractError,
    ExternalIdentityId, ExternalIdentityState, ExternalProviderId, GrantState,
    LOCAL_OWNER_PRINCIPAL,
};
pub use goal_attempt::GoalAttempt;
pub use goal_draft::{create_goal_draft, ingest_external_stimulus, ExternalStimulus, GoalDraft};
pub use goal_frame::{GoalAttemptSummary, GoalFrame, GoalRemainingBudget};
pub use goal_projection::GoalProjectionEvidence;
pub use goal_retry::{RetryDecision, RetryPolicy};
pub use governed_review::{
    GovernedReviewJob, GovernedReviewReceipt, ProposedReviewChange, ReviewBudget,
    ReviewContractError, ReviewEvidence, ReviewSchemaVersion, ReviewStatus, REVIEW_SCHEMA_CURRENT,
    REVIEW_SCHEMA_PREVIOUS,
};
pub use health::{ComponentHealth, HealthClass, HealthRegistry};
pub use message_history::{build_request_messages, select_text_history};
pub use objective::{Objective, ObjectiveStatus, ObjectiveSummary};
pub use session_input::{
    InMemoryPromptQueueStore, InterjectionBuffer, PromptQueueMetricSnapshot, PromptQueueStore,
    SessionInputCoordinator,
};
pub use settlement::{
    ChangeTransactionAuthority, HostSettlementService, InMemoryTransactionSettlementStore,
    TransactionReviewService, TransactionSettlementStore,
};
pub use thread_authority::{ThreadAuthorityKey, ThreadSettings};
pub use turn_control::{CollaborationMode, InterruptReason};
pub use use_case::{
    CreateSession, DeleteSession, ForkSession, GetSession, ListSessions, ResumeSession,
};
pub use workflow::{
    ConditionExpr, Edge, GraphState, JoinStrategy, JoinStrategyDef, LogEntry, Node, NodeKind,
    NodeStatus, OnExhausted, WorkflowDef,
};
pub use workspace_checkpoint::{
    CheckpointFileEntry, CheckpointFinalizeState, CheckpointId, FsDomainRef, RestoreOutcome,
    TurnCheckpoint, MAX_CHECKPOINT_FILES,
};
pub use workspace_identity::WorkspaceIdentity;
pub use workspace_trust::{
    ClientMode, DiscoveredConfigDigest, ExecutableConfigSource, TrustEvaluationInput, TrustReceipt,
    WorkspaceTrustDecision,
};

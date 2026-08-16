//! # Aletheon Base
//!
//! Shared contracts, types, communication primitives, and infrastructure for the Aletheon runtime.
//!
//! Like Linux kernel header files define the contract between subsystems
//! (`file_operations`, `net_proto_ops`), this crate defines the contracts
//! between Aletheon subsystems.
//!
//! ## Module Layout (Linux kernel style)
//!
//! - `include/` — Subsystem trait contracts (like kernel `include/`)
//! - `types/` — Shared data types
//! - `events/` — Event system (types + infrastructure)
//! - `ipc/` — Inter-process communication (like kernel `net/`)
//! - `kernel/` — Core infrastructure (observability, registry, debug, errors)
//! - `policy/` — Execution policy engine
//! - `dasein/` — Phenomenological module

#![allow(deprecated)]

// === Module declarations ===

mod adapters;
pub mod compaction;
pub mod contract;
pub mod dasein;
pub mod include;
pub mod ipc;
pub mod primitives;
pub mod protocol;
pub mod reflection;
pub mod turn_policy;
pub mod types;
pub mod workspace_identity;

pub use protocol::client::{
    ActivityKind, ActivitySnapshot, ActivityState, CheckpointListEntry, CheckpointListSnapshot,
    CheckpointMutationCoverage, CheckpointReviewSettlement, CheckpointReviewSnapshot,
    CheckpointRollbackAction, ClientEvent as ProtocolClientEvent, ClientMessage, ClientRequest,
    EventCursor, EventSubscription, ReviewFinding, ReviewFindingLocation, ReviewFindingSeverity,
    ReviewFindingStatus, SessionEventPage, SessionListSnapshot, SessionReadSnapshot,
    SnapshotRequest, TaskPhase, TaskRuntimeFacts, TaskSettlement, TaskSnapshot, TaskStepSnapshot,
    TransactionReviewAction, TransactionReviewParams, TransactionReviewSnapshot,
    TransactionSettlementDecision, TransactionSettlementGetParams, TransactionSettlementReceipt,
    UiSnapshot, CHECKPOINT_LIST_SCHEMA_VERSION, CLIENT_PROTOCOL_VERSION,
    SESSION_READ_MODEL_SCHEMA_VERSION,
};

// === Module re-exports ===

// Subsystem trait modules (from include/)
pub use include::body;
pub use include::memory;
pub use include::subsystem;

// Shared type modules (from types/)
pub use types::agent_control;
pub use types::attempt;
pub use types::capability;
pub use types::change_transaction;
pub use types::cognitive_workflow;
pub use types::config_provenance::{ConfigProvenance, ConfigSource, ConfigSourceKind, Provenanced};
pub use types::conscious_arbitration;
pub use types::conscious_core;
pub use types::context;
pub use types::context_budget::{
    BudgetMissingReason, ContextBudgetProjection, ContextBudgetSource, ContextBudgetSourceKind,
    ContextCompactionMode, ContextCompactionProjection, ContextCostTokens, HistoryBudgetTokens,
    HistoryTokens, ModelContextWindowTokens, ProfileInputLimitTokens, RolloutBudgetProjection,
    RolloutBudgetTokens, RolloutBudgetValue,
};
pub use types::data_governance;
pub use types::evidence;
pub use types::goal;
pub use types::llm_types;
pub use types::message;
pub use types::model_projection;
pub use types::paths;
pub use types::permission;
pub use types::sandbox;
pub use types::session;
pub use types::tool;
pub use types::turn_control;
pub use types::workspace;

pub use types::conscious_arbitration::{
    CapabilityBatchDecision, CapabilityBatchPlan, ConsciousArbitrationMode, ConsciousFieldReadout,
    FieldDecisionKind, FieldDecisionReason, LatestConsciousContextPort,
};
pub use types::conscious_core::{
    BroadcastIntegrationReceipt, ConsciousContextProjection, ConsciousProcessor,
    ContextProjectionReceipt, ProcessorAck, ProcessorContext, ProcessorHealth, ProcessorId,
    ProcessorResponse, StructuredSelfView, MAX_PROCESSOR_ACKNOWLEDGEMENTS,
    MAX_PROCESSOR_RESPONSE_CANDIDATES, MAX_SELF_VIEW_ITEMS,
};
pub use types::context::Context;
pub use types::execution_target::{
    ExecutionTarget, ExecutionTargetSelection, ExecutionTargetSource,
};
pub use types::workspace::{
    ActionProposalFrame, BroadcastAck, BroadcastAckStatus, BroadcastDelivery, BroadcastEpoch,
    CandidateScore, CareConcernFrame, ContentId, GoalFrame, GovernedActionOutcomeFrame,
    PredictionErrorFrame, PredictionFrame, RecalledExperienceFrame, SalienceVector,
    SelectionExplanation, SelectionResult, VisibilityScope, WorkspaceAttribution,
    WorkspaceBroadcast, WorkspaceCandidate, WorkspaceContent, WorkspaceObservation,
    WorkspaceProvenance, WorkspaceReflection, MAX_BROADCAST_RESPONSES, MAX_BROADCAST_WINNERS,
    WORKSPACE_SCHEMA_V1,
};

// Event modules

// IPC modules (from ipc/)
// Note: `contracts::ipc` is already the directory module (pub mod ipc above).
// Old `contracts::ipc::IpcMessage` etc. are now at `contracts::ipc::ipc_msg::IpcMessage`.
// Re-export ipc_msg types at ipc level for backward compatibility.

// Kernel modules (from kernel/)

// Policy modules (from policy/)

// === Re-exports for backward compatibility ===
// These preserve the flat API surface so external consumers don't need to change.

// Subsystem traits (from include/)
pub use include::admission::BudgetController;
pub use include::body::{Action, ActionResult, BodyRuntime};
pub use include::chronos::{Clock, Elapsed, Timer};
pub use include::memory::EmbeddingProvider;
pub use include::subsystem::{Subsystem, SubsystemContext, SubsystemHealth, Version};
pub use include::turn::{
    AgoraView, CapabilityAuthority, CapabilityCall, CapabilityErrorClass, CapabilityReceiptDetails,
    CapabilityRequest, CapabilityResult, CapabilityRetryDisposition, CapabilityTerminalReceipt,
    CapabilityTerminalStatus, DaseinView, InvocationControl, NoopTurnEventSink, RecallRequest,
    RecallSet, StubTurnServices, TurnEventSink, TurnRequirement, TurnServices,
};

// Shared types (from types/)
pub use types::admission::{
    AdmissionError, AdmissionRequest, AuditEventId, BudgetRequest, BudgetReservationId,
    BudgetReservationReceipt, BudgetScope, BudgetScopeId, BudgetScopeKind, BudgetTransferReceipt,
    CapabilityId, CapabilityScope, ExecutionPermit, LeaseRequest, PermitId, PrincipalId,
    ResourceLeaseId, RevokeReason, SandboxDecision, SandboxRequirement, UsageReport,
    BUDGET_SCOPE_SCHEMA_VERSION, LOCAL_OWNER_PRINCIPAL,
};
pub use types::agent_control::{
    AgentApprovalPolicy, AgentArtifact, AgentAttenuationReport, AgentBroadcastRef, AgentBudget,
    AgentBudgetField, AgentContextFork, AgentControlError, AgentControlErrorKind,
    AgentControlMessage, AgentControlPort, AgentDelegationAuthority, AgentHandle, AgentListRequest,
    AgentMessageDeliveryState, AgentMessageKind, AgentMessagePayload, AgentProfile,
    AgentRecoveryDecision, AgentRecoveryReceipt, AgentResult, AgentRunStatus,
    AgentRuntimeCapability, AgentSendRequest, AgentSnapshot, AgentSpawnIntent, AgentSpawnRequest,
    AgentTaskId, AgentWaitRequest, AgentWorkspaceMode, ParentRestriction, RiskTier,
    RuntimeResumability, AGENT_MESSAGE_SCHEMA_V1,
};
pub use types::agent_settlement::{
    can_reparent, settlement_idempotency_key, AgentResourceClass, BackgroundResourceDecl,
    ReparentContext, ReparentReceipt, SettlementPhase, SettlementReceipt, SettlementTerminal,
    MAX_BACKGROUND_RESOURCES,
};
pub use types::approval::{
    ApprovalArtifactRef, ApprovalCategory, ApprovalContractError, ApprovalId, ApprovalResolution,
    ApprovalRisk, ApprovalSnapshot, ApprovalStatus, ApprovalSubject,
};
pub use types::attempt::{
    AttemptEvidence, AttemptId, AttemptStatus, AttemptUsage, CognitiveRole, FailureClass,
    RuntimeFailure, RuntimeId, RuntimeObservability, RuntimeResult,
};
pub use types::capability::{Capability, CapabilityLevel, CapabilitySet};
pub use types::coding_job::{
    ChangedFile, ChangedFileKind, CodingAttemptRequest, CodingJobId, CodingJobReport,
    CodingJobSpec, CodingJobStatus, CodingNetworkPolicy, VerificationCheck, VerificationReport,
    VerificationSeverity, WorkspaceBoundary,
};
pub use types::evaluation::{
    EvaluationContractError, EvaluationContractId, EvaluationDecision, EvaluationEvidenceSnapshot,
    EvaluationExecutionContext, EvaluationMode, EvaluationReceipt, EvaluationReceiptId,
    EvaluationReceiptRef, EvaluationSettings, EvaluationSnapshotId, EvaluationSubject,
    EvaluationThresholds, EvidenceRef, RequiredEvidence, RequiredGate, TaskEvaluationContract,
    TaskKind, EVALUATION_SCHEMA_V1,
};
pub use types::goal::{
    GoalBudget, GoalBudgetUsage, GoalId, GoalSnapshot, GoalSpec, GoalState, GoalWaitReason,
};
pub use types::llm_types::{
    canonicalize_tool_definitions, tool_schema_digest, CacheTelemetry, InferenceCapabilities,
    InferenceUsage, LlmProvider, LlmResponse, LlmStream, ModelInfo, ModelRuntimeFacts, StopReason,
    StreamChunk, ToolDefinition, ToolDefinitionCanonicalizationError,
};
pub use types::local_authority::{
    ApprovalPolicy, ConnectionId, LocalOsPrincipal, PermissionProfileId, PrincipalContext,
    ProtectedPathPolicy, ThreadId, WorkspacePolicy, WorkspaceResolveError, WorkspaceSelection,
};
pub use types::message::{ContentBlock, ImageSource, Message, Priority as MessagePriority, Role};
pub use types::operation::{
    CancelReason, MonoDeadlineMillis, OperationExitReason, OperationId, OperationKind,
    OperationRecord, OperationRequest, OperationResult, OperationState, ProcessId,
};
pub use types::permission::{
    PermissionBehavior, PermissionContext, PermissionMode, PermissionRule,
};
pub use types::process::{
    AgentId, AgentProfileId, ExitReason, ExitStatus, MailboxId, NamespaceId, OsProcessId,
    ProcessIdentity, ProcessOwnership, ProcessRecord, ProcessSignal, ProcessSnapshot, ProcessState,
    RuntimeProcessId, SpaceId, SpawnSpec,
};
pub use types::sandbox::{
    resolve_profile, IsolationLevel, ProfileName, ProfileResolveError, ResolvedSandboxPolicy,
    SandboxBackend, SandboxCapabilities, SandboxCommand, SandboxConfig, SandboxExecutor,
    SandboxPreference, SandboxProfileConfig, SandboxProfiles, SandboxResult, DENY_GLOB_MAX_DEPTH,
    DENY_GLOB_MAX_ENTRIES, DENY_GLOB_MAX_MATCHES,
};
pub use types::session::{
    AppendOutcome, ItemId, ItemPayload, ItemRecord, SessionAppendStore, SessionFork,
    SessionForkedEvent, SessionNotification, SessionPrincipalBoundEvent, SessionProtocolV5,
    SessionReadStore, SessionRecord, SessionStatus, TaskProjectionFact, TurnId, TurnRecord,
    TurnRecoveryClassification, SESSION_SCHEMA_VERSION, TASK_PROJECTION_FACT_SCHEMA_VERSION,
};
pub use types::space::{
    AccessMode, AgoraSpaceId, AgoraVersion, ArtifactId, ContextBinding, ContextSpace, SessionId,
    SpaceSnapshotId, VersionedOverlay,
};
pub use types::time::{wall_to_datetime, MonoDeadline, MonoTime, WallTime};
pub use types::tool::{
    AgentToolContext, ApprovalOwner, PatchDelta, PatchDeltaApplied, PatchDeltaFailed,
    PatchDeltaFileChange, PendingApprovalKey, PermissionLevel as ToolPermissionLevel,
    ThreadGrantKey, Tool, ToolApprovalAuthority, ToolCacheDependencies, ToolContext, ToolResult,
    ToolResultMeta,
};
pub use types::tool_stream::{
    tool_event_channel, tool_event_channel_for_call, BoundToolEventReceiver, ToolEventSink,
    ToolExecutionError, ToolExecutionEvent, ToolNotification, ToolNotificationKind, ToolProgress,
    TOOL_PROGRESS_CHANNEL_CAP,
};
pub use types::turn::{
    TurnEvent, TurnFailure, TurnFailureKind, TurnMetrics, TurnRequest, TurnResult, TurnStop,
    TurnTerminalStatus,
};

// IPC types (from ipc/)
pub use ipc::envelope_v2::{
    DeliveryPattern as EnvelopeV2Delivery, EnvelopeV2, MessageId, SchemaId,
    Target as EnvelopeV2Target,
};

// Kernel foundations (from kernel/)

// Primitives (RFC-017 canonical vocabulary)
// Primitive evidence vocabulary remains available at the crate root.
pub use primitives::{Evidence, Hypothesis};

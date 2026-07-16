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

pub mod contract;
pub mod dasein;
pub mod events;
pub mod include;
pub mod ipc;
pub mod kernel;
pub mod policy;
pub mod primitives;
pub mod protocol;
pub mod security;
pub mod types;

pub use protocol::client::{
    ClientEvent as ProtocolClientEvent, ClientMessage, ClientRequest, EventCursor,
    EventSubscription, SnapshotRequest, UiSnapshot, CLIENT_PROTOCOL_VERSION,
};

// === Backward-compatible module re-exports ===
// These allow `fabric::genome::*`, `fabric::self_field::*`, etc. to continue working.
// Downstream crates can also use the new paths: `fabric::include::genome::*`, `fabric::types::tool::*`, etc.

// Subsystem trait modules (from include/)
pub use include::agora;
pub use include::body;
pub use include::cognit;
pub use include::memory;
pub use include::meta;
pub use include::plugin;
pub use include::runtime;
pub use include::self_field;
pub use include::space;
pub use include::subsystem;

// Shared type modules (from types/)
pub use types::agent;
pub use types::agent_control;
pub use types::attempt;
pub use types::capability;
pub use types::channel;
pub use types::conscious_core;
pub use types::context;
pub use types::evidence;
pub use types::extension;
pub use types::external_event;
pub use types::external_identity;
pub use types::genome;
pub use types::goal;
pub use types::google;
pub use types::grounding;
pub use types::hook;
pub use types::hook_ext;
pub use types::llm_types;
pub use types::message;
pub use types::objective;
pub use types::paths;
pub use types::permission;
pub use types::resource;
pub use types::sandbox;
pub use types::session;
pub use types::tool;
pub use types::vision;
pub use types::workspace;

pub use protocol::conscious_core::{
    CandidateDisposition, ConsciousCoreSnapshot, InspectorProcessorAck,
};
pub use types::conscious_core::{
    BroadcastIntegrationReceipt, ConsciousContextProjection, ConsciousProcessor,
    ContextProjectionReceipt, ProcessorAck, ProcessorContext, ProcessorHealth, ProcessorId,
    ProcessorResponse, StructuredSelfView, MAX_PROCESSOR_ACKNOWLEDGEMENTS,
    MAX_PROCESSOR_RESPONSE_CANDIDATES, MAX_SELF_VIEW_ITEMS,
};
pub use types::conscious_core_trace::{
    AcceptanceEvidence, ConsciousCoreTrace, ConsciousTraceEvent, IndicatorResult,
    CONSCIOUS_CORE_TRACE_SCHEMA_V1,
};
pub use types::workspace::{
    ActionProposalFrame, BroadcastAck, BroadcastAckStatus, BroadcastDelivery, BroadcastEpoch,
    CandidateScore, CareConcernFrame, ContentId, GoalFrame, GovernedActionOutcomeFrame,
    PredictionErrorFrame, PredictionFrame, RecalledExperienceFrame, SalienceVector,
    SelectionExplanation, SelectionResult, ToolOutcomeFrame, VisibilityScope, WorkspaceAttribution,
    WorkspaceBroadcast, WorkspaceCandidate, WorkspaceContent, WorkspaceObservation,
    WorkspaceProvenance, WorkspaceReflection, MAX_BROADCAST_RESPONSES, MAX_BROADCAST_WINNERS,
    WORKSPACE_SCHEMA_V1,
};

// Event modules
pub use events::evolution;
pub use events::ui_event;

// IPC modules (from ipc/)
// Note: `fabric::ipc` is already the directory module (pub mod ipc above).
// Old `fabric::ipc::IpcMessage` etc. are now at `fabric::ipc::ipc_msg::IpcMessage`.
// Re-export ipc_msg types at ipc level for backward compatibility.
pub use ipc::envelope;
pub use ipc::ipc_types;
pub use ipc::protocol as legacy_protocol;
pub use ipc::transport;

// Kernel modules (from kernel/)
pub use kernel::debug;
pub use kernel::observable;
pub use kernel::registry;

// Policy modules (from policy/)
pub use policy::execpolicy;
pub use policy::permission_authority;
pub use policy::verifier;

// === Re-exports for backward compatibility ===
// These preserve the flat API surface so external consumers don't need to change.

// Subsystem traits (from include/)
pub use include::admission::AdmissionController;
pub use include::agora::{
    AgoraCommit, AgoraOperation, AgoraOps, AgoraProposal, AgoraService, AgoraViewRequest,
    CommitReceipt, RejectReason, VersionConflict, WorkspaceCommitPermit,
};
pub use include::body::{Action, ActionResult, BodyRuntime};
pub use include::capability_invoker::CapabilityInvoker;
pub use include::chronos::{Clock, Elapsed, Timer};
pub use include::cognit::{
    BehaviorAdjustment, CognitOps, CostEstimate, Critique, EvolutionLogEntry, ExecutionResult,
    Experience, LearnedRule, Observation, Plan, PlanStep, Reflection, ReflectionEntry,
    ReflectionOutcome, ReflectionTrigger,
};
pub use include::memory::{
    CompactResult, CompactStrategy, EmbeddingProvider, MemoryBackend, MemoryEntry, MemoryFilter,
    MemoryHandle, MemoryQuery, MemoryStats, MemoryType,
};
pub use include::meta::{
    Evaluation, MetaRuntimeOps, MigrationResult, RuntimeCandidate, TestResult,
};
pub use include::plugin::{Plugin, PluginContext};
pub use include::process::{OperationHandle, OperationManager, ProcessHandle, ProcessManager};
pub use include::runtime::{
    AgentInfo, AgentStatus, RuntimeOps, ScheduleKind, ScheduledTask, StepResult,
};
pub use include::self_field::{
    AwarenessCore, AwarenessExtension, AwarenessExtensionCounts, AwarenessGrowthSuggestion,
    AwarenessRiskLevel, Care, Conflict, Identity, Intent, IntentSource, MutationIntent, Resolution,
    SelfAwareness, SelfFieldOps, SelfState, Verdict, VerdictAction, VerdictHandler,
};
pub use include::space::SpaceManager;
pub use include::subsystem::{InitPhase, Subsystem, SubsystemContext, SubsystemHealth, Version};
pub use include::turn::{
    AgoraView, CapabilityAuthority, CapabilityCall, CapabilityRequest, CapabilityResult,
    DaseinView, InvocationControl, NoopTurnEventSink, RecallRequest, RecallSet, StubTurnServices,
    TurnEventSink, TurnServices,
};

// Shared types (from types/)
pub use types::admission::{
    AdmissionError, AdmissionRequest, AuditEventId, BudgetRequest, BudgetReservationId,
    BudgetReservationReceipt, BudgetScope, BudgetScopeId, BudgetScopeKind, CapabilityId,
    CapabilityScope, ExecutionPermit, LeaseRequest, PermitId, PrincipalId, ResourceLeaseId,
    RevokeReason, SandboxDecision, SandboxRequirement, UsageReport, BUDGET_SCOPE_SCHEMA_VERSION,
};
pub use types::agent::Pid;
pub use types::agent_control::{
    AgentArtifact, AgentBroadcastRef, AgentBudget, AgentContextFork, AgentControlError,
    AgentControlErrorKind, AgentControlMessage, AgentControlPort, AgentHandle, AgentListRequest,
    AgentMessageDeliveryState, AgentMessageKind, AgentMessagePayload, AgentMessageReceipt,
    AgentProfile, AgentRecoveryDecision, AgentRecoveryReceipt, AgentResult, AgentRunStatus,
    AgentSendRequest, AgentSnapshot, AgentSpawnRequest, AgentTaskId, AgentWaitRequest,
    RuntimeResumability, AGENT_MESSAGE_SCHEMA_V1,
};
pub use types::approval::{
    ApprovalArtifactRef, ApprovalCategory, ApprovalContractError, ApprovalId, ApprovalResolution,
    ApprovalRisk, ApprovalSnapshot, ApprovalStatus, ApprovalSubject,
};
pub use types::attempt::{
    AttemptEvidence, AttemptId, AttemptStatus, AttemptUsage, CognitiveRole, FailureClass,
    RuntimeFailure, RuntimeId, RuntimeResult,
};
pub use types::capability::{Capability, CapabilityLevel, CapabilitySet};
pub use types::channel::{
    ActionType, ChannelHealth, ChannelId, ConversationId, ExternalSenderId, InboundMessage,
    MessageContent, OutboundMessage, UserAction,
};
pub use types::coding_job::{
    ChangedFile, ChangedFileKind, CodingJobId, CodingJobReport, CodingJobSpec, CodingJobStatus,
    CodingNetworkPolicy, VerificationCheck, VerificationReport, VerificationSeverity,
    WorkspaceBoundary,
};
pub use types::context::{Context, TraceState};
pub use types::extension::{
    ActivationConstraints, ExtensionCatalog, ExtensionContractError, ExtensionDescriptor,
    ExtensionId, ExtensionKind, ExtensionOrigin, ExtensionSnapshot,
};
pub use types::external_event::{
    DriveFileMetadata, ExternalContentRef, ExternalEventDraft, ExternalEventEnvelope,
    ExternalEventError, ExternalEventId, ExternalObjectRef, GoogleEvent, MailChange,
    EXTERNAL_EVENT_SCHEMA_VERSION,
};
pub use types::external_identity::{
    CapabilityGrant, ExternalIdentity, ExternalIdentityId, ExternalIdentityState, ExternalScope,
    GrantState, IdentityProvider, LOCAL_OWNER_PRINCIPAL,
};
pub use types::genome::Genome;
pub use types::goal::{
    GoalBudget, GoalBudgetUsage, GoalId, GoalSnapshot, GoalSpec, GoalState, GoalWaitReason,
};
pub use types::google::{
    CalendarEvent, CalendarEventPage, CalendarTimeRange, GmailMessage, GmailMessagePage,
    GmailMessageSummary, GmailQuery, GoogleContractError, ProviderRecordRef,
    MAX_GOOGLE_PROVIDER_ID_BYTES,
};
pub use types::hook::{HookContext, HookPoint, HookResult, HookToolResult};
pub use types::hook_ext::{CommandHookResult, HookConfig, HookType};
pub use types::llm_types::{
    LlmProvider, LlmResponse, LlmStream, ModelInfo, StopReason, StreamChunk, ToolDefinition, Usage,
};
pub use types::message::{ContentBlock, ImageSource, Message, Priority as MessagePriority, Role};
pub use types::objective::{Objective, ObjectiveStatus, ObjectiveSummary};
pub use types::operation::{
    CancelReason, MonoDeadlineMillis, OperationExitReason, OperationId, OperationKind,
    OperationRecord, OperationRequest, OperationResult, OperationState, ProcessId,
};
pub use types::permission::{
    ModeConfig, PermissionBehavior, PermissionContext, PermissionMode, PermissionRule,
};
pub use types::process::{
    AgentId, AgentProfileId, ExitReason, ExitStatus, MailboxId, NamespaceId, OsProcessId,
    ProcessIdentity, ProcessRecord, ProcessSignal, ProcessSnapshot, ProcessState, SpaceId,
    SpawnSpec,
};
pub use types::resource::{ManagedResource, ResourceState};
pub use types::sandbox::{
    IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxCommand, SandboxConfig,
    SandboxExecutor, SandboxPreference, SandboxResult,
};
pub use types::session::{
    AppendOutcome, ItemId, ItemPayload, ItemRecord, SessionAppendStore, SessionFork,
    SessionForkedEvent, SessionNotification, SessionProtocolV1, SessionRecord, SessionStatus,
    TurnId, TurnRecord, SESSION_SCHEMA_VERSION,
};
pub use types::space::{
    AccessMode, AgoraSpaceId, AgoraVersion, ArtifactId, ContextBinding, ContextSpace, MemoryViewId,
    ProjectionVersion, SessionId, SpaceSnapshotId, VersionedOverlay, WorldProjectionId,
};
pub use types::time::{wall_to_datetime, MonoDeadline, MonoTime, WallTime};
pub use types::tool::{
    AgentToolContext, PermissionLevel as ToolPermissionLevel, Tool, ToolContext, ToolResult,
    ToolResultMeta,
};
pub use types::turn::{TurnEvent, TurnMetrics, TurnRequest, TurnResult, TurnStop};

// Event types
pub use events::spine::{
    EventId, EventIdentity, EventPayload, EventPosition, EventSpine, EventTreeId, EventVisibility,
    ParentEventId, SpineEvent, TreeSequence, UnsequencedEvent,
};
pub use events::subscription::{AsyncEnvelopeHandler, EnvelopeHandler};
pub use events::types as event;
pub use events::types::{EventType, Priority, SubscriptionId};
pub use events::ui_event::{
    AwarenessLevel, ClientEvent, CollaborationMode, EvolutionStage, InterruptReason, PlanUpdate,
    SubAgentHandle, SubAgentState, SubAgentStatus,
};

// IPC types (from ipc/)
pub use ipc::bus::communication_bus::{BusConfig, CommunicationBus};
pub use ipc::bus::kernel_bus::KernelEventBus;
pub use ipc::envelope::{Endpoint, Envelope, EnvelopeId, ModuleId, Pattern, Payload, Target};
pub use ipc::envelope_v2::{
    DeliveryPattern as EnvelopeV2Delivery, EnvelopeV2, MessageId, SchemaId,
    Target as EnvelopeV2Target,
};
pub use ipc::ipc_msg::{ForkDirective, ForkResult, GroupId, IpcMessage, MessageKind, Signal};
pub use ipc::ipc_types::{
    AgentId as IpcAgentId, AgentMessage, IpcBackend, IpcPreference, IpcPriority, IpcProbeError,
    MessageType,
};
pub use ipc::protocol::Protocol;
pub use ipc::transport::{
    HealthStatus, Transport as EnvelopeTransport, TransportHealth, TransportKind,
};

// Kernel foundations (from kernel/)
pub use kernel::debug::{DebugEvent, DebugLevel, DebugSink, Tracepoint};
pub use kernel::debug_bus::{DebugBusHook, EventFilter, PerfCounter};
pub use kernel::error::{
    handle_tool_error, llm_backoff, llm_degradation_chain, tool_backoff, tool_degradation_chain,
    AgentError, BackoffStrategy, DegradationChain, DegradationStrategy, ErrorCategory,
    ErrorSeverity, RegistryErrorKind, ToolErrorAction,
};
pub use kernel::observable::{Observable, SubsystemStatus};
pub use kernel::registry::{RegistrationId, Registry};

// Policy (from policy/)
pub use policy::execpolicy::{
    default_heuristics, load_policy_from_str, load_policy_layered, Decision, NetworkProtocol,
    NetworkRule, PatternToken, Policy, PrefixRule,
};
// Note: policy::execpolicy::Evaluation is not re-exported at crate root
// to avoid conflict with include::meta::Evaluation.
// Access via fabric::policy::execpolicy::Evaluation or fabric::execpolicy::Evaluation.

// Primitives (RFC-017 canonical vocabulary)
// Note: primitives::Event is not re-exported at crate root to avoid conflict
// with events::types::Event (the trait). Access via fabric::primitives::Event.
pub use primitives::{
    Command, Commitment, Evidence, Hypothesis, Mailbox, Narrative, Query, Stream,
};

// Compaction (shared context-compaction interface + pruning helpers)
pub use include::compaction::{prune_tool_outputs, truncate_utf8_bytes, CompactorTrait};

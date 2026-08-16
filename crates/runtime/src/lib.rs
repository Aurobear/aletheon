//! External runtime capability manifests, deterministic selection, and the
//! Runtime owner contract (RA-01).

pub mod agent_admission;
pub mod backpressure;
pub use backpressure::BackpressureConfig;
pub mod agent_admission_policy;
pub mod agent_candidates;
pub mod agent_cleanup;
pub mod agent_context;
pub mod agent_events;
pub mod agent_identity;
pub mod agent_lifecycle;
pub mod agent_lifecycle_hooks;
pub mod agent_live_runs;
pub mod agent_mailbox;
pub mod agent_recovery;
pub mod agent_repository;
pub mod agent_resources;
pub mod agent_runtime_projection;
pub mod agent_settlement;
pub mod agent_settlement_adapters;
pub mod agent_stream_adapter;
pub mod agent_supervisor;
pub mod agent_topology;
pub mod agent_wait;
pub mod agent_writer;
pub mod cache_shape;
pub mod command;
pub mod compaction;
pub mod compaction_normalize;
pub mod context_fragment;
pub mod context_working_set;
pub mod durable_write;
pub mod error;
pub mod event;
pub mod event_projection;
pub mod event_spine;
pub mod extension_provider;
pub mod generation_fence;
pub mod ids;
pub mod iteration_budget;
pub mod journal;
pub mod lifecycle;
pub mod lifecycle_contributors;
pub mod mailbox;
pub mod manifest;
pub mod orchestration;
pub mod per_turn_scope;
pub mod ports;
pub mod post_turn;
pub mod pre_turn;
pub mod prefix_cache_observability;
pub mod process_registration;
pub mod profile;
pub mod prompt_partition;
pub mod prompt_queue;
pub mod public_session_projection;
pub mod query;
pub mod read_model;
pub mod selector;
pub mod session_authority;
pub mod session_head;
pub mod session_projection;
pub mod session_service;
pub mod session_shadow;
pub mod session_writer;
pub mod settlement_engine;
pub mod settlement_evidence;
pub mod settlement_ports;
pub mod settlement_receipts;
pub mod tool_stream_bridge;
pub mod turn_cancellation;
pub mod turn_diff_tracker;
pub mod turn_outcome;
pub mod turn_pipeline_lifecycle;
pub mod turn_policy;
pub mod turn_recovery;
pub mod turn_reducer;
pub mod turn_registry;
pub mod turn_tool_projection;
pub mod turn_writer;

pub use agent_admission::{
    constrain_cognitive_workspace, AgentAdmissionLease, AgentAdmissionMetrics, AgentAdmissionPort,
    AgentAdmissionRequest, AgentStorageRequest,
};
pub use agent_admission_policy::{AgentAdmissionPolicy, BoundedAgentAdmission};
pub use agent_candidates::{
    AgentCandidateSubmissionPort, CandidateAdmissionStatus, CandidateCause, CandidateSubmission,
    CandidateSubmissionReceipt,
};
pub use agent_cleanup::{
    AgentCleanupCoordinator, AgentCleanupReport, AgentWorktreeReclaimer, MAX_CLEANUP_BATCH,
};
pub use agent_context::{
    parse_content_id, AgentContextItem, AgentContextItemKind, AgentContextProjection,
    AgentContextProjectionBuilder, MAX_CONTEXT_BROADCAST_REFS, MAX_CONTEXT_CONSTRAINTS,
    MAX_CONTEXT_GOAL_BYTES, MAX_CONTEXT_ITEMS, MAX_CONTEXT_ITEM_BYTES, MAX_CONTEXT_TOTAL_BYTES,
};
pub use agent_events::{AgentRuntimeEvent, AgentRuntimeEventSink};
pub use agent_identity::{runtime_capability, ValidatedAgentIdentity};
pub use agent_lifecycle::{
    reduce_agent_lifecycle, reduce_agent_status_transition, AgentLifecycleEffect,
    AgentLifecycleEvent, AgentLifecycleTransition, InvalidAgentLifecycleTransition,
};
pub use agent_lifecycle_hooks::{
    agent_lifecycle_hook_context, AgentLifecycleHookSink, AgentLifecycleObservation,
    AgentLifecyclePoint, NoopAgentLifecycleHookSink,
};
pub use agent_live_runs::{LiveAgentRun, LiveAgentRuns, ReparentAuthority};
pub use agent_mailbox::{AgentMailboxBridge, AgentRuntimeInbox};
pub use agent_recovery::{
    AgentRecoveryCoordinator, AgentRecoveryHost, AgentRecoveryObservation, AgentRecoveryReport,
    AgentRecoveryRuntimeInput, FailClosedRuntimeProcessSupervisor, RuntimeProcessReclaimOutcome,
    RuntimeProcessSupervisor, MAX_STARTUP_RECOVERY_ROWS,
};
pub use agent_repository::{
    agent_spawn_request_hash, agent_workspace_id, AgentMessageRecord, AgentResourceLease,
    AgentResourceLeaseKind, AgentRunProjection, AgentRunRecord, AgentTerminalReceipt,
};
pub use agent_resources::BackgroundResourceRegistration;
pub use agent_runtime_projection::RuntimeAgentRunProjection;
pub use agent_settlement::{recovery_disposition, RecoveryResourceDisposition};
pub use agent_settlement_adapters::{
    settle_admission, terminal_with_memory_flush, FailClosedSettlementResourcePort,
    ManagedSettlementResourcePort, RepositorySettlementLeasePort,
};
pub use agent_stream_adapter::RuntimeAgentStreamAdapter;
pub use agent_supervisor::{
    AgentProfileResolver, AgentRuntimeSelectionPolicy, DelegateBackend, DelegateBackendId,
    DelegateBackendRegistry, DelegateCommand, DelegateExecutionContext, DelegateMessage,
    DelegateMessageReceipt, DelegateReceipt, DelegateRecoveryRequest, DelegateResult,
    DelegateSpawnRequest, DelegateTaskBackend, RuntimePreferenceHistory,
};
pub use agent_topology::AgentTopologyRoutes;
pub use agent_wait::{AgentWaitTimer, SystemAgentWaitTimer};
pub use agent_writer::{
    mint_agent_run_uuid, AgentProjectionPresence, ObservedAgentBackend, RuntimeAgentSupervisor,
};
pub use command::{
    CancelTurnCommand, CommandReceipt, CreateSessionCommand, ForkSessionCommand,
    ResumeSessionCommand, RuntimeCommand, SpawnAgentRunCommand, StartTurnCommand,
};
pub use context_working_set::{
    ContextCompactionReceipt, ContextCompactor, ContextCompactorFactory, ContextWorkingSet,
};
pub use error::RuntimeError;
pub use event::{
    AgentMailboxDelivery, AgentStreamEvent, RuntimeEvent, TurnStreamEvent, TurnTerminal,
    AGENT_STREAM_SCHEMA_VERSION, TURN_STREAM_SCHEMA_VERSION,
};
pub use event_spine::{
    EventId, EventIdentity, EventPayload, EventPosition, EventSpine, EventTreeId, EventVisibility,
    ParentEventId, SpineEvent, TreeSequence, UnsequencedEvent,
};
pub use extension_provider::AgentRuntimeProvider;
pub use generation_fence::GenerationFence;
pub use ids::{AgentRunId, Generation, SessionId, TurnId};
pub use journal::{RuntimeJournalShadow, ShadowEntry, ShadowMismatch, StreamKind};
pub use manifest::{
    InteractionMode, RuntimeCapability, RuntimeManifest, RuntimeResourceRequirements, TaskEncoding,
    ToolGovernance, WorkspaceMode, MAX_RUNTIME_STORAGE_BYTES, MAX_RUNTIME_STORAGE_ITEMS,
};
pub use orchestration::{
    EvidenceDrivenController, EvidenceGap, OrchestrationStep, Stage, TaskRisk, TransitionReason,
};
pub use per_turn_scope::{PerTurnScope, ScopeExit, ScopeGuard, ScopedResource};
pub use ports::{
    AgentEventSink, AgentProfilePort, RuntimeCommandPort, RuntimeEventPort, RuntimeQueryPort,
    TurnEventSink, TurnLifecycleWriter,
};
pub use process_registration::{NoopRuntimeProcessRegistration, RuntimeProcessRegistrationPort};
pub use profile::AgentProfileDocument;
pub use query::{AgentRunQuery, RuntimeQuery, SessionSnapshotQuery};
pub use selector::{
    RuntimeCandidateRejection, RuntimeSelectionDecision, RuntimeSelectionError,
    RuntimeSelectionRequest, RuntimeSelector,
};
pub use session_authority::{
    SessionAuthority, SessionMaintenanceReceipt, SessionWritePermit, TurnProjection,
};
pub use session_head::{PendingAppend, SessionHead, SessionHeadIndex};
pub use session_service::SessionService;
pub use session_shadow::{SessionShadowVerifier, ShadowReport};
pub use session_writer::RuntimeSessionWriter;
pub use settlement_engine::{
    SettlementEngine, SettlementMetricSnapshot, SettlementMetrics, SettlementRequest,
};
pub use settlement_evidence::{
    NoopSettlementEvidenceSink, SettlementEvidence, SettlementEvidenceSink,
    SpineSettlementEvidenceSink,
};
pub use settlement_ports::SettlementQuiescePort;
pub use settlement_ports::{SettlementLeasePort, SettlementResourcePort};
pub use settlement_receipts::{InMemorySettlementReceiptStore, SettlementReceiptStore};
pub use turn_cancellation::TurnCancellation;
pub use turn_outcome::{
    BlockReason, CancelReason, StopReason, TurnExecutionResult, TurnFailure, TurnOutcome, TurnUsage,
};
pub use turn_reducer::{TransitionOutcome, TurnReducerSeam, TurnTransition};
pub use turn_registry::{ActiveTurn, ActiveTurnKey, ActiveTurnRegistry};
pub use turn_writer::RuntimeTurnWriter;

pub use prompt_queue::{
    evaluate_cancel, evaluate_edit, truncate_prompt_content, PromptEnvelope, PromptId, PromptKind,
    PromptState, QueueOpResult, QueueSnapshot, MAX_INTERJECTION_BYTES, MAX_PROMPT_BYTES,
    MAX_QUEUE_LEN,
};

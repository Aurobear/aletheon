//! # Aletheon Agora
//!
//! The shared cognitive workspace (RFC-014). Mutable working state remains
//! session-isolated and in-memory; accepted workspace commits and broadcast
//! epochs are durably logged for integrity and replay. Long-term content
//! retention still belongs to Mnemosyne.

pub mod attention;
pub mod blackboard;
pub mod broadcast;
pub mod competition;
pub mod conscious_context_slot;
pub mod conscious_core_ports;
pub mod conscious_core_trace;
pub mod conscious_field;
pub mod contract;
pub mod ops;
pub mod persistence;
pub mod scratchpad;
pub mod task_graph;
pub mod trace;
pub mod workspace;

pub use ::contracts::cognitive_workflow::{
    AgentResultReceipt, AgoraProjectionReceipt, AgoraProjectionRequest, AgoraTaskList,
    AgoraTaskProjection, ArtifactLifecycle, CognitiveArtifact, CognitiveArtifactEnvelope,
    CognitiveArtifactId, CognitiveArtifactKind, CognitiveRole, CognitiveStage, CognitiveTaskNode,
    CognitiveTaskNodeId, CognitiveTaskStatus, StageDecision, StageDecisionKind,
};
pub use attention::Attention;
pub use blackboard::Blackboard;
pub use broadcast::{
    BroadcastCoordinator, BroadcastHub, BroadcastHubConfig, BroadcastProcessor, BroadcastReplay,
    ProcessorRegistration, SqliteBroadcastStore,
};
pub use competition::{
    AdmissionMetrics, AdmissionOutcome, CandidatePool, CandidatePoolConfig, SelectionMetrics,
    SelectionPolicy,
};
pub use contract::{
    AgoraCommit, AgoraOperation, AgoraOps, AgoraProposal, AgoraService, AgoraView,
    AgoraViewRequest, CommitReceipt, RejectReason, VersionConflict, WorkspaceCommitPermit,
};
pub use ops::AgoraRegistry;
pub use persistence::{AgoraPersistence, InMemoryCommitLog, SqliteAgoraPersistence};
pub use scratchpad::{RetentionPolicy, Scratchpad, ScratchpadEntry};
pub use task_graph::{TaskGraph, TaskNode, TaskStatus};
pub use trace::{Trace, TraceEntry};
pub use workspace::Workspace;

pub use conscious_context_slot::ConsciousContextSlot;
pub use conscious_core_trace::{
    AcceptanceEvidence, ConsciousCoreTrace, ConsciousTraceEvent, IndicatorResult,
    CONSCIOUS_CORE_TRACE_SCHEMA_V1,
};

pub mod conscious_core_inspector;

pub mod cognitive_role_workflow;
pub mod cognitive_workspace;

pub mod host_acceptance;

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
pub mod ops;
pub mod persistence;
pub mod scratchpad;
pub mod task_graph;
pub mod trace;
pub mod workspace;

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
pub use ops::AgoraRegistry;
pub use persistence::{AgoraPersistence, InMemoryCommitLog};
pub use scratchpad::{RetentionPolicy, Scratchpad, ScratchpadEntry};
pub use task_graph::{TaskGraph, TaskNode, TaskStatus};
pub use trace::{Trace, TraceEntry};
pub use workspace::{AgoraCommit, AgoraOperation, AgoraProposal, VersionConflict, Workspace};

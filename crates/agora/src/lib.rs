//! # Aletheon Agora
//!
//! The shared cognitive workspace (RFC-014). Session-isolated, in-memory.
//! Holds working memory: blackboard, attention, task graph, scratchpad, and
//! reasoning trace. Never persistent by itself — persists via snapshot →
//! Mnemosyne, orchestrated by the executive layer.

pub mod attention;
pub mod blackboard;
pub mod ops;
pub mod scratchpad;
pub mod task_graph;
pub mod trace;
pub mod workspace;

pub use attention::Attention;
pub use blackboard::Blackboard;
pub use ops::AgoraRegistry;
pub use scratchpad::{RetentionPolicy, Scratchpad, ScratchpadEntry};
pub use task_graph::{TaskGraph, TaskNode, TaskStatus};
pub use trace::{Trace, TraceEntry};
pub use workspace::Workspace;

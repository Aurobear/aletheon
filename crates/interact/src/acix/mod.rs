//! Agent-Computer Interface (ACIX) — high-level perception, action, experience, and task management.
//!
//! Agent-Computer Interface (ACIX) — high-level perception, action, experience, and task management.

pub mod aci;
pub mod experience;
pub mod grounding;
pub mod task;

pub use aci::Aci;
pub use experience::{
    ActionRecord, Embedder, Experience, ExperienceLevel, ExperienceMemory, MockEmbedder,
};
pub use grounding::{GroundingProvider, GroundingResult, MockGroundingProvider};
pub use task::{
    TaskAction, TaskDecomposer, TaskGraph, TaskManager, TaskNode, TaskStatus, TaskWorker,
};

//! # Aletheon Memory
//!
//! SQLite-backed implementations of the `MemoryBackend` trait for all 4
//! memory types: Episodic, Semantic, Procedural, and Self.
//!
//! Each backend has its own SQLite file (no lock contention).
//! `MemoryRouter` dispatches by `MemoryType`.

pub mod backends;
pub mod ops;

// Backward-compatible re-exports (flat API)
pub use backends::{EpisodicMemory, ProceduralMemory, SelfMemory, SemanticMemory};
pub use ops::{ConsolidationConfig, ConsolidationResult, MemoryContext, MemoryRouter, ReflectionSummary, SkillSummary};
pub use ops::{compute_activation, ActivationEntry};
pub use ops::{apply_access_boost, compute_strength, should_forget};

// Sub-module re-exports for direct path access (e.g. `memory::episodic::EpisodicMemory`)
pub use backends::episodic;
pub use backends::procedural;
pub use backends::self_memory;
pub use backends::semantic;

pub use ops::router;
pub use ops::consolidation;
pub use ops::decay;
pub use ops::activation;
pub use ops::schema;

#[cfg(test)]
pub mod testing;

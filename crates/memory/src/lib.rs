//! # Aletheon Memory
//!
//! SQLite-backed implementations of the `MemoryBackend` trait.
//! EpisodicMemory is always available (used by the daemon for reflections).
//! Cognitive backends (MemoryRouter + semantic/procedural/self) are behind the
//! off-by-default `cognitive-memory` feature (M-H Option A).

pub mod backends;
pub mod ops;

// Always-available exports
pub use backends::EpisodicMemory;
pub use ops::{apply_access_boost, compute_strength, should_forget};
pub use ops::{compute_activation, ActivationEntry};

// Cognitive exports (off by default)
#[cfg(feature = "cognitive-memory")]
pub use backends::{ProceduralMemory, SelfMemory, SemanticMemory};
#[cfg(feature = "cognitive-memory")]
pub use ops::{
    ConsolidationConfig, ConsolidationResult, MemoryContext, MemoryRouter, ReflectionSummary,
    SkillSummary,
};

// Sub-module re-exports for direct path access
pub use backends::episodic;
pub use ops::activation;
pub use ops::decay;
pub use ops::schema;

#[cfg(feature = "cognitive-memory")]
pub use backends::procedural;
#[cfg(feature = "cognitive-memory")]
pub use backends::self_memory;
#[cfg(feature = "cognitive-memory")]
pub use backends::semantic;
#[cfg(feature = "cognitive-memory")]
pub use ops::consolidation;
#[cfg(feature = "cognitive-memory")]
pub use ops::router;

#[cfg(test)]
pub mod testing;

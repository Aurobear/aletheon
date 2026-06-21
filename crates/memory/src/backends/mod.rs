//! Storage backends for memory subsystems.
//!
//! Each backend implements the `MemoryBackend` trait for its memory type:
//! - `EpisodicMemory` — events, reflections, observations
//! - `SemanticMemory` — knowledge, concepts, facts (with FTS5 + vector search)
//! - `ProceduralMemory` — skills, workflows, reusable patterns
//! - `SelfMemory` — identity changes, lineage graph, boundary decisions

pub mod episodic;
pub mod procedural;
pub mod self_memory;
pub mod semantic;

pub use episodic::EpisodicMemory;
pub use procedural::ProceduralMemory;
pub use self_memory::SelfMemory;
pub use semantic::SemanticMemory;

//! Storage backends for memory subsystems.
//!
//! Each backend implements the `MemoryBackend` trait for its memory type:
//! - `EpisodicMemory` — events, reflections, observations (always built)
//! - `SemanticMemory` — knowledge, concepts, facts (cognitive-memory feature)
//! - `ProceduralMemory` — skills, workflows (cognitive-memory feature)
//! - `SelfMemory` — identity changes, lineage (cognitive-memory feature)

pub mod episodic;
#[cfg(feature = "cognitive-memory")]
pub mod procedural;
#[cfg(feature = "cognitive-memory")]
pub mod self_memory;
#[cfg(feature = "cognitive-memory")]
pub mod semantic;

pub use episodic::EpisodicMemory;
#[cfg(feature = "cognitive-memory")]
pub use procedural::ProceduralMemory;
#[cfg(feature = "cognitive-memory")]
pub use self_memory::SelfMemory;
#[cfg(feature = "cognitive-memory")]
pub use semantic::SemanticMemory;

//! # Aletheon Memory
//!
//! SQLite-backed implementations of the `MemoryBackend` trait for all 4
//! memory types: Episodic, Semantic, Procedural, and Self.
//!
//! Each backend has its own SQLite file (no lock contention).
//! `MemoryRouter` dispatches by `MemoryType`.

pub mod activation;
pub mod decay;
pub mod episodic;
pub mod procedural;
pub mod router;
pub mod schema;
pub mod self_memory;
pub mod semantic;

// Re-export primary types
pub use episodic::EpisodicMemory;
pub use procedural::ProceduralMemory;
pub use router::MemoryRouter;
pub use self_memory::SelfMemory;
pub use semantic::SemanticMemory;

#[cfg(test)]
pub mod testing;

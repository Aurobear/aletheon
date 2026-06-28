//! # Aletheon Memory
//!
//! SQLite-backed implementations of the `MemoryBackend` trait for all 4
//! memory types: Episodic, Semantic, Procedural, and Self.
//!
//! Each backend has its own SQLite file (no lock contention).
//! `MemoryRouter` dispatches by `MemoryType`.

pub mod schema;
pub mod episodic;
pub mod semantic;
pub mod procedural;
pub mod self_memory;
pub mod router;
pub mod activation;
pub mod decay;

// Re-export primary types
pub use episodic::EpisodicMemory;
pub use semantic::SemanticMemory;
pub use procedural::ProceduralMemory;
pub use self_memory::SelfMemory;
pub use router::MemoryRouter;

#[cfg(test)]
pub mod testing;

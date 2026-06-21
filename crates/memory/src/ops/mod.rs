//! Memory operations — routing, consolidation, decay, activation, schema.
//!
//! - `router` — dispatches to the correct backend by `MemoryType`
//! - `consolidation` — promotes episodic reflections to semantic knowledge
//! - `decay` — Ebbinghaus forgetting curve computation
//! - `activation` — ACT-R-inspired activation scoring
//! - `schema` — shared SQLite schema helpers

pub mod activation;
pub mod consolidation;
pub mod decay;
pub mod router;
pub mod schema;

pub use activation::{compute_activation, ActivationEntry};
pub use consolidation::{ConsolidationConfig, ConsolidationResult};
pub use decay::{apply_access_boost, compute_strength, should_forget};
pub use router::{MemoryContext, MemoryRouter, ReflectionSummary, SkillSummary};

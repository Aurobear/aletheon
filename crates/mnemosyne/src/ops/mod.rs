//! Memory operations — routing, consolidation, decay, activation, schema.

pub mod activation;
#[cfg(feature = "cognitive-memory")]
pub mod consolidation;
pub mod decay;
#[cfg(feature = "cognitive-memory")]
pub mod router;
pub mod schema;

pub use activation::{compute_activation, ActivationEntry};
#[cfg(feature = "cognitive-memory")]
pub use consolidation::{ConsolidationConfig, ConsolidationResult};
pub use decay::{apply_access_boost, compute_strength, should_forget};
#[cfg(feature = "cognitive-memory")]
pub use router::{MemoryContext, MemoryRouter, ReflectionSummary, SkillSummary};

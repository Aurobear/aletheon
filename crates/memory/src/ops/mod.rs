//! Memory operations — routing, consolidation, decay, activation, schema.

pub mod activation;
pub mod decay;
pub mod schema;
#[cfg(feature = "cognitive-memory")]
pub mod router;
#[cfg(feature = "cognitive-memory")]
pub mod consolidation;

pub use activation::{compute_activation, ActivationEntry};
pub use decay::{apply_access_boost, compute_strength, should_forget};
#[cfg(feature = "cognitive-memory")]
pub use consolidation::{ConsolidationConfig, ConsolidationResult};
#[cfg(feature = "cognitive-memory")]
pub use router::{MemoryContext, MemoryRouter, ReflectionSummary, SkillSummary};

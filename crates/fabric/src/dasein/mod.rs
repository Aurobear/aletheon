//! DaseinModule ABI types — pure interfaces, zero implementations.
//!
//! Philosophy grounding:
//! - Stimmung: Heidegger's Befindlichkeit (attunement)
//! - TemporalStream: Husserl's inner time consciousness (retention-primal impression-protention)
//! - Bewandtnisganzheit: Heidegger's involvement whole (meaningful relational network)
//! - MutableSelfModel: Sartre's pour-soi (self-negating being-for-itself)
//! - CareStructure: Heidegger's Sorge (care = projection + thrownness + fallenness)

pub mod context;
pub mod event;
pub mod ops;
pub mod transition;
pub mod types;

pub use context::*;
pub use event::*;
pub use ops::*;
pub use transition::*;
pub use types::*;

//! RFC-017 primitives — the canonical shared vocabulary.
//!
//! Every subsystem communicates using these primitives instead of concrete
//! implementations. Pure types only; no business logic.

pub mod cognitive;

pub use cognitive::{Evidence, Hypothesis};

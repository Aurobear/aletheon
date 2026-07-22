pub mod bridge;
pub mod core;
pub mod hil_evidence_verifier;
#[path = "impl/mod.rs"]
mod r#impl;
pub mod outcome_verifier;
pub mod service;

pub use core::traits::DefaultMetaRuntime;
pub use core::types::*;
pub use service::*;

/// Stable mutation-intent facade consumed by Executive evolution policy.
pub mod evolution {
    pub use crate::r#impl::morphogenesis::mutation_intent::MutationIntentGenerator;
}

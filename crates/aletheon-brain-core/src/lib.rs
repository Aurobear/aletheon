//! # Aletheon BrainCore
//!
//! The cognitive computation engine — handles "how do I?" reasoning, planning,
//! critique, reflection, and learning.
//!
//! BrainCore has NO self. It does not decide "should I?" (that is SelfField's job).
//! It produces plans, evaluates them, reflects on outcomes, and extracts rules.
//!
//! ## Components
//!
//! - **Reasoner** — multi-strategy reasoning (Direct, ChainOfThought)
//! - **Planner** — intent → Plan with PlanSteps + rollback actions
//! - **Reflector** — post-execution reflection (what worked/failed/improve)
//! - **Critic** — multi-dimensional critique (correctness, completeness, risk, efficiency, reversibility)
//! - **Learner** — experience → learned rules
//! - **WorldModel** — environment state tracking via observations
//! - **BrainCore** — wires all components, implements BrainCoreOps + Subsystem

pub mod config;
pub mod core;
pub mod bridge;
pub mod r#impl;

// Re-export core components
pub use core::{BrainCore, BrainCoreConfig};
pub use core::reasoner::{Reasoner, ReasoningStrategy};
pub use core::planner::Planner;
pub use core::critic::Critic;
pub use core::reflector::Reflector;
pub use core::learner::Learner;
pub use core::world_model::WorldModel;

// Re-export bridge components
pub use bridge::llm::LlmBridge;
pub use bridge::inference::InferenceBridge;
pub use bridge::learning::LearningBridge;

// Re-export impl components
pub use r#impl::llm;
pub use r#impl::inference;
pub use r#impl::learning;
pub use r#impl::provider_registry;

#[cfg(test)]
pub mod testing;

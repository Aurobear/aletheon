//! # Aletheon Cognition
//!
//! The cognitive computation engine — handles "how do I?" reasoning, planning,
//! critique, reflection, and learning.
//!
//! Cognition has NO self. It does not decide "should I?" (that is SelfField's job).
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
//! - **Harness** — production sessions compose the focused components through
//!   `harness`.

pub mod bridge;
pub mod config;
pub mod core;
pub mod harness;
pub mod r#impl;

// Re-export core components
pub use core::critic::Critic;
pub use core::learner::Learner;
pub use core::planner::Planner;
pub use core::reasoner::{Reasoner, ReasoningStrategy};
pub use core::reflector::Reflector;
pub use core::world_model::WorldModel;

// Re-export bridge components
pub use bridge::dual_model::{DualModelBridge, DualModelConfig, TaskComplexity};
pub use bridge::inference::InferenceBridge;
pub use bridge::learning::LearningBridge;
pub use bridge::llm::LlmBridge;

// Re-export harness components
pub use harness::config::HarnessConfig;
pub use harness::{
    CanonicalTurnEventSink, ChannelCognitiveStreamSink, CognitError, CognitErrorKind,
    CognitRetryDisposition, CognitiveSession, CognitiveSessionDependencies, CognitiveStreamEvent,
    CognitiveStreamSink, HarnessKind,
};
pub use r#impl::inference;
pub use r#impl::learning;
pub use r#impl::llm;
pub use r#impl::provider_registry;

pub mod testing;

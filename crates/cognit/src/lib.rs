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

pub(crate) mod adapters;
mod application;
pub mod bridge;
pub mod composition;
pub mod config;
pub mod core;
pub mod harness;
pub mod ports;

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
/// Stable inference contracts and the runtime scheduling facade.
///
/// Provider transports remain private under `adapters`; consumers receive only
/// shared contracts and the host-facing scheduler/pulse types.
pub mod inference {
    pub use crate::application::inference::*;

    pub mod provider {
        pub use crate::adapters::inference::provider::*;
    }
    pub use provider::*;

    pub mod pulse {
        pub use crate::adapters::inference::pulse::*;
    }
    pub use pulse::*;

    pub mod scheduler {
        pub use crate::adapters::inference::scheduler::*;
    }
    pub use scheduler::*;
}

/// Stable learning domain facade.
pub mod learning {
    pub use crate::application::learning::*;
}

/// Host-facing event observers used to connect cognition to the event spine.
pub mod event_handlers {
    pub use crate::application::event_handlers::*;
}

/// Policy client facade. Transport selection remains owned by composition.
pub mod policy {
    pub use crate::adapters::policy::*;
}

pub mod testing;

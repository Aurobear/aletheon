//! Cognitive harnesses — pluggable reasoning pipelines.
//!
//! Harnesses orchestrate the cognitive flow: Goal → Context → Planner → Reasoner →
//! Executor → Verifier → Reflector → Memory Update.
//!
//! The General production path composes [`HarnessCognitiveSession`] (owned by this
//! crate) and drives turns with [`HarnessLoopDriver`] (see [`driver`]): a minimal
//! model → tools → repeat loop whose policy, compaction, goals, verification,
//! memory and robot behavior are plugged in as lifecycle hooks.
//!
//! `ReActLoop` (the `linear` module) is retained as the legacy test/compat
//! implementation — it is not part of the General production path and must not be
//! re-introduced at the daemon construction point. Future harnesses
//! (ResearchHarness, CodingHarness, OSHarness) will live here alongside `robot`.

use serde::{Deserialize, Serialize};

pub mod agent;
pub mod config;
pub mod core_session;
pub mod driver;
pub mod event_sink;
pub mod factory;
pub mod interrupt;
pub mod lifecycle;
pub mod linear;
pub mod robot;
pub mod session;
pub mod session_log;

pub use config::HarnessConfig;
pub use core_session::HarnessCognitiveSession;
pub use factory::{
    selected_harness_kind, CognitiveSessionFactory, ExecutionTargetRoutingError,
    HarnessCognitiveSessionFactory, LinearCognitiveSessionFactory, RobotSessionCapability,
    TargetRoutedCognitiveSessionFactory,
};
pub use linear as react_loop; // backward-compat: ReActLoop is the linear harness
pub use linear::{BatchPlanner, CompactorTrait, ReActLoop};
pub use session::{
    CanonicalRuntimeTurnEventSink, CanonicalTurnEventSink, ChannelCognitiveStreamSink, CognitError,
    CognitErrorKind, CognitRetryDisposition, CognitiveSession, CognitiveSessionDependencies,
    CognitiveStreamEvent, CognitiveStreamSink, LinearCognitiveSession,
};

/// Stable identity for a configured cognitive harness capability.
///
/// Construction goes through [`CognitiveSessionFactory`], which is object-safe
/// for both Linear and Robot sessions. `HarnessKind` is selection metadata; it
/// does not construct an incomplete concrete loop without its required ports.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum HarnessKind {
    #[default]
    Linear,
    /// RobotHarness — bounded embodied execution with outcome verification.
    Robot,
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod harness_kind_tests {
    use super::HarnessKind;

    #[test]
    fn robot_now_parses() {
        assert!(serde_json::from_str::<HarnessKind>(r#""robot""#).is_ok());
    }

    #[test]
    fn linear_remains_default() {
        assert_eq!(HarnessKind::default(), HarnessKind::Linear);
    }
}

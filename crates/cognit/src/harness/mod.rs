//! Cognitive harnesses — pluggable reasoning pipelines.
//!
//! Harnesses orchestrate the cognitive flow: Goal → Context → Planner → Reasoner →
//! Executor → Verifier → Reflector → Memory Update.
//!
//! Currently only `linear` (ReActLoop) is implemented. Future harnesses
//! (ResearchHarness, CodingHarness, RobotHarness, OSHarness) will live here.

use serde::{Deserialize, Serialize};

pub mod config;
pub mod event_sink;
pub mod interrupt;
pub mod linear;

pub use config::HarnessConfig;
pub use linear as react_loop; // backward-compat: ReActLoop is the linear harness
pub use linear::{CompactorTrait, ReActLoop};

/// Selects which concrete harness implementation `build_harness` constructs.
///
/// Phase 2 fallback (see RFC-018): `ReActLoop::run` is generic over
/// `<L: LlmProvider, F: Fn(...) -> Fut, Fut: Future>`, which is not
/// object-safe (generic methods cannot be part of a `dyn Trait`). Rather than
/// force an incompatible trait-object seam, this factory keeps `ReActLoop` as
/// a concrete return type and provides the config-selected construction seam:
/// adding a new harness kind means adding a variant here and a construction
/// arm in `build_harness`, without touching `executive`'s call sites beyond
/// the `HarnessKind` selection itself.
///
/// Selectable from TOML via `harness_kind = "linear"` (see
/// `executive::core::config::RuntimeConfig::harness_kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HarnessKind {
    #[default]
    Linear,
}

/// Construct a harness for the given `kind`.
///
/// Currently only `HarnessKind::Linear` (ReActLoop) is implemented. Future
/// harnesses (Research/Coding/Robot) should add a variant to `HarnessKind`
/// and a matching construction arm here.
pub fn build_harness(
    kind: HarnessKind,
    config: HarnessConfig,
    compressor: Box<dyn CompactorTrait>,
) -> ReActLoop {
    match kind {
        HarnessKind::Linear => ReActLoop::new(config, compressor),
    }
}

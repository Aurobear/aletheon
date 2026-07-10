//! Cognitive harnesses — pluggable reasoning pipelines.
//!
//! Harnesses orchestrate the cognitive flow: Goal → Context → Planner → Reasoner →
//! Executor → Verifier → Reflector → Memory Update.
//!
//! Currently only `linear` (ReActLoop) is implemented. Future harnesses
//! (ResearchHarness, CodingHarness, RobotHarness, OSHarness) will live here.

pub mod config;
pub mod event_sink;
pub mod interrupt;
pub mod linear;

pub use config::HarnessConfig;
pub use linear as react_loop; // backward-compat: ReActLoop is the linear harness

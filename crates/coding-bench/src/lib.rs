//! Coding benchmark harness — ≥30 task categories, metrics, and release gate (Wave 5).

pub mod profile;
pub mod metrics;
pub mod tasks;
pub mod runner;

pub use profile::{DeploymentProfile, ProfileKind, FeatureSet};
pub use metrics::{BenchmarkMetrics, GateThresholds, GateResult};
pub use tasks::{BenchmarkTask, TaskCategory};
pub use runner::{BenchmarkRunner, RunResult};

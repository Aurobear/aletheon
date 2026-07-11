//! Supervision policies for process failures.

pub mod tree;

pub use tree::{GroupStrategy, RestartDecision, RestartPolicy, SupervisorTree};

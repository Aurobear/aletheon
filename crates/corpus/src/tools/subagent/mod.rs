//! Isolated sub-agent execution primitives.

pub mod command;

pub use command::{CommandOutput, CommandRequest, CommandRunner, CommandRunnerError};

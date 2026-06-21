//! User interaction: TUI, CLI, and ACIX.

pub mod ui;
pub mod cli;
pub mod acix;

/// Backward compatibility: acix_tools is now acix::tools
pub use acix::tools as acix_tools;

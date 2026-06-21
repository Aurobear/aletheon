//! User interaction: TUI and ACIX.

pub mod tui;
pub mod acix;

/// Backward compatibility: acix_tools is now acix::tools
pub use acix::tools as acix_tools;

/// Backward compatibility: cli module is now tui::cli
pub use tui::cli;

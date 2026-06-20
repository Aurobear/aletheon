//! Implementation modules — tools, sandbox, driver, security, mcp, and acix.

pub mod driver;
pub mod hooks;
pub mod mcp;
pub mod platform;
pub mod sandbox;
pub mod security;
pub mod tools;

// acix requires input, display, and a11y driver features
#[cfg(all(feature = "input", feature = "display", feature = "a11y"))]
pub mod acix;

pub mod skills;
pub mod ui;

#[cfg(feature = "cli")]
pub mod cli;

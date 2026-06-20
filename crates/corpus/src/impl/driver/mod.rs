//! SDK/driver layer — Linux kernel interface bindings.
//!
//! Each module wraps a specific Linux kernel interface or system service
//! with safe Rust APIs. Feature-gated by hardware capability.

pub mod types;

#[cfg(feature = "input")]
pub mod input;

#[cfg(feature = "display")]
pub mod display;

#[cfg(feature = "a11y")]
pub mod a11y;

#[cfg(feature = "sandbox-primitives")]
pub mod sandbox_driver;

#[cfg(feature = "ocr")]
pub mod ocr;

pub mod factory;

pub mod io;
pub mod proc;

pub use types::*;

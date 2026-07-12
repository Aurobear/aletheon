//! Hardware drivers and platform adapters.

pub mod factory;
pub mod io;
pub mod proc;
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

pub mod platform;

pub use types::*;

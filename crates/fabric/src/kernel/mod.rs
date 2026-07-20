//! Core infrastructure — observability, registry, debug, error handling.

pub mod debug;
pub mod debug_bus;
pub mod observable;
pub mod registry;

// error module extracted to fabric; re-export for backward compat
pub mod error;

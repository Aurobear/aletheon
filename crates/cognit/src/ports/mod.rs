//! Ports — async trait boundaries for external dependency injection.
//!
//! Each port defines a stable contract that Cognit depends on without
//! coupling to any particular implementation.

pub mod cognitive_run;
pub mod policy_provider;

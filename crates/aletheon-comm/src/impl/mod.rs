//! # Implementation — Concrete comm implementations
//!
//! Contains the concrete implementations of event bus, event log,
//! routing policy, subscription registry, and IPC backends.

pub mod kernel_bus;
pub mod event_log;
pub mod routing_policy;
pub mod subscription;
pub mod ipc;

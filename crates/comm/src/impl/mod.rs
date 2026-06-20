//! # Implementation — Concrete comm implementations
//!
//! Contains the concrete implementations of event bus, event log,
//! routing policy, subscription registry, and IPC backends.

pub mod communication_bus;
pub mod debug_bus;
pub mod event_log;
pub mod in_process;
pub mod ipc;
pub mod kernel_bus;
pub mod pubsub;
pub mod request_response;
pub mod routing_policy;
pub mod subscription;
pub mod unix_socket_transport;

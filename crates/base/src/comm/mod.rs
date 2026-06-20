//! Communication subsystem implementation.
//!
//! This module contains the concrete implementations of the communication
//! protocols and transports defined in the base crate.

pub mod bridge;
pub mod r#impl;

// Re-export main types at comm level for convenience
pub use bridge::event_bridge::EventBridge;
pub use r#impl::communication_bus::{BusConfig, CommunicationBus};
pub use r#impl::debug_bus::{DebugBusHook, EventFilter, EventRecorder, PerfCounter};
pub use r#impl::event_log::{EventLog, LogEntry};
pub use r#impl::in_process::InProcessTransport;
pub use r#impl::kernel_bus::KernelEventBus;
pub use r#impl::pubsub::PubSubProtocol;
pub use r#impl::request_response::RequestResponseProtocol;
pub use r#impl::routing_policy::{RouteAction, RoutingPolicy};
pub use r#impl::subscription::SubscriptionRegistry;
pub use r#impl::unix_socket_transport::UnixSocketTransport;

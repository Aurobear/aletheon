//! # Bridge — Adapters between core traits and impl
//!
//! Provides bridge types that connect the abstract core traits
//! to concrete implementations.

pub mod event_bridge;

pub use event_bridge::EventBridge;

// Re-export commonly used impl types for convenience
pub use crate::r#impl::communication_bus::{BusConfig, CommunicationBus};
pub use crate::r#impl::kernel_bus::KernelEventBus;
pub use crate::r#impl::event_log::EventLog;
pub use crate::r#impl::subscription::SubscriptionRegistry;
pub use crate::r#impl::routing_policy::{RouteAction, RoutingPolicy};

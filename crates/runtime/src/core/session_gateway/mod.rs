//! Session Gateway — unified facade for external Agent debug access.
//!
//! Provides structured state queries and real-time event streams under the
//! `session.*` JSON-RPC namespace.  Built on top of the existing
//! [`DebugHandler`](crate::impl::daemon::debug_handler) infrastructure.
//!
//! ## Sub-modules
//!
//! - [`param_registry`] — dynamic parameter registration (ROS `rosparam` equivalent)
//! - [`subsystem_query`] — per-module structured state export trait
//! - [`snapshot`] — runtime snapshot builder (markdown output)
//! - [`gateway`] — main `SessionGateway` struct + JSON-RPC method dispatch
//!
//! ## Design doc
//!
//! `docs/plans/2026-07-03-session-gateway-design.md`

pub mod param_registry;
pub mod subsystem_query;
pub mod snapshot;
pub mod gateway;

pub use param_registry::ParamRegistry;
pub use subsystem_query::{SubsystemQuery, SubsystemRegistry, QueryError};
pub use gateway::{SessionGateway, SessionStateRef};
pub use snapshot::SnapshotBuilder;

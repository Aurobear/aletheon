//! Session Gateway — unified facade for external Agent debug access.
//!
//! Provides structured state queries and real-time event streams under the
//! `session.*` JSON-RPC namespace.  Built on top of the existing
//! [`DebugHandler`](crate::host::daemon::debug_handler) infrastructure.
//!
//! ## Sub-modules
//!
//! - [`gateway`] — main `SessionGateway` struct + JSON-RPC method dispatch
//! - [`session_state`] — SessionStateRef, state update methods, state query handler
//! - [`turn_context`] — Snapshot, memory, self, dasein, and ask handlers
//! - [`approval_flow`] — Param and journal handlers
//! - [`param_registry`] — dynamic parameter registration (ROS `rosparam` equivalent)
//! - [`subsystem_query`] — per-module structured state export trait
//! - [`snapshot`] — runtime snapshot builder (markdown output)
//!
//! ## Design doc
//!
//! `docs/plans/2026-07-03-session-gateway-design.md`

pub mod gateway;
pub mod param_registry;
pub mod snapshot;
pub mod subsystem_query;

pub mod approval_flow;
pub mod session_state;
pub mod turn_context;

pub use gateway::{SessionGateway, SessionStateRef};
pub use param_registry::ParamRegistry;
pub use snapshot::SnapshotBuilder;
pub use subsystem_query::{QueryError, SubsystemQuery, SubsystemRegistry};

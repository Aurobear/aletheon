//! Aletheon Gateway — the neutral channel dispatch engine.
//!
//! This crate holds channel-provider-agnostic types (intents, effects,
//! durable store, capability registry, dispatcher) plus the concrete
//! Telegram transport. It depends only on `fabric` and infrastructure
//! crates (tokio/reqwest/rusqlite/...) — never on `executive`. Executive
//! implements the ports declared here (transports, turn/goal executors,
//! approval repositories) and wires the dispatcher into the daemon.

pub mod dispatcher;
pub mod effect;
pub mod handlers;
pub mod intent;
pub mod notify;
pub mod ports;
pub mod registry;
pub mod store;
pub mod telegram;

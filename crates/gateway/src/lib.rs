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
mod adapters;

mod store_facade {
    pub use crate::adapters::sqlite_store::{ChannelStore, InsertOutcome};
}
pub use store_facade::{ChannelStore, InsertOutcome};

/// Construct the built-in owner channel behind the stable transport port.
pub fn build_telegram_transport(
    token: String,
    api_base: Option<String>,
    poll_timeout_secs: u64,
    cancel: tokio_util::sync::CancellationToken,
) -> std::sync::Arc<dyn dispatcher::ChannelTransport> {
    std::sync::Arc::new(adapters::telegram::TelegramTransport::new(
        token, api_base, poll_timeout_secs, cancel,
    ))
}

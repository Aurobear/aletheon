//! Aletheon Gateway — the neutral channel dispatch engine.
//!
//! This crate holds channel-provider-agnostic types (intents, effects,
//! durable projection ports, capability registry, and routing). It depends
//! only on neutral contracts and async infrastructure, never on a physical
//! persistence implementation. Aletheon composition supplies concrete ports.
//!
//! The typed protocol, client, server route adapter and Telegram channel
//! transport were converged here from the former `gateway-protocol`,
//! `gateway-client`, `gateway-server` and `gateway-channel-telegram` owner
//! seam crates (Gateway five-crate convergence). Wire schema, protocol
//! version and error codes are unchanged.

pub mod capability;
pub mod channel;
#[cfg(feature = "client")]
pub mod client;
pub mod effect;
pub mod external_read;
pub mod intent;
pub mod notify;
pub mod ports;
#[cfg(feature = "protocol")]
pub mod protocol;
pub mod registry;
pub mod router;
#[cfg(feature = "server")]
pub mod server;

pub use ports::ChannelProjectionStore;

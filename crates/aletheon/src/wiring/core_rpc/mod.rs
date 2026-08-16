//! User-daemon client for the machine inference host.
//!
//! The protocol and client live with the binary composition root; the machine
//! core server remains a separately supervised host until the InferenceBroker
//! extraction is complete.

pub mod client;
pub mod protocol;
pub mod server;

pub use client::CoreRpcClient;
pub use protocol::{CoreFrame, CoreRequest, DEFAULT_MAX_FRAME_BYTES};
pub use server::{CorePeerPolicy, CoreRpcServer};

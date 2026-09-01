//! Machine core RPC transport (crate-private).
//!
//! The wire schema and peer policy are private to this transport; HTTP provider
//! modules never import `CorePeerPolicy`. Only the composition-root surface is
//! re-exported by the crate root.

pub mod client;
pub mod protocol;
pub mod server;

pub use client::{CoreRpcClient, CoreRpcReadinessError};
pub use protocol::{CoreFrame, CoreRequest, DEFAULT_MAX_FRAME_BYTES};
pub use server::{CorePeerPolicy, CoreRpcServer};

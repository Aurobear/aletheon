//! HTTP inference providers, machine core RPC transport, and machine
//! backpressure adapters.
//!
//! This crate owns the machine-wide provider registry, the HTTP provider
//! implementations, the authenticated local core RPC transport, and machine
//! backpressure. The `aletheon` host owns only socket paths, peer-policy
//! configuration and server lifecycle; it never imports provider internals.

pub mod backpressure;
pub mod factory;
pub mod registry;
pub mod runtime_facts;

pub(crate) mod providers;
pub(crate) mod utf8_stream;

// Core RPC is a crate-private transport; only the composition-root surface is
// re-exported so the host and integration tests never reach into the transport.
mod core_rpc;
pub use core_rpc::{
    CoreFrame, CorePeerPolicy, CoreRequest, CoreRpcClient, CoreRpcReadinessError, CoreRpcServer,
    DEFAULT_MAX_FRAME_BYTES,
};

mod registry_port;
pub use registry_port::RegistryInferencePort;

pub use registry::ProviderRegistry;

//! Retired Memory Gateway entry point — one-way compatibility re-export.
//!
//! The `MemoryGatewayService` implementation moved to `mnemosyne::memory_gateway`
//! (memory domain owner). This module re-exports the public surface so existing
//! Aletheon-internal callers and compatibility tests keep compiling while the
//! cutover completes. No new implementation is added here.

pub use mnemosyne::{
    MemoryGatewayService, SupplementalBindingNegotiator, SupplementalBindingRecallPort,
};

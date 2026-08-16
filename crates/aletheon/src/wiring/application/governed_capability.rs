//! Retired governed-capability entry point — one-way compatibility re-export.
//!
//! The `CapabilityService` / `TurnCapabilityInvoker` traits and governed
//! capability invoker moved to `kernel::capability::governed` (capability
//! domain owner). This module re-exports the public surface so existing
//! Aletheon-internal callers and compatibility tests keep compiling while the
//! cutover completes. No new implementation is added here.

pub use kernel::capability::governed::{
    ActionModulationSnapshot, AuthorizedInvocation, CapabilityExecutionContext,
    CapabilityRuntimeFactory, CapabilityService, GovernedActionDecision,
    GovernedActionLoop, GovernedActionLoopResolver, GovernedCapabilityInvoker,
    RegistryAuthorityProvider, SelectedActionContext, SelectedActionOutcomeReceipt,
    TurnAuthorityProvider, TurnCapabilityInvoker,
};

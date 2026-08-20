pub mod native_cognit;
pub mod pi_rpc;

pub use kernel::process::controller::{LinuxProcessController, LinuxRuntimeProcessSupervisor};
pub use native_cognit::{
    AgentProfileRegistry, NativeCognitRuntime, NativeCognitRuntimeResources, ResolvedAgentProfile,
    NATIVE_COGNIT_RUNTIME_ID,
};
pub use pi_rpc::{
    pi_manifest, pi_rpc_environment_from_process, PiDelegateBackend, PI_DELEGATE_ALIAS,
};

pub mod worktree_recovery;

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod test_registry;
pub mod turn_operations;

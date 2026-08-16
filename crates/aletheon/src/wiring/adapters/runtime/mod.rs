pub mod native_cognit;
pub mod pi;
pub mod pi_protocol;
pub mod pi_rpc;
pub mod provider_worker;

pub use kernel::process::controller::{LinuxProcessController, LinuxRuntimeProcessSupervisor};
pub use native_cognit::{
    AgentProfileRegistry, NativeCognitRuntime, NativeCognitRuntimeResources, ResolvedAgentProfile,
    NATIVE_COGNIT_RUNTIME_ID,
};
pub use pi::{pi_environment_from_process, ResolvedPiConfig, PI_CODER_RUNTIME_ID};
pub use pi_rpc::{
    pi_manifest, pi_rpc_environment_from_process, PiDelegateBackend, PI_DELEGATE_ALIAS,
};
pub use provider_worker::ProviderWorkerRuntime;

pub mod worktree_recovery;

#[doc(hidden)]
pub mod test_registry;

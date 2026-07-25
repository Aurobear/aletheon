#![allow(
    deprecated,
    clippy::too_many_arguments,
    clippy::module_inception,
    clippy::new_without_default,
    clippy::single_match,
    clippy::unnecessary_unwrap,
    clippy::ptr_arg,
    clippy::needless_update,
    clippy::manual_strip
)]

#[doc(hidden)]
pub(crate) mod adapters;
pub mod application;
pub(crate) mod compatibility;
pub mod composition;
pub mod core;
pub mod extensions;
pub mod tools;

pub mod host;
pub use kernel;

// Re-export from core for backward compatibility
pub use composition::config::{
    AgentConfig, AppConfig, DaemonConfig, ExecutiveConfig, HooksConfig, McpServerConfig,
    MemoryConfig, PluginsConfig, ProviderConfig, SandboxConfig, Transport,
};
pub use core::orchestrator::AletheonExecutive;
pub use core::verdict_handler::DefaultVerdictHandler;
pub use kernel::admission::ProductionAdmissionController;

// Stable application facades.
pub use application::{approval, conscious, goal, orchestration};

/// Composition-only runtime handles.
///
/// Hosts construct these concrete local components, then inject application
/// ports. Request handlers must not use this facade as a domain shortcut.
pub mod runtime {
    pub use crate::application::agent::AgentRuntime;

    pub mod health {
        pub use crate::application::health::*;
    }

    pub mod storage_quota {
        pub use crate::application::storage_quota::*;
    }

    pub mod events {
        pub use crate::adapters::events::*;
    }

    pub mod plugin {
        pub use crate::adapters::plugin::*;
    }

    pub mod session {
        pub use crate::adapters::session::*;
    }
}

/// Integration-test access to concrete external adapters.
///
/// This surface is not a production contract. It exists so black-box tests can
/// characterize adapter behavior while production code receives ports through
/// composition.
#[doc(hidden)]
pub mod testing {
    pub mod turn_coordinator {
        pub use crate::composition::turn_coordinator::*;
    }
    pub mod agent_control {
        pub use crate::adapters::agent_control::sqlite_repository::*;
    }
    pub mod artifact {
        pub use crate::adapters::artifact::*;
    }
    pub mod channel {
        pub use crate::adapters::channel::*;
    }
    pub mod coding_runtime {
        pub use crate::adapters::runtime::*;
    }
    pub mod external {
        pub use crate::adapters::external::*;
    }
    pub mod provider_account {
        pub use crate::adapters::google::*;
    }
    pub mod supplemental_memory {
        pub use crate::adapters::gbrain::*;
    }
}

pub use composition::TurnService;
pub use runtime::AgentRuntime;

// ── Re-exports for CLI exec path (bin crate uses these via executive) ───
pub use crate::composition::exec_session::ExecSessionBuilder;
pub use fabric::types::admission::RiskLevel;
pub use fabric::{
    AdmissionController, AdmissionRequest, CapabilityId, CapabilityRequest, CapabilityResult,
    CapabilityScope, ContentBlock, LlmProvider, LlmResponse, LlmStream, LocalOsPrincipal, Message,
    NoopTurnEventSink, OperationId, PrincipalId, ProcessId, RecallSet, SandboxRequirement,
    StopReason, StreamChunk, ToolDefinition, TurnRequest, TurnServices, Usage, UsageReport,
};

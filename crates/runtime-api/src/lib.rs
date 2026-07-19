//! Capability Runtime Framework — transport-agnostic contracts for Pi,
//! Codex, Grok, Hermes, Native Cognit, and ROS runtimes. No concrete
//! runtime dependency lives here.

pub mod manifest;
pub mod work_order;
pub mod lifecycle;
pub mod events;
pub mod receipt;

pub use manifest::{RuntimeCapability, RuntimeManifest, InteractionMode, WorkspaceMode, ToolGovernance};
pub use work_order::{WorkOrder, TaskKind, AcceptanceCriterion, VerificationPlan};
pub use lifecycle::{CapabilityRuntime, RuntimeHandle, RuntimeSnapshot, PreparedRuntime};
pub use events::{RuntimeEvent, ToolRequestEvent, CommandOutputEvent};
pub use receipt::{RuntimeReceipt, CompletionStatus, RuntimeUsage};

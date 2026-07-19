//! Pi Coding Runtime adapter — implements CapabilityRuntime for Pi RPC.
//! Production wiring in Wave 3 (merge pi-coder + pi-rpc).

use runtime_api::manifest::{InteractionMode, RuntimeCapability, RuntimeManifest, ToolGovernance, WorkspaceMode};
use runtime_api::work_order::WorkOrder;
use runtime_api::lifecycle::{CapabilityRuntime, PreparedRuntime, RuntimeHandle};
use runtime_api::events::RuntimeEvent;
use runtime_api::receipt::{CompletionStatus, RuntimeReceipt, RuntimeUsage};
use async_trait::async_trait;
use std::collections::BTreeSet;
use std::sync::Arc;

pub struct PiRuntime {
    manifest: RuntimeManifest,
}

impl PiRuntime {
    pub fn new() -> Self {
        Self {
            manifest: RuntimeManifest {
                id: "pi/coding".into(),
                aliases: vec!["pi".into()],
                display_name: "Pi Coding Runtime".into(),
                capabilities: BTreeSet::from([
                    RuntimeCapability::CodeRead,
                    RuntimeCapability::CodeSearch,
                    RuntimeCapability::CodeEdit,
                    RuntimeCapability::Shell,
                    RuntimeCapability::Test,
                ]),
                interaction_modes: BTreeSet::from([
                    InteractionMode::OneShot,
                    InteractionMode::Resident,
                    InteractionMode::Steering,
                    InteractionMode::FollowUp,
                ]),
                workspace_mode: WorkspaceMode::IsolatedWorktree,
                tool_governance: ToolGovernance::Observed,
            },
        }
    }
}

impl Default for PiRuntime {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl CapabilityRuntime for PiRuntime {
    fn manifest(&self) -> &RuntimeManifest { &self.manifest }
    async fn prepare(&self, _order: WorkOrder) -> Result<PreparedRuntime, String> {
        Err("Pi runtime not connected — production wiring in Wave 3".into())
    }
    async fn start(&self, _prepared: PreparedRuntime, _events: Arc<dyn runtime_api::RuntimeEventSink>) -> Result<RuntimeHandle, String> {
        Err("Pi runtime not connected".into())
    }
    async fn cancel(&self, _handle: RuntimeHandle) -> Result<(), String> {
        Ok(())
    }
    async fn settle(&self, _handle: RuntimeHandle) -> Result<RuntimeReceipt, String> {
        Ok(RuntimeReceipt {
            status: CompletionStatus::Blocked,
            output: "pi stub".into(),
            usage: RuntimeUsage { tokens_in: 0, tokens_out: 0, elapsed_ms: 0 },
            workspace_diff: None,
            errors: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_manifest_has_code_capabilities() {
        let rt = PiRuntime::new();
        assert!(rt.manifest().has(&RuntimeCapability::CodeEdit));
        assert!(rt.manifest().has(&RuntimeCapability::Shell));
    }
}

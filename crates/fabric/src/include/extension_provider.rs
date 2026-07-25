//! Capability provider traits for the extension platform.
//!
//! These traits define the contracts that Runtime adapters must implement.
//! Each trait corresponds to one CapabilityKind.

use async_trait::async_trait;
use serde_json::Value;

use crate::types::admission::RiskLevel;
use crate::types::agent_control::{AgentHandle, AgentSpawnRequest};
use crate::types::hook::{HookContext, HookMode, HookPoint, HookResult};
use crate::types::llm_types::ToolDefinition;

/// Provider for Tool capabilities (CapabilityKind::Tool).
#[async_trait]
pub trait ToolProvider: Send + Sync {
    /// Execute a tool call and return the result.
    async fn call(&self, name: &str, params: Value) -> anyhow::Result<Value>;

    /// List the tools this provider exposes.
    fn list_tools(&self) -> Vec<ToolDefinition>;

    /// Risk level for the named tool.
    fn risk_level(&self, name: &str) -> RiskLevel;
}

/// Provider for Hook capabilities (CapabilityKind::HookProvider).
#[async_trait]
pub trait HookProvider: Send + Sync {
    /// Invoke a hook at the given point with the specified mode.
    async fn invoke(
        &self,
        point: HookPoint,
        mode: HookMode,
        ctx: HookContext,
    ) -> anyhow::Result<HookResult>;
}

/// Provider for Agent Runtime capabilities (CapabilityKind::AgentRuntimeProvider).
#[async_trait]
pub trait AgentRuntimeProvider: Send + Sync {
    /// Start a new agent session in this runtime.
    async fn start(&self, request: AgentSpawnRequest) -> anyhow::Result<AgentHandle>;

    /// Observe the current state of a running session.
    async fn observe(&self, handle: &AgentHandle) -> anyhow::Result<Value>;

    /// Steer an active session with new input.
    async fn steer(&self, handle: &AgentHandle, input: Value) -> anyhow::Result<()>;

    /// Send a follow-up message to a session.
    async fn follow_up(&self, handle: &AgentHandle, input: Value) -> anyhow::Result<Value>;

    /// Cancel a running session.
    async fn cancel(&self, handle: &AgentHandle, reason: &str) -> anyhow::Result<()>;

    /// Wait for a session to complete and collect the result.
    async fn wait(&self, handle: &AgentHandle) -> anyhow::Result<Value>;

    /// Check runtime health.
    async fn health(&self) -> anyhow::Result<()>;
}

/// Provider for Connector capabilities (CapabilityKind::ConnectorProvider).
#[async_trait]
pub trait ConnectorProvider: Send + Sync {
    /// List the tools this connector exposes.
    async fn list_tools(&self) -> anyhow::Result<Vec<ToolDefinition>>;

    /// Execute a connector tool call.
    async fn call(&self, name: &str, params: Value) -> anyhow::Result<Value>;

    /// Check connector health.
    async fn health(&self) -> anyhow::Result<()>;
}

//! Runtime-owned provider port for dynamically installed Agent backends.

use ::contracts::{AgentHandle, AgentSpawnRequest};
use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait AgentRuntimeProvider: Send + Sync {
    async fn start(&self, request: AgentSpawnRequest) -> anyhow::Result<AgentHandle>;
    async fn observe(&self, handle: &AgentHandle) -> anyhow::Result<Value>;
    async fn steer(&self, handle: &AgentHandle, input: Value) -> anyhow::Result<()>;
    async fn follow_up(&self, handle: &AgentHandle, input: Value) -> anyhow::Result<Value>;
    async fn cancel(&self, handle: &AgentHandle, reason: &str) -> anyhow::Result<()>;
    async fn wait(&self, handle: &AgentHandle) -> anyhow::Result<Value>;
    async fn health(&self) -> anyhow::Result<()>;
}

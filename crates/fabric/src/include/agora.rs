//! Agora (working-memory) trait contract — the shared cognitive workspace.
//!
//! Like the other subsystem contracts in `include/`, this defines the interface
//! Executive and the cognitive subsystems use to read/write the session-scoped
//! blackboard. The implementation lives in the `agora` crate (`AgoraRegistry`).
//!
//! Session-scoped, in-memory. Persists only via `snapshot()` → Mnemosyne.

use anyhow::Result;
use async_trait::async_trait;

/// Agora (working-memory) operations — the shared cognitive workspace.
#[async_trait]
pub trait AgoraOps: Send + Sync {
    /// Write a value onto a session's blackboard.
    async fn publish(&self, session: &str, key: &str, value: serde_json::Value) -> Result<()>;
    /// Read a value from a session's blackboard.
    async fn recall(&self, session: &str, key: &str) -> Result<Option<serde_json::Value>>;
    /// Merge a JSON patch into the session workspace.
    async fn update(&self, session: &str, patch: serde_json::Value) -> Result<()>;
    /// Snapshot the entire session workspace (for debug / commit).
    async fn snapshot(&self, session: &str) -> Result<serde_json::Value>;
    /// Clear a session's workspace.
    async fn clear(&self, session: &str) -> Result<()>;
    /// Append an entry onto a session's reasoning trace.
    async fn trace(&self, session: &str, kind: &str, content: serde_json::Value) -> Result<()>;
}

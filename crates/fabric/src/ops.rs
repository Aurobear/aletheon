//! Subsystem operation traits — the contract between Executive and subsystems.
//!
//! Each trait defines the interface that Executive uses to delegate work.
//! Implementations live in the respective subsystem crates and are wired
//! through CoreSystems in the runtime (Group B).

use anyhow::Result;
use async_trait::async_trait;

// ---------------------------------------------------------------------------
// Subsystem ops traits
// ---------------------------------------------------------------------------

/// Cognitive operations — reasoning, planning, reflection, learning.
#[async_trait]
pub trait CognitOps: Send + Sync {
    async fn build_context(
        &self,
        session_id: &str,
        messages: &[crate::Message],
    ) -> Result<serde_json::Value>;
    async fn reason(&self, ctx: &serde_json::Value, goal: &str) -> Result<serde_json::Value>;
    async fn reflect(&self, outcome: &serde_json::Value) -> Result<serde_json::Value>;
}

/// Dasein (self-field) operations — identity, boundary, narrative.
#[async_trait]
pub trait DaseinOps: Send + Sync {
    async fn review(&self, intent: &crate::Intent, ctx: &crate::Context) -> Result<crate::Verdict>;
    async fn narrate(&self, event: &str, detail: &str);
    async fn snapshot(&self) -> Result<serde_json::Value>;
}

/// Mnemosyne (memory) operations — recall, store, prompt composition.
#[async_trait]
pub trait MnemosyneOps: Send + Sync {
    async fn recall(&self, query: &str, limit: usize) -> Result<Vec<serde_json::Value>>;
    async fn store(&self, block: &serde_json::Value) -> Result<()>;
    async fn compose_prompt_block(&self, session_id: &str) -> Result<String>;
    async fn consolidate(&self) -> Result<()>;
}

/// Corpus (body) operations — tool execution, skill matching, hooks.
#[async_trait]
pub trait CorpusOps: Send + Sync {
    async fn execute_tool(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        session_id: &str,
    ) -> Result<crate::ToolResult>;
    async fn list_tools(&self) -> Result<Vec<crate::ToolDefinition>>;
    async fn run_hooks(&self, event: &crate::HookContext) -> Result<Vec<crate::HookResult>>;
}

/// Agora (working-memory) operations — the shared cognitive workspace.
///
/// Session-scoped, in-memory. Persists only via `snapshot()` → Mnemosyne.
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

// ---------------------------------------------------------------------------
// Harness traits
// ---------------------------------------------------------------------------

/// Tool executor — abstracts tool dispatch for harnesses.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, name: &str, input: serde_json::Value) -> Result<crate::ToolResult>;
}

/// A cognitive harness orchestrates a reasoning pipeline.
///
/// Harnesses are pluggable:
/// - LinearCognitiveHarness (current ReAct equivalent)
/// - Future: ResearchHarness, CodingHarness, RobotHarness, OSHarness
#[async_trait]
pub trait CognitiveHarness: Send + Sync {
    async fn run(
        &self,
        input: &str,
        messages: &[crate::Message],
        tool_defs: &[crate::ToolDefinition],
        executor: &dyn ToolExecutor,
    ) -> Result<(String, serde_json::Value)>;
}

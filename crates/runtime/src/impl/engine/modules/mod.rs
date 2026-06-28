//! Bus message protocol types for inter-module communication.
//!
//! Each request/response pair uses serde tagged enums for discriminated dispatch:
//! - Requests: `#[serde(tag = "op")]`
//! - Responses: `#[serde(tag = "result")]`

pub mod body_module;
pub mod memory_module;
pub mod perception_module;
pub mod self_field_module;

use base::self_field::{Care, Identity, Intent, Verdict};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

/// Request to the Memory module.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum MemoryRequest {
    /// Format core memory blocks for LLM context injection.
    FormatForContext,
    /// Store a recall memory entry.
    StoreRecall {
        session_id: String,
        entry_type: String,
        content: String,
        metadata: Option<String>,
    },
    /// Search recall memory by query.
    SearchRecall { query: String, limit: usize },
}

/// Response from the Memory module.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result")]
pub enum MemoryResponse {
    /// Formatted context string from core memory.
    ContextFormatted { text: String },
    /// Recall entry stored successfully.
    RecallStored { id: i64 },
    /// Recall search results.
    RecallSearchResults { entries: Vec<RecallEntry> },
    /// An error occurred.
    Error { message: String },
}

/// Serializable recall entry (mirrors `MemoryEntry` without `rusqlite` dependency).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallEntry {
    pub id: i64,
    pub session_id: String,
    pub entry_type: String,
    pub content: String,
    pub metadata: Option<String>,
}

// ---------------------------------------------------------------------------
// Body
// ---------------------------------------------------------------------------

/// Request to the Body module.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum BodyRequest {
    /// Get all tool definitions for LLM function-calling.
    Definitions,
    /// Get a specific tool by name.
    GetTool { name: String },
    /// List all registered tool names.
    ListTools,
}

/// Response from the Body module.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result")]
pub enum BodyResponse {
    /// Tool definitions for LLM.
    Definitions { tools: Vec<ToolDefinitionMsg> },
    /// Tool found.
    ToolFound { name: String, description: String },
    /// Tool not found.
    ToolNotFound { name: String },
    /// List of tool names.
    ToolList { names: Vec<String> },
    /// An error occurred.
    Error { message: String },
}

/// Serializable tool definition (mirrors `base::ToolDefinition`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinitionMsg {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

// ---------------------------------------------------------------------------
// SelfField
// ---------------------------------------------------------------------------

/// Request to the SelfField module.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum SelfFieldRequest {
    /// Review an intent through the policy pipeline.
    Review {
        intent: Intent,
        ctx: serde_json::Value,
    },
    /// Record a narrative entry.
    Narrate { event: String, reason: String },
    /// Get current identity.
    GetIdentity,
    /// Get current cares.
    GetCares,
}

/// Response from the SelfField module.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result")]
pub enum SelfFieldResponse {
    /// Verdict from review.
    Verdict { verdict: Verdict },
    /// Narrative recorded.
    Narrated,
    /// Current identity.
    Identity { identity: Identity },
    /// Current cares.
    Cares { cares: Vec<Care> },
    /// An error occurred.
    Error { message: String },
}

// ---------------------------------------------------------------------------
// Perception (pub-sub, no request/response)
// ---------------------------------------------------------------------------

/// Perception event message published to topic "perception.events".
///
/// This is a publish-only message (no request/response pattern).
/// Subscribers receive these via `subscribe_topic("perception.events")`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerceptionEventMsg {
    pub source: String,
    pub priority: String,
    pub summary: String,
    pub raw: serde_json::Value,
}

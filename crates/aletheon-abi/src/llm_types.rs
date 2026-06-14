//! LLM-related shared types.
//!
//! Merged from argos-types llm module into aletheon-abi.

use serde::{Deserialize, Serialize};

/// Tool definition sent to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

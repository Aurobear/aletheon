//! Reconstructable receipt for the exact context of one model inference.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelContextProjectionReceipt {
    pub inference_id: String,
    pub operation_id: String,
    pub role: String,
    pub stage: String,
    pub task_node_id: Option<String>,
    pub fragments: Vec<ModelContextFragmentReceipt>,
    pub omitted_fragment_ids: Vec<String>,
    pub message_bytes: u64,
    pub tool_schema_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelContextFragmentReceipt {
    pub fragment_id: String,
    pub source: String,
    pub source_version: String,
    pub artifact_ref: Option<String>,
    /// Present only while crossing the trusted persistence adapter. Canonical
    /// session receipts replace it with `artifact_ref` before append.
    pub inline_content: Option<String>,
    pub selection_reason: String,
    pub classification: ModelContextClassification,
    pub truncated: bool,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelContextClassification {
    Instruction,
    TrustedRuntime,
    UntrustedInput,
    ModelHistory,
    ToolEvidence,
}

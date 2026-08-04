use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::InferenceUsage;

pub const INFERENCE_TERMINAL_RECEIPT_SCHEMA_V1: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InferenceTerminalStatus {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InferenceTerminalReceipt {
    pub schema_version: u32,
    pub inference_id: String,
    pub operation_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub system_prefix_digest: String,
    pub tool_schema_digest: String,
    pub status: InferenceTerminalStatus,
    pub usage: InferenceUsage,
    pub failure_kind: Option<String>,
    /// Diagnostic identity of the cache-relevant request prefix (see
    /// `executive::host::daemon::cache_shape::InferencePrefixShape::digest`).
    /// Host-side diagnostic only; never a correctness dependency and never a
    /// claim that the provider hit or missed cache. `None` when the host did
    /// not compute a shape for this turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix_shape_digest: Option<String>,
    /// Host-side explanation for a changed stable prefix, or the conservative
    /// `provider_miss_or_eviction` inference when the shape stayed identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_cache_miss_reason: Option<String>,
}

impl InferenceTerminalReceipt {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != INFERENCE_TERMINAL_RECEIPT_SCHEMA_V1 {
            return Err("unsupported inference receipt schema version");
        }
        if [
            &self.inference_id,
            &self.operation_id,
            &self.provider_id,
            &self.model_id,
            &self.system_prefix_digest,
            &self.tool_schema_digest,
        ]
        .into_iter()
        .any(|value| value.trim().is_empty())
        {
            return Err("inference receipt identifiers and digests must be non-empty");
        }
        let failed = self.status != InferenceTerminalStatus::Succeeded;
        if failed != self.failure_kind.is_some() {
            return Err("failure_kind must be present exactly for non-success status");
        }
        if let Some(kind) = &self.failure_kind {
            if !matches!(
                kind.as_str(),
                "provider_transient"
                    | "provider_terminal"
                    | "context_overflow"
                    | "timeout"
                    | "cancelled"
                    | "invalid_request"
                    | "unknown"
            ) {
                return Err("unsupported inference failure kind");
            }
        }
        if let Some(reason) = &self.local_cache_miss_reason {
            if !matches!(
                reason.as_str(),
                "provider_or_model_changed"
                    | "transport_changed"
                    | "system_changed"
                    | "tool_schema_changed"
                    | "profile_changed"
                    | "compaction_or_rewrite"
                    | "provider_miss_or_eviction"
            ) {
                return Err("unsupported local cache miss reason");
            }
        }
        Ok(())
    }
}

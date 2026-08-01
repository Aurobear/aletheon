//! Versioned, content-minimizing Memory Agent maintenance contracts.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::memory::{
    MemoryLifecycleReceiptV1, MemoryObservationKindV1, MemoryProtocolValidationError,
    MemoryRecordKindV1, MemorySensitivityV1, MAX_MEMORY_CONTENT_BYTES, MAX_MEMORY_ID_BYTES,
    MAX_MEMORY_SOURCE_REFS, MAX_MEMORY_SOURCE_REF_BYTES,
};

pub const MEMORY_MAINTENANCE_SCHEMA_V1: u16 = 1;
pub const MAX_MAINTENANCE_ITEMS: u16 = 64;
pub const MAX_MAINTENANCE_REASON_CODES: usize = 32;
pub const MAX_MAINTENANCE_EVIDENCE_ITEMS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryMaintenancePhaseV1 {
    IntakeEvaluation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryMaintenanceStatusRequestV1 {
    pub request_id: String,
}

impl MemoryMaintenanceStatusRequestV1 {
    pub fn validate(&self) -> Result<(), MemoryProtocolValidationError> {
        validate_id("request_id", &self.request_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryMaintenanceRunRequestV1 {
    pub request_id: String,
    pub phase: MemoryMaintenancePhaseV1,
    pub max_items: u16,
    #[serde(default)]
    pub dry_run: bool,
}

impl MemoryMaintenanceRunRequestV1 {
    pub fn validate(&self) -> Result<(), MemoryProtocolValidationError> {
        validate_id("request_id", &self.request_id)?;
        if !(1..=MAX_MAINTENANCE_ITEMS).contains(&self.max_items) {
            return Err(MemoryProtocolValidationError(
                "maintenance max_items is outside its bounded range".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryMaintenanceStatusV1 {
    pub request_id: String,
    pub pending_items: u64,
    pub active_leases: u64,
    pub expired_leases: u64,
    pub oldest_pending_age_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryMaintenanceRunReceiptV1 {
    pub request_id: String,
    pub dry_run: bool,
    pub claimed: u16,
    pub promoted_local: u16,
    pub rejected: u16,
    pub deferred: u16,
    pub receipts: Vec<MemoryLifecycleReceiptV1>,
    pub reason_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryMaintenanceBudgetV1 {
    pub deadline_ms: u64,
    pub max_provider_rounds: u16,
    pub max_provider_retries: u16,
    pub max_tool_calls: u16,
    pub max_input_bytes: u64,
    pub max_output_bytes: u64,
    pub max_projected_writes: u16,
}

impl MemoryMaintenanceBudgetV1 {
    pub fn validate(&self) -> Result<(), MemoryProtocolValidationError> {
        if self.deadline_ms == 0
            || self.max_provider_rounds == 0
            || self.max_provider_retries > self.max_provider_rounds
            || self.max_tool_calls != 0
            || self.max_input_bytes == 0
            || self.max_input_bytes > MAX_MEMORY_CONTENT_BYTES as u64
            || self.max_output_bytes == 0
            || self.max_output_bytes > MAX_MEMORY_CONTENT_BYTES as u64
            || self.max_projected_writes > MAX_MAINTENANCE_ITEMS
        {
            return Err(MemoryProtocolValidationError(
                "memory maintenance budget is invalid or unbounded".into(),
            ));
        }
        Ok(())
    }
}

/// Proposal input supplied to an AgentRuntime. It contains no credentials,
/// writable workspace, authority, threshold, or final score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryMaintenanceTaskV1 {
    pub task_id: String,
    pub durable_intake_id: String,
    pub observation_kind: MemoryObservationKindV1,
    pub proposed_record_kind: MemoryRecordKindV1,
    pub content: String,
    pub sensitivity: MemorySensitivityV1,
    pub source_refs: Vec<String>,
    pub budget: MemoryMaintenanceBudgetV1,
}

impl MemoryMaintenanceTaskV1 {
    pub fn validate(&self) -> Result<(), MemoryProtocolValidationError> {
        validate_id("task_id", &self.task_id)?;
        validate_id("durable_intake_id", &self.durable_intake_id)?;
        if self.content.trim().is_empty() || self.content.len() > MAX_MEMORY_CONTENT_BYTES {
            return Err(MemoryProtocolValidationError(
                "maintenance task content is empty or exceeds byte limit".into(),
            ));
        }
        if self.source_refs.len() > MAX_MEMORY_SOURCE_REFS
            || self
                .source_refs
                .iter()
                .any(|value| value.trim().is_empty() || value.len() > MAX_MEMORY_SOURCE_REF_BYTES)
        {
            return Err(MemoryProtocolValidationError(
                "maintenance task source references are invalid".into(),
            ));
        }
        self.budget.validate()
    }
}

/// Untrusted semantic evidence. The host may use risk flags to lower a
/// decision, but this proposal cannot mint authority or raise a score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemorySemanticProposalV1 {
    pub schema_version: u16,
    pub task_id: String,
    pub control_instruction_detected: bool,
    pub contradiction_detected: bool,
    pub exact_duplicate_record_ids: Vec<String>,
    pub evidence: Vec<String>,
}

impl MemorySemanticProposalV1 {
    pub fn validate_for(&self, task_id: &str) -> Result<(), MemoryProtocolValidationError> {
        if self.schema_version != MEMORY_MAINTENANCE_SCHEMA_V1 || self.task_id != task_id {
            return Err(MemoryProtocolValidationError(
                "semantic proposal version or task binding is invalid".into(),
            ));
        }
        validate_id("task_id", &self.task_id)?;
        if self.exact_duplicate_record_ids.len() > MAX_MAINTENANCE_EVIDENCE_ITEMS
            || self.evidence.len() > MAX_MAINTENANCE_EVIDENCE_ITEMS
            || self
                .exact_duplicate_record_ids
                .iter()
                .chain(self.evidence.iter())
                .any(|value| value.trim().is_empty() || value.len() > MAX_MEMORY_SOURCE_REF_BYTES)
        {
            return Err(MemoryProtocolValidationError(
                "semantic proposal evidence exceeds its bounds".into(),
            ));
        }
        Ok(())
    }
}

fn validate_id(name: &str, value: &str) -> Result<(), MemoryProtocolValidationError> {
    if value.trim().is_empty() || value.len() > MAX_MEMORY_ID_BYTES {
        Err(MemoryProtocolValidationError(format!(
            "{name} is empty or exceeds byte limit"
        )))
    } else {
        Ok(())
    }
}

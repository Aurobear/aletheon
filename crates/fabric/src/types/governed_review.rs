//! Domain-neutral contracts for bounded, externally submitted evidence reviews.

use serde::{Deserialize, Serialize};

pub const REVIEW_SCHEMA_CURRENT: u16 = 2;
pub const REVIEW_SCHEMA_PREVIOUS: u16 = 1;
pub const MAX_REVIEW_ID_BYTES: usize = 128;
pub const MAX_REVIEW_TYPE_BYTES: usize = 128;
pub const MAX_REVIEW_REFERENCE_BYTES: usize = 2048;
pub const MAX_REVIEW_POLICY_REF_BYTES: usize = 2048;
pub const MAX_REVIEW_OPERATION_BYTES: usize = 128;
pub const MAX_REVIEW_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_REVIEW_ERROR_BYTES: usize = 2048;
pub const MAX_REVIEW_SUBJECT_REFS: usize = 256;
pub const MAX_REVIEW_EVIDENCE: usize = 256;
pub const MAX_REVIEW_OPERATIONS: usize = 64;
pub const MAX_REVIEW_FINDINGS: usize = 256;
pub const MAX_REVIEW_CHANGES: usize = 128;
pub const MAX_REVIEW_PRECONDITIONS: usize = 64;
pub const MAX_REVIEW_CAPABILITIES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReviewSchemaVersion(pub u16);

impl ReviewSchemaVersion {
    pub fn new(value: u16) -> Result<Self, ReviewContractError> {
        if matches!(value, REVIEW_SCHEMA_CURRENT | REVIEW_SCHEMA_PREVIOUS) {
            Ok(Self(value))
        } else {
            Err(ReviewContractError::UnsupportedSchema(value))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewBudget {
    pub max_input_bytes: u64,
    pub max_output_bytes: u64,
    pub max_tokens: u64,
    pub max_tool_calls: u32,
    pub wall_time_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewEvidence {
    pub reference: String,
    pub media_type: String,
    pub content: Option<String>,
    pub digest: String,
    pub observed_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedReviewJob {
    pub schema_version: u16,
    pub job_id: String,
    pub subject_type: String,
    pub subject_refs: Vec<String>,
    pub evidence: Vec<ReviewEvidence>,
    pub evidence_digest: String,
    pub policy_ref: String,
    pub allowed_operations: Vec<String>,
    pub budget: ReviewBudget,
    pub deadline_unix_ms: u64,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Queued,
    Running,
    Completed,
    Rejected,
    Cancelled,
    Expired,
    Failed,
}

impl ReviewStatus {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedReviewChange {
    pub operation: String,
    pub target_ref: String,
    pub preconditions: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub inference_requests: u32,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedReviewReceipt {
    pub schema_version: u16,
    pub job_id: String,
    pub status: ReviewStatus,
    pub evidence_digest: String,
    pub evidence_assessed: Vec<String>,
    pub findings: Vec<String>,
    pub proposed_changes: Vec<ProposedReviewChange>,
    pub confidence_millis: u16,
    pub unresolved_conflicts: Vec<String>,
    pub policy_decision: String,
    pub runtime_capabilities: Vec<String>,
    pub usage: ReviewUsage,
    pub started_at_unix_ms: Option<u64>,
    pub completed_at_unix_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReviewContractError {
    #[error("unsupported review schema version {0}")]
    UnsupportedSchema(u16),
    #[error("{0} must not be empty")]
    Empty(&'static str),
    #[error("{field} exceeds {max} bytes")]
    StringTooLong { field: &'static str, max: usize },
    #[error("{field} exceeds {max} entries")]
    TooMany { field: &'static str, max: usize },
    #[error("{0} must be a lowercase SHA-256 digest")]
    InvalidDigest(&'static str),
    #[error("review budget {0} must be positive")]
    ZeroBudget(&'static str),
    #[error("governed reviews cannot authorize tool calls")]
    ToolCallsForbidden,
    #[error("confidence_millis must be at most 1000")]
    InvalidConfidence,
    #[error("receipt status/timestamps are inconsistent")]
    InvalidStatusTimestamps,
}

fn string(value: &str, field: &'static str, max: usize) -> Result<(), ReviewContractError> {
    if value.trim().is_empty() {
        return Err(ReviewContractError::Empty(field));
    }
    if value.len() > max {
        return Err(ReviewContractError::StringTooLong { field, max });
    }
    Ok(())
}

fn digest(value: &str, field: &'static str) -> Result<(), ReviewContractError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ReviewContractError::InvalidDigest(field));
    }
    Ok(())
}

fn strings(
    values: &[String],
    field: &'static str,
    max_count: usize,
    max_bytes: usize,
) -> Result<(), ReviewContractError> {
    if values.len() > max_count {
        return Err(ReviewContractError::TooMany {
            field,
            max: max_count,
        });
    }
    for value in values {
        string(value, field, max_bytes)?;
    }
    Ok(())
}

impl GovernedReviewJob {
    pub fn validate(&self) -> Result<(), ReviewContractError> {
        ReviewSchemaVersion::new(self.schema_version)?;
        string(&self.job_id, "job_id", MAX_REVIEW_ID_BYTES)?;
        string(&self.subject_type, "subject_type", MAX_REVIEW_TYPE_BYTES)?;
        strings(
            &self.subject_refs,
            "subject_refs",
            MAX_REVIEW_SUBJECT_REFS,
            MAX_REVIEW_REFERENCE_BYTES,
        )?;
        if self.subject_refs.is_empty() {
            return Err(ReviewContractError::Empty("subject_refs"));
        }
        if self.evidence.is_empty() {
            return Err(ReviewContractError::Empty("evidence"));
        }
        if self.evidence.len() > MAX_REVIEW_EVIDENCE {
            return Err(ReviewContractError::TooMany {
                field: "evidence",
                max: MAX_REVIEW_EVIDENCE,
            });
        }
        for item in &self.evidence {
            string(
                &item.reference,
                "evidence.reference",
                MAX_REVIEW_REFERENCE_BYTES,
            )?;
            string(
                &item.media_type,
                "evidence.media_type",
                MAX_REVIEW_TYPE_BYTES,
            )?;
            string(
                &item.observed_version,
                "evidence.observed_version",
                MAX_REVIEW_REFERENCE_BYTES,
            )?;
            if let Some(content) = &item.content {
                if content.len() > self.budget.max_input_bytes as usize {
                    return Err(ReviewContractError::StringTooLong {
                        field: "evidence.content",
                        max: self.budget.max_input_bytes as usize,
                    });
                }
            }
            digest(&item.digest, "evidence.digest")?;
        }
        digest(&self.evidence_digest, "evidence_digest")?;
        string(&self.policy_ref, "policy_ref", MAX_REVIEW_POLICY_REF_BYTES)?;
        strings(
            &self.allowed_operations,
            "allowed_operations",
            MAX_REVIEW_OPERATIONS,
            MAX_REVIEW_OPERATION_BYTES,
        )?;
        string(
            &self.idempotency_key,
            "idempotency_key",
            MAX_REVIEW_ID_BYTES,
        )?;
        for (name, value) in [
            ("max_input_bytes", self.budget.max_input_bytes),
            ("max_output_bytes", self.budget.max_output_bytes),
            ("max_tokens", self.budget.max_tokens),
            ("wall_time_ms", self.budget.wall_time_ms),
            ("deadline_unix_ms", self.deadline_unix_ms),
        ] {
            if value == 0 {
                return Err(ReviewContractError::ZeroBudget(name));
            }
        }
        if self.budget.max_tool_calls != 0 {
            return Err(ReviewContractError::ToolCallsForbidden);
        }
        Ok(())
    }
}

impl GovernedReviewReceipt {
    pub fn validate(&self) -> Result<(), ReviewContractError> {
        ReviewSchemaVersion::new(self.schema_version)?;
        string(&self.job_id, "job_id", MAX_REVIEW_ID_BYTES)?;
        digest(&self.evidence_digest, "evidence_digest")?;
        strings(
            &self.evidence_assessed,
            "evidence_assessed",
            MAX_REVIEW_EVIDENCE,
            MAX_REVIEW_REFERENCE_BYTES,
        )?;
        strings(
            &self.findings,
            "findings",
            MAX_REVIEW_FINDINGS,
            MAX_REVIEW_TEXT_BYTES,
        )?;
        if self.proposed_changes.len() > MAX_REVIEW_CHANGES {
            return Err(ReviewContractError::TooMany {
                field: "proposed_changes",
                max: MAX_REVIEW_CHANGES,
            });
        }
        for change in &self.proposed_changes {
            string(&change.operation, "operation", MAX_REVIEW_OPERATION_BYTES)?;
            string(&change.target_ref, "target_ref", MAX_REVIEW_REFERENCE_BYTES)?;
            strings(
                &change.preconditions,
                "preconditions",
                MAX_REVIEW_PRECONDITIONS,
                MAX_REVIEW_TEXT_BYTES,
            )?;
            string(&change.rationale, "rationale", MAX_REVIEW_TEXT_BYTES)?;
        }
        strings(
            &self.unresolved_conflicts,
            "unresolved_conflicts",
            MAX_REVIEW_FINDINGS,
            MAX_REVIEW_TEXT_BYTES,
        )?;
        if !matches!(self.status, ReviewStatus::Queued | ReviewStatus::Running) {
            string(
                &self.policy_decision,
                "policy_decision",
                MAX_REVIEW_TEXT_BYTES,
            )?;
        } else if self.policy_decision.len() > MAX_REVIEW_TEXT_BYTES {
            return Err(ReviewContractError::StringTooLong {
                field: "policy_decision",
                max: MAX_REVIEW_TEXT_BYTES,
            });
        }
        strings(
            &self.runtime_capabilities,
            "runtime_capabilities",
            MAX_REVIEW_CAPABILITIES,
            MAX_REVIEW_OPERATION_BYTES,
        )?;
        if self.confidence_millis > 1000 {
            return Err(ReviewContractError::InvalidConfidence);
        }
        if self.status.is_terminal() != self.completed_at_unix_ms.is_some()
            || matches!(self.status, ReviewStatus::Running) && self.started_at_unix_ms.is_none()
        {
            return Err(ReviewContractError::InvalidStatusTimestamps);
        }
        if let Some(error) = &self.error {
            if error.len() > MAX_REVIEW_ERROR_BYTES {
                return Err(ReviewContractError::StringTooLong {
                    field: "error",
                    max: MAX_REVIEW_ERROR_BYTES,
                });
            }
        }
        Ok(())
    }
}

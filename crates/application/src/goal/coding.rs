//! Application-owned durable coding and verification records.

use contracts::{CodingJobReport, CodingJobStatus, VerificationReport};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct PersistedCodingJob {
    pub report: CodingJobReport,
    pub worktree_ref: PathBuf,
    pub diff_artifact_ref: PathBuf,
    pub diff_sha256: String,
    pub status: CodingJobStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub diff: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedVerificationReport {
    pub report: VerificationReport,
    pub created_at_ms: i64,
}

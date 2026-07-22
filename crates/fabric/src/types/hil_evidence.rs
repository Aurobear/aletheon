//! Signed HIL gate evidence — must pass before Production namespace becomes available.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HILEvidence {
    pub schema_version: u16,
    pub device_id: String,
    pub device_serial: String,
    pub software_commits: Vec<String>,
    pub manifest_digest: String,
    pub limits_digest: String,
    pub test_cases: Vec<String>,
    pub measured_stop_latency_ms: u64,
    pub result: HILResult,
    pub issued_unix_ms: i64,
    pub expiry_unix_ms: i64,
    pub signer_key_id: String,
    /// Canonical signature of the JSON report.
    pub signature: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HILResult {
    Passed,
    Failed,
    /// Inconclusive — must be retested.
    Inconclusive,
    /// Passed with noted deviations within tolerance.
    Conditional,
}

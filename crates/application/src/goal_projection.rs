//! Durable Goal projection data contract.

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GoalProjectionEvidence {
    pub attempt_ids: Vec<String>,
    pub artifact_ids: Vec<String>,
    pub source_commit: Option<String>,
    pub verification: Vec<String>,
}

//! Versioned repository context owned by the repository-inspection tool domain.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryContext {
    pub root: String,
    pub version: String,
    pub instructions: Vec<InstructionSource>,
    pub manifests: Vec<ManifestRef>,
    pub entry_files: Vec<RepositoryFileEvidence>,
    #[serde(default)]
    pub evidence_constraints: Vec<String>,
    #[serde(default)]
    pub exact_follow_up_paths: Vec<String>,
    pub missing_candidates: Vec<String>,
    pub vcs_state: VcsSnapshot,
    pub validation_commands: Vec<ValidationSpec>,
    pub protected_paths: Vec<String>,
    pub deployment_policy: Option<DeploymentPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryFileEvidence {
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub artifact_ref: String,
    pub preview: String,
    pub preview_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionSource {
    pub file: RepositoryFileEvidence,
    pub precedence: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestRef {
    pub file: RepositoryFileEvidence,
    pub kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VcsSnapshot {
    pub kind: Option<String>,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub dirty: Option<bool>,
    pub status_artifact_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationSpec {
    pub kind: String,
    pub command: String,
    pub source_path: String,
    pub source_line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentPolicy {
    pub source_path: String,
    pub requires_installed_runtime: bool,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub affected_path_prefixes: Vec<String>,
}

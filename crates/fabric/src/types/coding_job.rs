//! Contracts for isolated coding jobs and deterministic verification.

use crate::{AttemptId, GoalId};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CodingJobId(pub Uuid);

impl CodingJobId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CodingJobId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum CodingNetworkPolicy {
    #[default]
    Disabled,
    AllowHosts {
        hosts: Vec<String>,
    },
}

/// Canonical repository root plus lexical path policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceBoundary {
    repository_root: PathBuf,
    allowed_paths: Vec<PathBuf>,
    forbidden_paths: Vec<PathBuf>,
}

impl WorkspaceBoundary {
    pub fn new(
        repository_root: impl AsRef<Path>,
        allowed_paths: Vec<PathBuf>,
        forbidden_paths: Vec<PathBuf>,
    ) -> Result<Self, CodingJobValidationError> {
        let repository_root = repository_root
            .as_ref()
            .canonicalize()
            .map_err(|error| CodingJobValidationError::InvalidRepository(error.to_string()))?;
        if allowed_paths.is_empty() {
            return Err(CodingJobValidationError::EmptyAllowedScope);
        }
        for path in allowed_paths.iter().chain(&forbidden_paths) {
            validate_relative_lexical(path)?;
        }
        Ok(Self {
            repository_root,
            allowed_paths,
            forbidden_paths,
        })
    }

    pub fn repository_root(&self) -> &Path {
        &self.repository_root
    }

    pub fn allowed_paths(&self) -> &[PathBuf] {
        &self.allowed_paths
    }

    pub fn forbidden_paths(&self) -> &[PathBuf] {
        &self.forbidden_paths
    }

    /// Validate a repository-relative path, including existing symlink ancestors.
    pub fn validate_relative(&self, path: &Path) -> Result<PathBuf, CodingJobValidationError> {
        validate_relative_lexical(path)?;
        if self
            .forbidden_paths
            .iter()
            .any(|forbidden| path == forbidden || path.starts_with(forbidden))
        {
            return Err(CodingJobValidationError::ForbiddenPath(path.to_owned()));
        }
        if !self
            .allowed_paths
            .iter()
            .any(|allowed| path == allowed || path.starts_with(allowed))
        {
            return Err(CodingJobValidationError::OutsideAllowedScope(
                path.to_owned(),
            ));
        }

        let joined = self.repository_root.join(path);
        let existing = nearest_existing_ancestor(&joined)
            .ok_or_else(|| CodingJobValidationError::RepositoryEscape(path.to_owned()))?;
        let canonical = existing
            .canonicalize()
            .map_err(|_| CodingJobValidationError::RepositoryEscape(path.to_owned()))?;
        if !canonical.starts_with(&self.repository_root) {
            return Err(CodingJobValidationError::RepositoryEscape(path.to_owned()));
        }
        Ok(path.to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingJobSpec {
    pub job_id: CodingJobId,
    pub goal_id: GoalId,
    pub attempt_id: AttemptId,
    pub workspace: WorkspaceBoundary,
    pub base_commit: String,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub timeout_ms: u64,
    pub output_cap_bytes: usize,
    pub network_policy: CodingNetworkPolicy,
}

impl CodingJobSpec {
    pub fn validate(&self) -> Result<(), CodingJobValidationError> {
        if self.base_commit.trim().is_empty()
            || self.base_commit.starts_with('-')
            || self.base_commit.chars().any(char::is_whitespace)
        {
            return Err(CodingJobValidationError::InvalidBaseCommit);
        }
        if self.command.as_os_str().is_empty() {
            return Err(CodingJobValidationError::EmptyCommand);
        }
        if self.timeout_ms == 0 || self.output_cap_bytes == 0 {
            return Err(CodingJobValidationError::InvalidLimit);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangedFileKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: PathBuf,
    pub kind: ChangedFileKind,
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub content_sha256: String,
}

impl ChangedFile {
    pub fn new(
        boundary: &WorkspaceBoundary,
        path: PathBuf,
        kind: ChangedFileKind,
        before_bytes: u64,
        after_bytes: u64,
        content_sha256: String,
    ) -> Result<Self, CodingJobValidationError> {
        let path = boundary.validate_relative(&path)?;
        Ok(Self {
            path,
            kind,
            before_bytes,
            after_bytes,
            content_sha256,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingJobStatus {
    Running,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    Retained,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodingJobReport {
    pub job_id: CodingJobId,
    pub goal_id: GoalId,
    pub attempt_id: AttemptId,
    pub base_commit: String,
    pub status: CodingJobStatus,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub changed_files: Vec<ChangedFile>,
    pub diff_sha256: Option<String>,
    pub diff_artifact: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationSeverity {
    Advisory,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationCheck {
    pub name: String,
    pub severity: VerificationSeverity,
    pub passed: bool,
    pub timed_out: bool,
    pub cancelled: bool,
    pub summary: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationReport {
    pub job_id: CodingJobId,
    pub goal_id: GoalId,
    pub attempt_id: AttemptId,
    pub passed: bool,
    pub checks: Vec<VerificationCheck>,
    pub risk_summary: Vec<String>,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodingJobValidationError {
    InvalidRepository(String),
    EmptyAllowedScope,
    AbsolutePath(PathBuf),
    ParentTraversal(PathBuf),
    ForbiddenPath(PathBuf),
    OutsideAllowedScope(PathBuf),
    RepositoryEscape(PathBuf),
    InvalidBaseCommit,
    EmptyCommand,
    InvalidLimit,
}

impl fmt::Display for CodingJobValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid coding job: {self:?}")
    }
}

impl std::error::Error for CodingJobValidationError {}

fn validate_relative_lexical(path: &Path) -> Result<(), CodingJobValidationError> {
    if path.is_absolute() {
        return Err(CodingJobValidationError::AbsolutePath(path.to_owned()));
    }
    if path.as_os_str().is_empty()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(CodingJobValidationError::ParentTraversal(path.to_owned()));
    }
    Ok(())
}

fn nearest_existing_ancestor(path: &Path) -> Option<&Path> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.symlink_metadata().is_ok() {
            return Some(candidate);
        }
        current = candidate.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boundary(root: &Path) -> WorkspaceBoundary {
        WorkspaceBoundary::new(
            root,
            vec![PathBuf::from("src")],
            vec![PathBuf::from("src/secrets")],
        )
        .unwrap()
    }

    #[test]
    fn contracts_round_trip_through_serde() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        let spec = CodingJobSpec {
            job_id: CodingJobId::new(),
            goal_id: GoalId(1),
            attempt_id: AttemptId::new(),
            workspace: boundary(root.path()),
            base_commit: "deadbeef".into(),
            command: PathBuf::from("/trusted/pi"),
            args: vec!["--json".into()],
            timeout_ms: 1_000,
            output_cap_bytes: 8_192,
            network_policy: CodingNetworkPolicy::Disabled,
        };
        spec.validate().unwrap();
        let encoded = serde_json::to_string(&spec).unwrap();
        assert_eq!(
            serde_json::from_str::<CodingJobSpec>(&encoded).unwrap(),
            spec
        );
    }

    #[test]
    fn rejects_absolute_parent_and_empty_allowed_scope() {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(
            WorkspaceBoundary::new(root.path(), vec![], vec![]),
            Err(CodingJobValidationError::EmptyAllowedScope)
        ));
        let boundary = WorkspaceBoundary::new(root.path(), vec![".".into()], vec![]).unwrap();
        assert!(matches!(
            boundary.validate_relative(Path::new("/etc/passwd")),
            Err(CodingJobValidationError::AbsolutePath(_))
        ));
        assert!(matches!(
            boundary.validate_relative(Path::new("../escape")),
            Err(CodingJobValidationError::ParentTraversal(_))
        ));
    }

    #[test]
    fn forbidden_path_precedes_allowed_scope() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src/secrets")).unwrap();
        let boundary = boundary(root.path());
        assert!(matches!(
            boundary.validate_relative(Path::new("src/secrets/key")),
            Err(CodingJobValidationError::ForbiddenPath(_))
        ));
        assert!(matches!(
            boundary.validate_relative(Path::new("docs/readme")),
            Err(CodingJobValidationError::OutsideAllowedScope(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_and_repository_root_escape() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        symlink(outside.path(), root.path().join("src/link")).unwrap();
        let boundary = boundary(root.path());
        assert!(matches!(
            boundary.validate_relative(Path::new("src/link/file")),
            Err(CodingJobValidationError::RepositoryEscape(_))
        ));
    }

    #[test]
    fn changed_file_never_accepts_unchecked_absolute_path() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        let boundary = boundary(root.path());
        assert!(ChangedFile::new(
            &boundary,
            PathBuf::from("/tmp/escape"),
            ChangedFileKind::Added,
            0,
            1,
            "hash".into(),
        )
        .is_err());
    }
}

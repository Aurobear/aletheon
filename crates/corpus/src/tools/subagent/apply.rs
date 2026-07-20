//! Fail-closed application of a previously verified and approved git patch.

use super::command::{CommandOutput, CommandRequest, CommandRunner};
use fabric::{ApprovalId, ApprovalStatus};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const OUTPUT_CAP: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplySpec {
    pub repository_root: PathBuf,
    pub expected_head: String,
    pub diff_artifact: PathBuf,
    pub diff_sha256: String,
    pub verification_artifact: PathBuf,
    pub verification_sha256: String,
    pub allowed_paths: Vec<PathBuf>,
    pub approval_id: ApprovalId,
    pub subject_hash: String,
    pub timeout: Duration,
    pub dry_run: bool,
}

/// Immutable approval facts returned by the executive's read-only adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyAuthorization {
    pub approval_id: ApprovalId,
    pub status: ApprovalStatus,
    pub subject_hash: String,
    pub expected_head: String,
    pub diff_sha256: String,
    pub verification_sha256: String,
    pub allowed_paths: Vec<PathBuf>,
}

pub trait ApplyAuthorizer: Send + Sync {
    fn authorization(&self, approval_id: ApprovalId) -> Result<Option<ApplyAuthorization>, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub approval_id: ApprovalId,
    pub head: String,
    pub diff_sha256: String,
    pub changed_paths: Vec<PathBuf>,
    pub dry_run: bool,
}

#[derive(Debug)]
pub enum ApplyError {
    InvalidSpec(String),
    Artifact(String),
    Unauthorized(String),
    StaleHead { expected: String, actual: String },
    Scope(String),
    CheckFailed(String),
    ApplyFailed(String),
    Cancelled,
    TimedOut,
    Command(String),
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSpec(value) => write!(f, "invalid apply specification: {value}"),
            Self::Artifact(value) => write!(f, "artifact validation failed: {value}"),
            Self::Unauthorized(value) => write!(f, "approval denied: {value}"),
            Self::StaleHead { expected, actual } => {
                write!(
                    f,
                    "repository HEAD changed: expected {expected}, got {actual}"
                )
            }
            Self::Scope(value) => write!(f, "path scope rejected: {value}"),
            Self::CheckFailed(value) => write!(f, "git apply check failed: {value}"),
            Self::ApplyFailed(value) => write!(f, "git apply failed: {value}"),
            Self::Cancelled => write!(f, "apply cancelled"),
            Self::TimedOut => write!(f, "apply timed out"),
            Self::Command(value) => write!(f, "apply command failed: {value}"),
        }
    }
}

impl std::error::Error for ApplyError {}

pub struct ControlledApply {
    authorizer: Arc<dyn ApplyAuthorizer>,
    runner: CommandRunner,
    git_program: PathBuf,
}

#[derive(Debug)]
struct PathSnapshot {
    path: PathBuf,
    worktree: Option<(Vec<u8>, u32)>,
    index: Option<(String, String)>,
}

impl ControlledApply {
    pub fn new(authorizer: Arc<dyn ApplyAuthorizer>) -> Result<Self, ApplyError> {
        let git_program = which::which("git")
            .map_err(|error| ApplyError::Command(format!("locating git: {error}")))?
            .canonicalize()
            .map_err(|error| ApplyError::Command(format!("canonicalizing git: {error}")))?;
        Ok(Self {
            authorizer,
            runner: CommandRunner,
            git_program,
        })
    }

    pub async fn apply(
        &self,
        spec: ApplySpec,
        cancel: CancellationToken,
    ) -> Result<ApplyOutcome, ApplyError> {
        validate_spec(&spec)?;
        if cancel.is_cancelled() {
            return Err(ApplyError::Cancelled);
        }
        let repository = spec
            .repository_root
            .canonicalize()
            .map_err(|error| ApplyError::InvalidSpec(format!("repository root: {error}")))?;
        if !repository.join(".git").exists() {
            return Err(ApplyError::InvalidSpec(
                "repository root is not a git worktree".into(),
            ));
        }

        let diff_artifact = spec
            .diff_artifact
            .canonicalize()
            .map_err(|error| ApplyError::Artifact(format!("canonicalizing diff: {error}")))?;
        let verification_artifact = spec.verification_artifact.canonicalize().map_err(|error| {
            ApplyError::Artifact(format!("canonicalizing verification report: {error}"))
        })?;
        let diff = tokio::fs::read(&diff_artifact)
            .await
            .map_err(|error| ApplyError::Artifact(format!("reading diff: {error}")))?;
        let verification = tokio::fs::read(&verification_artifact)
            .await
            .map_err(|error| {
                ApplyError::Artifact(format!("reading verification report: {error}"))
            })?;
        verify_hash("diff", &diff, &spec.diff_sha256)?;
        verify_hash(
            "verification report",
            &verification,
            &spec.verification_sha256,
        )?;

        let authorization = self
            .authorizer
            .authorization(spec.approval_id)
            .map_err(ApplyError::Unauthorized)?
            .ok_or_else(|| ApplyError::Unauthorized("approval not found".into()))?;
        verify_authorization(&spec, &authorization)?;

        let head = self
            .git(
                &repository,
                vec!["rev-parse".into(), "HEAD".into()],
                spec.timeout,
                cancel.clone(),
            )
            .await?;
        ensure_completed(&head)?;
        if head.exit_code != Some(0) {
            return Err(ApplyError::Command(bound_message(&head.stderr)));
        }
        let actual_head = head.stdout.trim().to_owned();
        if actual_head != spec.expected_head {
            return Err(ApplyError::StaleHead {
                expected: spec.expected_head,
                actual: actual_head,
            });
        }

        reject_symlink_patch(&diff)?;
        let paths_output = self
            .git(
                &repository,
                vec![
                    "apply".into(),
                    "--numstat".into(),
                    "-z".into(),
                    "--".into(),
                    diff_artifact.to_string_lossy().into_owned(),
                ],
                spec.timeout,
                cancel.clone(),
            )
            .await?;
        ensure_completed(&paths_output)?;
        if paths_output.exit_code != Some(0) {
            return Err(ApplyError::Scope(bound_message(&paths_output.stderr)));
        }
        let changed_paths = parse_numstat(&paths_output.stdout_bytes)?;
        if changed_paths.is_empty() {
            return Err(ApplyError::InvalidSpec(
                "diff contains no changed paths".into(),
            ));
        }
        validate_scope(&repository, &changed_paths, &spec.allowed_paths)?;

        let check = self
            .git(
                &repository,
                vec![
                    "apply".into(),
                    "--check".into(),
                    "--index".into(),
                    "--".into(),
                    diff_artifact.to_string_lossy().into_owned(),
                ],
                spec.timeout,
                cancel.clone(),
            )
            .await?;
        ensure_completed(&check)?;
        if check.exit_code != Some(0) {
            return Err(ApplyError::CheckFailed(bound_message(&check.stderr)));
        }

        if !spec.dry_run {
            // Snapshot only approved, affected paths. If the process is killed
            // between index/worktree writes, rollback never touches unrelated
            // user files and never uses reset/checkout.
            let snapshot = self
                .snapshot_paths(&repository, &changed_paths, spec.timeout, cancel.clone())
                .await?;
            let applied = self
                .git(
                    &repository,
                    vec![
                        "apply".into(),
                        "--index".into(),
                        "--".into(),
                        diff_artifact.to_string_lossy().into_owned(),
                    ],
                    spec.timeout,
                    cancel,
                )
                .await;
            let failure = match applied {
                Err(error) => Some(error),
                Ok(output) if output.cancelled => Some(ApplyError::Cancelled),
                Ok(output) if output.timed_out => Some(ApplyError::TimedOut),
                Ok(output) if output.exit_code != Some(0) => {
                    Some(ApplyError::ApplyFailed(bound_message(&output.stderr)))
                }
                Ok(_) => None,
            };
            if let Some(failure) = failure {
                self.restore_paths(&repository, &snapshot, spec.timeout)
                    .await
                    .map_err(|rollback| {
                        ApplyError::ApplyFailed(format!("{failure}; rollback failed: {rollback}"))
                    })?;
                return Err(failure);
            }
        }

        Ok(ApplyOutcome {
            approval_id: spec.approval_id,
            head: actual_head,
            diff_sha256: spec.diff_sha256,
            changed_paths,
            dry_run: spec.dry_run,
        })
    }

    async fn git(
        &self,
        repository: &Path,
        args: Vec<String>,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> Result<CommandOutput, ApplyError> {
        self.runner
            .run(
                CommandRequest {
                    program: self.git_program.clone(),
                    args,
                    working_dir: repository.to_path_buf(),
                    environment: BTreeMap::from([
                        ("PATH".into(), "/usr/bin:/bin".into()),
                        ("LC_ALL".into(), "C".into()),
                        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
                    ]),
                    stdin: None,
                    timeout,
                    stream_cap_bytes: OUTPUT_CAP,
                },
                cancel,
            )
            .await
            .map_err(|error| ApplyError::Command(error.to_string()))
    }

    async fn snapshot_paths(
        &self,
        repository: &Path,
        paths: &[PathBuf],
        timeout: Duration,
        cancel: CancellationToken,
    ) -> Result<Vec<PathSnapshot>, ApplyError> {
        let mut snapshots = Vec::with_capacity(paths.len());
        for path in paths {
            let absolute = repository.join(path);
            let worktree = match tokio::fs::read(&absolute).await {
                Ok(bytes) => {
                    let metadata = tokio::fs::metadata(&absolute)
                        .await
                        .map_err(|error| ApplyError::Command(error.to_string()))?;
                    #[cfg(unix)]
                    let mode = std::os::unix::fs::PermissionsExt::mode(&metadata.permissions());
                    #[cfg(not(unix))]
                    let mode = 0;
                    Some((bytes, mode))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(ApplyError::Command(error.to_string())),
            };
            let staged = self
                .git(
                    repository,
                    vec![
                        "ls-files".into(),
                        "--stage".into(),
                        "--".into(),
                        path.to_string_lossy().into_owned(),
                    ],
                    timeout,
                    cancel.clone(),
                )
                .await?;
            ensure_completed(&staged)?;
            if staged.exit_code != Some(0) {
                return Err(ApplyError::Command(bound_message(&staged.stderr)));
            }
            let index = parse_index_entry(&staged.stdout, path)?;
            snapshots.push(PathSnapshot {
                path: path.clone(),
                worktree,
                index,
            });
        }
        Ok(snapshots)
    }

    async fn restore_paths(
        &self,
        repository: &Path,
        snapshots: &[PathSnapshot],
        timeout: Duration,
    ) -> Result<(), ApplyError> {
        for snapshot in snapshots {
            let absolute = repository.join(&snapshot.path);
            match &snapshot.worktree {
                Some((bytes, mode)) => {
                    if let Some(parent) = absolute.parent() {
                        tokio::fs::create_dir_all(parent)
                            .await
                            .map_err(|error| ApplyError::Command(error.to_string()))?;
                    }
                    tokio::fs::write(&absolute, bytes)
                        .await
                        .map_err(|error| ApplyError::Command(error.to_string()))?;
                    #[cfg(unix)]
                    tokio::fs::set_permissions(
                        &absolute,
                        std::os::unix::fs::PermissionsExt::from_mode(*mode),
                    )
                    .await
                    .map_err(|error| ApplyError::Command(error.to_string()))?;
                }
                None => match tokio::fs::remove_file(&absolute).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(ApplyError::Command(error.to_string())),
                },
            }
            let args = match &snapshot.index {
                Some((mode, object)) => vec![
                    "update-index".into(),
                    "--add".into(),
                    "--cacheinfo".into(),
                    format!("{mode},{object},{}", snapshot.path.to_string_lossy()),
                ],
                None => vec![
                    "update-index".into(),
                    "--force-remove".into(),
                    "--".into(),
                    snapshot.path.to_string_lossy().into_owned(),
                ],
            };
            let restored = self
                .git(repository, args, timeout, CancellationToken::new())
                .await?;
            if restored.exit_code != Some(0) {
                return Err(ApplyError::Command(bound_message(&restored.stderr)));
            }
        }
        Ok(())
    }
}

fn validate_spec(spec: &ApplySpec) -> Result<(), ApplyError> {
    if spec.timeout.is_zero()
        || spec.expected_head.is_empty()
        || spec.diff_sha256.is_empty()
        || spec.verification_sha256.is_empty()
        || spec.subject_hash.is_empty()
        || spec.allowed_paths.is_empty()
    {
        return Err(ApplyError::InvalidSpec("required field is empty".into()));
    }
    for path in &spec.allowed_paths {
        validate_relative(path)?;
    }
    Ok(())
}

fn verify_hash(name: &str, bytes: &[u8], expected: &str) -> Result<(), ApplyError> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        return Err(ApplyError::Artifact(format!("{name} hash mismatch")));
    }
    Ok(())
}

fn verify_authorization(
    spec: &ApplySpec,
    authorization: &ApplyAuthorization,
) -> Result<(), ApplyError> {
    if authorization.status != ApprovalStatus::Approved {
        return Err(ApplyError::Unauthorized(format!(
            "approval status is {:?}",
            authorization.status
        )));
    }
    let approved_scope: BTreeSet<_> = authorization.allowed_paths.iter().collect();
    let requested_scope: BTreeSet<_> = spec.allowed_paths.iter().collect();
    if authorization.approval_id != spec.approval_id
        || authorization.subject_hash != spec.subject_hash
        || authorization.expected_head != spec.expected_head
        || authorization.diff_sha256 != spec.diff_sha256
        || authorization.verification_sha256 != spec.verification_sha256
        || approved_scope != requested_scope
    {
        return Err(ApplyError::Unauthorized(
            "apply specification does not match approved subject".into(),
        ));
    }
    Ok(())
}

fn validate_relative(path: &Path) -> Result<(), ApplyError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ApplyError::Scope(format!("unsafe path {}", path.display())));
    }
    Ok(())
}

fn validate_scope(
    repository: &Path,
    changed: &[PathBuf],
    allowed: &[PathBuf],
) -> Result<(), ApplyError> {
    for path in changed {
        validate_relative(path)?;
        if !allowed
            .iter()
            .any(|root| path == root || path.starts_with(root))
        {
            return Err(ApplyError::Scope(format!(
                "{} is outside approved scope",
                path.display()
            )));
        }
        let mut cursor = repository.to_path_buf();
        for component in path.components() {
            cursor.push(component.as_os_str());
            if let Ok(metadata) = std::fs::symlink_metadata(&cursor) {
                if metadata.file_type().is_symlink() {
                    return Err(ApplyError::Scope(format!(
                        "{} traverses a symlink",
                        path.display()
                    )));
                }
            }
        }
    }
    Ok(())
}

fn reject_symlink_patch(diff: &[u8]) -> Result<(), ApplyError> {
    let text = String::from_utf8_lossy(diff);
    if text.lines().any(|line| {
        matches!(
            line,
            "new file mode 120000" | "new mode 120000" | "old mode 120000"
        )
    }) {
        return Err(ApplyError::Scope(
            "patch creates or changes a symlink".into(),
        ));
    }
    Ok(())
}

fn parse_numstat(bytes: &[u8]) -> Result<Vec<PathBuf>, ApplyError> {
    let fields: Vec<&[u8]> = bytes
        .split(|byte| *byte == 0)
        .filter(|v| !v.is_empty())
        .collect();
    let mut paths = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let record = fields[index];
        let tabs: Vec<_> = record.splitn(3, |byte| *byte == b'\t').collect();
        if tabs.len() != 3 {
            return Err(ApplyError::Scope("malformed git numstat output".into()));
        }
        if tabs[2].is_empty() {
            // With -z, renames/copies encode old and new paths as two following fields.
            if index + 2 >= fields.len() {
                return Err(ApplyError::Scope(
                    "malformed rename in numstat output".into(),
                ));
            }
            paths.push(path_from_git(fields[index + 1])?);
            paths.push(path_from_git(fields[index + 2])?);
            index += 3;
        } else {
            paths.push(path_from_git(tabs[2])?);
            index += 1;
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn parse_index_entry(
    output: &str,
    expected_path: &Path,
) -> Result<Option<(String, String)>, ApplyError> {
    if output.is_empty() {
        return Ok(None);
    }
    let lines: Vec<_> = output.lines().collect();
    if lines.len() != 1 {
        return Err(ApplyError::InvalidSpec(format!(
            "{} has unmerged index stages",
            expected_path.display()
        )));
    }
    let (metadata, path) = lines[0]
        .split_once('\t')
        .ok_or_else(|| ApplyError::Command("malformed ls-files output".into()))?;
    if Path::new(path) != expected_path {
        return Err(ApplyError::Command(
            "ls-files returned an unexpected path".into(),
        ));
    }
    let fields: Vec<_> = metadata.split_whitespace().collect();
    if fields.len() != 3 || fields[2] != "0" {
        return Err(ApplyError::InvalidSpec("unmerged index entry".into()));
    }
    Ok(Some((fields[0].into(), fields[1].into())))
}

fn path_from_git(bytes: &[u8]) -> Result<PathBuf, ApplyError> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| ApplyError::Scope("non-UTF-8 patch path is unsupported".into()))?;
    let path = PathBuf::from(value);
    validate_relative(&path)?;
    Ok(path)
}

fn ensure_completed(output: &CommandOutput) -> Result<(), ApplyError> {
    if output.cancelled {
        Err(ApplyError::Cancelled)
    } else if output.timed_out {
        Err(ApplyError::TimedOut)
    } else {
        Ok(())
    }
}

fn bound_message(message: &str) -> String {
    message.chars().take(2048).collect()
}

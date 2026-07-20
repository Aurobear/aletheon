//! Ownership-checked temporary git worktrees for coding jobs.

use super::command::{CommandOutput, CommandRequest, CommandRunner};
use anyhow::{bail, Context, Result};
use fabric::{ChangedFile, ChangedFileKind, Clock, CodingJobId, WorkspaceBoundary};
use kernel::chronos::SystemClock;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const GIT_OUTPUT_CAP: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct WorktreeManagerConfig {
    pub base_dir: PathBuf,
    pub failed_ttl: Duration,
    pub failed_cap: usize,
    pub disk_budget_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeLease {
    pub job_id: CodingJobId,
    pub path: PathBuf,
    pub base_commit: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeSnapshot {
    pub porcelain_v2: Vec<u8>,
    pub diff: Vec<u8>,
    pub diff_truncated: bool,
    pub diff_sha256: String,
    pub changed_files: Vec<ChangedFile>,
}

#[derive(Debug, Clone)]
struct RetainedWorktree {
    lease: WorktreeLease,
    retained_at: i64,
}

pub struct WorktreeManager {
    config: WorktreeManagerConfig,
    base_dir: PathBuf,
    git_program: PathBuf,
    runner: CommandRunner,
    clock: Arc<dyn Clock>,
    retained: Mutex<HashMap<CodingJobId, RetainedWorktree>>,
}

impl WorktreeManager {
    pub fn new(config: WorktreeManagerConfig) -> Result<Self> {
        Self::with_clock(config, Arc::new(SystemClock::new()))
    }

    pub fn with_clock(config: WorktreeManagerConfig, clock: Arc<dyn Clock>) -> Result<Self> {
        if config.failed_cap == 0 || config.disk_budget_bytes == 0 {
            bail!("worktree failed cap and disk budget must be positive");
        }
        std::fs::create_dir_all(&config.base_dir).context("creating managed worktree base")?;
        let base_dir = config
            .base_dir
            .canonicalize()
            .context("canonicalizing managed worktree base")?;
        let git_program = which::which("git")
            .context("locating git executable")?
            .canonicalize()
            .context("canonicalizing git executable")?;
        Ok(Self {
            config,
            base_dir,
            git_program,
            runner: CommandRunner,
            clock,
            retained: Mutex::new(HashMap::new()),
        })
    }

    pub fn managed_path(&self, job_id: CodingJobId) -> PathBuf {
        self.base_dir.join(format!("job-{}", job_id.0))
    }

    pub async fn create(
        &self,
        job_id: CodingJobId,
        repository: &Path,
        base_commit: &str,
        cancel: CancellationToken,
    ) -> Result<WorktreeLease> {
        let repository = repository
            .canonicalize()
            .context("canonicalizing repository root")?;
        if disk_usage(&self.base_dir)? >= self.config.disk_budget_bytes {
            bail!("managed worktree disk budget exhausted");
        }
        let path = self.managed_path(job_id);
        verify_generated_path(&self.base_dir, &path, job_id, false)?;
        if path.exists() && std::fs::read_dir(&path)?.next().is_some() {
            bail!("generated worktree path already exists and is nonempty");
        }

        let verified = self
            .git(
                &repository,
                vec![
                    "rev-parse".into(),
                    "--verify".into(),
                    format!("{base_commit}^{{commit}}"),
                ],
                cancel.clone(),
            )
            .await?;
        ensure_success(&verified, "verifying base commit")?;
        let immutable_base = verified.stdout.trim().to_owned();
        if immutable_base.is_empty() {
            bail!("git returned an empty base commit");
        }

        let output = self
            .git(
                &repository,
                vec![
                    "worktree".into(),
                    "add".into(),
                    "--detach".into(),
                    path.to_string_lossy().into_owned(),
                    immutable_base.clone(),
                ],
                cancel,
            )
            .await?;
        ensure_success(&output, "creating detached worktree")?;
        let canonical_path = path
            .canonicalize()
            .context("canonicalizing created worktree")?;
        verify_generated_path(&self.base_dir, &canonical_path, job_id, true)?;
        Ok(WorktreeLease {
            job_id,
            path: canonical_path,
            base_commit: immutable_base,
            created_at: self.clock.wall_now().0,
        })
    }

    pub async fn collect(
        &self,
        lease: &WorktreeLease,
        boundary: &WorkspaceBoundary,
        cancel: CancellationToken,
    ) -> Result<WorktreeSnapshot> {
        self.verify_lease(lease)?;
        let status = self
            .git(
                &lease.path,
                vec![
                    "status".into(),
                    "--porcelain=v2".into(),
                    "-z".into(),
                    "--untracked-files=all".into(),
                ],
                cancel.clone(),
            )
            .await?;
        ensure_success(&status, "collecting worktree status")?;
        let mut changed = parse_status(&status.stdout)?;
        changed.sort_by(|left, right| left.0.cmp(&right.0));
        changed.dedup_by(|left, right| left.0 == right.0);

        let mut changed_files = Vec::with_capacity(changed.len());
        for (path, kind) in changed {
            let relative = PathBuf::from(&path);
            boundary.validate_relative(&relative)?;
            let after = lease.path.join(&relative);
            // A post-execution symlink may not redirect hashing outside the worktree.
            validate_worktree_path(&lease.path, &after, &relative)?;
            let after_bytes = if after.is_file() {
                after.metadata()?.len()
            } else {
                0
            };
            let before_bytes = self.base_file_size(lease, &path, cancel.clone()).await?;
            let content_sha256 = if after.is_file() {
                hash_file(&after)?
            } else {
                hex_sha256(&[])
            };
            changed_files.push(ChangedFile::new(
                boundary,
                relative,
                kind,
                before_bytes,
                after_bytes,
                content_sha256,
            )?);
        }

        let diff = self
            .git(
                &lease.path,
                vec![
                    "diff".into(),
                    "--binary".into(),
                    "--no-ext-diff".into(),
                    lease.base_commit.clone(),
                ],
                cancel,
            )
            .await?;
        ensure_success(&diff, "collecting worktree diff")?;
        Ok(WorktreeSnapshot {
            porcelain_v2: status.stdout_bytes,
            diff_sha256: hex_sha256(&diff.stdout_bytes),
            diff: diff.stdout_bytes,
            diff_truncated: diff.stdout_truncated,
            changed_files,
        })
    }

    pub async fn finish(
        &self,
        lease: WorktreeLease,
        succeeded: bool,
        cancel: CancellationToken,
    ) -> Result<()> {
        self.verify_lease(&lease)?;
        if succeeded {
            self.remove(&lease, cancel).await
        } else {
            self.retained.lock().unwrap().insert(
                lease.job_id,
                RetainedWorktree {
                    lease,
                    retained_at: self.clock.wall_now().0,
                },
            );
            self.prune(cancel).await?;
            Ok(())
        }
    }

    pub async fn remove(&self, lease: &WorktreeLease, cancel: CancellationToken) -> Result<()> {
        self.verify_lease(lease)?;
        let common = self
            .git(
                &lease.path,
                vec!["rev-parse".into(), "--git-common-dir".into()],
                cancel.clone(),
            )
            .await?;
        ensure_success(&common, "resolving worktree repository")?;
        let common_path = if Path::new(common.stdout.trim()).is_absolute() {
            PathBuf::from(common.stdout.trim())
        } else {
            lease.path.join(common.stdout.trim())
        }
        .canonicalize()
        .context("canonicalizing git common directory")?;
        let repository = common_path
            .parent()
            .map(Path::to_owned)
            .context("git common directory has no repository parent")?;
        let output = self
            .git(
                &repository,
                vec![
                    "worktree".into(),
                    "remove".into(),
                    "--force".into(),
                    lease.path.to_string_lossy().into_owned(),
                ],
                cancel.clone(),
            )
            .await?;
        ensure_success(&output, "removing managed worktree")?;
        self.retained.lock().unwrap().remove(&lease.job_id);
        let prune = self
            .git(&repository, vec!["worktree".into(), "prune".into()], cancel)
            .await?;
        ensure_success(&prune, "pruning worktree metadata")
    }

    pub async fn prune(&self, cancel: CancellationToken) -> Result<usize> {
        let now = self.clock.wall_now().0;
        let ttl_ms = self.config.failed_ttl.as_millis().min(i64::MAX as u128) as i64;
        let mut retained: Vec<_> = self.retained.lock().unwrap().values().cloned().collect();
        retained.sort_by_key(|entry| (entry.retained_at, entry.lease.job_id.0));
        let overflow = retained.len().saturating_sub(self.config.failed_cap);
        let mut remove = Vec::new();
        for (index, entry) in retained.into_iter().enumerate() {
            if now.saturating_sub(entry.retained_at) >= ttl_ms || index < overflow {
                remove.push(entry.lease);
            }
        }
        let count = remove.len();
        for lease in remove {
            self.remove(&lease, cancel.clone()).await?;
        }
        Ok(count)
    }

    pub fn retained_leases(&self) -> Vec<WorktreeLease> {
        let mut leases: Vec<_> = self
            .retained
            .lock()
            .unwrap()
            .values()
            .map(|entry| entry.lease.clone())
            .collect();
        leases.sort_by_key(|lease| lease.created_at);
        leases
    }

    fn verify_lease(&self, lease: &WorktreeLease) -> Result<()> {
        verify_generated_path(&self.base_dir, &lease.path, lease.job_id, true)
    }

    async fn base_file_size(
        &self,
        lease: &WorktreeLease,
        relative: &str,
        cancel: CancellationToken,
    ) -> Result<u64> {
        let output = self
            .git(
                &lease.path,
                vec![
                    "cat-file".into(),
                    "-s".into(),
                    format!("{}:{relative}", lease.base_commit),
                ],
                cancel,
            )
            .await?;
        if output.exit_code != Some(0) {
            return Ok(0);
        }
        output
            .stdout
            .trim()
            .parse()
            .context("parsing base file byte count")
    }

    async fn git(
        &self,
        directory: &Path,
        args: Vec<String>,
        cancel: CancellationToken,
    ) -> Result<CommandOutput> {
        self.runner
            .run(
                CommandRequest {
                    program: self.git_program.clone(),
                    args: std::iter::once("-C".into())
                        .chain(std::iter::once(directory.to_string_lossy().into_owned()))
                        .chain(args)
                        .collect(),
                    working_dir: directory.to_owned(),
                    environment: BTreeMap::new(),
                    stdin: None,
                    timeout: Duration::from_secs(30),
                    stream_cap_bytes: GIT_OUTPUT_CAP,
                },
                cancel,
            )
            .await
            .map_err(Into::into)
    }
}

fn ensure_success(output: &CommandOutput, operation: &str) -> Result<()> {
    if output.timed_out {
        bail!("{operation} timed out");
    }
    if output.cancelled {
        bail!("{operation} cancelled");
    }
    if output.exit_code != Some(0) {
        bail!(
            "{operation} failed with {:?}: {}",
            output.exit_code,
            output.stderr.trim()
        );
    }
    Ok(())
}

fn verify_generated_path(
    base: &Path,
    path: &Path,
    job_id: CodingJobId,
    require_existing: bool,
) -> Result<()> {
    let expected_name = format!("job-{}", job_id.0);
    if path.file_name().and_then(|name| name.to_str()) != Some(expected_name.as_str()) {
        bail!("worktree path is not owned by the supplied job ID");
    }
    let owned_path = if require_existing {
        path.canonicalize()
            .context("canonicalizing managed worktree path")?
    } else if path.exists() {
        path.canonicalize()
            .context("canonicalizing candidate worktree path")?
    } else {
        path.to_owned()
    };
    if !owned_path.starts_with(base) || owned_path.parent() != Some(base) {
        bail!("worktree path escapes managed base");
    }
    Ok(())
}

fn validate_worktree_path(root: &Path, path: &Path, relative: &Path) -> Result<()> {
    let existing = nearest_existing(path).context("changed path has no existing ancestor")?;
    let canonical = existing
        .canonicalize()
        .context("canonicalizing changed path")?;
    if !canonical.starts_with(root) {
        bail!("changed path escapes worktree through symlink: {relative:?}");
    }
    Ok(())
}

fn nearest_existing(path: &Path) -> Option<&Path> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.symlink_metadata().is_ok() {
            return Some(candidate);
        }
        current = candidate.parent();
    }
    None
}

fn parse_status(status: &str) -> Result<Vec<(String, ChangedFileKind)>> {
    let mut changed = Vec::new();
    let mut records = status.split('\0').filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        if let Some(path) = record.strip_prefix("? ") {
            changed.push((path.into(), ChangedFileKind::Added));
        } else if record.starts_with("1 ") {
            let fields: Vec<&str> = record.splitn(9, ' ').collect();
            if fields.len() != 9 {
                bail!("malformed porcelain v2 ordinary record");
            }
            changed.push((fields[8].into(), kind_from_xy(fields[1])));
        } else if record.starts_with("2 ") {
            let fields: Vec<&str> = record.splitn(10, ' ').collect();
            if fields.len() != 10 {
                bail!("malformed porcelain v2 rename record");
            }
            changed.push((fields[9].into(), kind_from_xy(fields[1])));
            let _original_path = records.next();
        } else if record.starts_with("u ") {
            bail!("unmerged paths are not valid coding output");
        }
    }
    Ok(changed)
}

fn kind_from_xy(xy: &str) -> ChangedFileKind {
    if xy.contains('D') {
        ChangedFileKind::Deleted
    } else if xy.contains('A') {
        ChangedFileKind::Added
    } else {
        ChangedFileKind::Modified
    }
}

fn disk_usage(root: &Path) -> Result<u64> {
    let mut total = 0_u64;
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_file() {
            total = total.saturating_add(entry.metadata()?.len());
        }
    }
    Ok(total)
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(hex_sha256(&bytes))
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::chronos::TestClock;

    async fn git(repo: &Path, args: &[&str]) -> String {
        let output = tokio::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().into()
    }

    async fn fixture() -> (tempfile::TempDir, String, String) {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-q"]).await;
        git(repo.path(), &["config", "user.email", "test@example.com"]).await;
        git(repo.path(), &["config", "user.name", "Test"]).await;
        std::fs::create_dir(repo.path().join("src")).unwrap();
        std::fs::write(repo.path().join("src/lib.rs"), "pub fn old() {}\n").unwrap();
        git(repo.path(), &["add", "src/lib.rs"]).await;
        git(repo.path(), &["commit", "-qm", "base"]).await;
        let first = git(repo.path(), &["rev-parse", "HEAD"]).await;
        std::fs::write(repo.path().join("src/lib.rs"), "pub fn main() {}\n").unwrap();
        git(repo.path(), &["commit", "-qam", "main"]).await;
        let second = git(repo.path(), &["rev-parse", "HEAD"]).await;
        (repo, first, second)
    }

    fn manager(
        base: &Path,
        clock: Arc<TestClock>,
        cap: usize,
        ttl: Duration,
        disk: u64,
    ) -> WorktreeManager {
        WorktreeManager::with_clock(
            WorktreeManagerConfig {
                base_dir: base.into(),
                failed_ttl: ttl,
                failed_cap: cap,
                disk_budget_bytes: disk,
            },
            clock,
        )
        .unwrap()
    }

    fn boundary(path: &Path) -> WorkspaceBoundary {
        WorkspaceBoundary::new(path, vec!["src".into()], vec!["src/forbidden".into()]).unwrap()
    }

    #[tokio::test]
    async fn detached_base_is_pinned_and_main_worktree_stays_immutable() {
        let (repo, first, main) = fixture().await;
        let base = tempfile::tempdir().unwrap();
        let manager = manager(
            base.path(),
            Arc::new(TestClock::new(1, 0)),
            3,
            Duration::from_secs(60),
            10_000_000,
        );
        let lease = manager
            .create(
                CodingJobId::new(),
                repo.path(),
                &first,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(lease.base_commit, first);
        assert!(std::fs::read_to_string(lease.path.join("src/lib.rs"))
            .unwrap()
            .contains("old"));
        std::fs::write(lease.path.join("src/lib.rs"), "pub fn changed() {}\n").unwrap();
        assert_eq!(git(repo.path(), &["rev-parse", "HEAD"]).await, main);
        assert!(git(repo.path(), &["status", "--porcelain"])
            .await
            .is_empty());
        manager
            .remove(&lease, CancellationToken::new())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn status_diff_and_hashes_are_correct_and_sorted() {
        let (repo, _first, base_commit) = fixture().await;
        let base = tempfile::tempdir().unwrap();
        let manager = manager(
            base.path(),
            Arc::new(TestClock::new(1, 0)),
            3,
            Duration::from_secs(60),
            10_000_000,
        );
        let lease = manager
            .create(
                CodingJobId::new(),
                repo.path(),
                &base_commit,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        std::fs::write(lease.path.join("src/lib.rs"), "pub fn changed() {}\n").unwrap();
        std::fs::write(lease.path.join("src/new.rs"), "new\n").unwrap();
        let snapshot = manager
            .collect(&lease, &boundary(&lease.path), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            snapshot
                .changed_files
                .iter()
                .map(|file| file.path.clone())
                .collect::<Vec<_>>(),
            vec![PathBuf::from("src/lib.rs"), PathBuf::from("src/new.rs")]
        );
        assert!(snapshot
            .diff
            .windows(b"changed".len())
            .any(|w| w == b"changed"));
        assert_eq!(snapshot.diff_sha256, hex_sha256(&snapshot.diff));
        assert!(snapshot
            .changed_files
            .iter()
            .all(|file| file.content_sha256.len() == 64));
        manager
            .remove(&lease, CancellationToken::new())
            .await
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn changed_symlink_escape_and_forged_lease_are_rejected() {
        use std::os::unix::fs::symlink;
        let (repo, _first, commit) = fixture().await;
        let base = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let manager = manager(
            base.path(),
            Arc::new(TestClock::new(1, 0)),
            3,
            Duration::from_secs(60),
            10_000_000,
        );
        let lease = manager
            .create(
                CodingJobId::new(),
                repo.path(),
                &commit,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        symlink(outside.path(), lease.path.join("src/link")).unwrap();
        assert!(manager
            .collect(&lease, &boundary(&lease.path), CancellationToken::new())
            .await
            .is_err());
        let mut forged = lease.clone();
        forged.path = outside.path().into();
        assert!(manager
            .remove(&forged, CancellationToken::new())
            .await
            .is_err());
        manager
            .remove(&lease, CancellationToken::new())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn success_cleanup_and_failed_retention_are_explicit() {
        let (repo, _first, commit) = fixture().await;
        let base = tempfile::tempdir().unwrap();
        let manager = manager(
            base.path(),
            Arc::new(TestClock::new(1, 0)),
            3,
            Duration::from_secs(60),
            10_000_000,
        );
        let success = manager
            .create(
                CodingJobId::new(),
                repo.path(),
                &commit,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let success_path = success.path.clone();
        manager
            .finish(success, true, CancellationToken::new())
            .await
            .unwrap();
        assert!(!success_path.exists());
        let failed = manager
            .create(
                CodingJobId::new(),
                repo.path(),
                &commit,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let failed_path = failed.path.clone();
        manager
            .finish(failed, false, CancellationToken::new())
            .await
            .unwrap();
        assert!(failed_path.exists());
        assert_eq!(manager.retained_leases().len(), 1);
    }

    #[tokio::test]
    async fn ttl_and_oldest_first_cap_prune_retained_failures() {
        let (repo, _first, commit) = fixture().await;
        let base = tempfile::tempdir().unwrap();
        let clock = Arc::new(TestClock::new(1_000, 0));
        let manager = manager(
            base.path(),
            clock.clone(),
            1,
            Duration::from_millis(100),
            10_000_000,
        );
        let first = manager
            .create(
                CodingJobId::new(),
                repo.path(),
                &commit,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let first_path = first.path.clone();
        manager
            .finish(first, false, CancellationToken::new())
            .await
            .unwrap();
        clock.advance(1);
        let second = manager
            .create(
                CodingJobId::new(),
                repo.path(),
                &commit,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let second_path = second.path.clone();
        manager
            .finish(second, false, CancellationToken::new())
            .await
            .unwrap();
        assert!(
            !first_path.exists(),
            "oldest retained worktree must be capped first"
        );
        assert!(second_path.exists());
        clock.advance(101);
        assert_eq!(manager.prune(CancellationToken::new()).await.unwrap(), 1);
        assert!(!second_path.exists());
    }

    #[tokio::test]
    async fn nonempty_generated_path_and_disk_overflow_refuse_creation() {
        let (repo, _first, commit) = fixture().await;
        let base = tempfile::tempdir().unwrap();
        let limited = manager(
            base.path(),
            Arc::new(TestClock::new(1, 0)),
            3,
            Duration::from_secs(60),
            1,
        );
        std::fs::write(base.path().join("budget"), "xx").unwrap();
        assert!(limited
            .create(
                CodingJobId::new(),
                repo.path(),
                &commit,
                CancellationToken::new()
            )
            .await
            .is_err());

        let roomy = manager(
            base.path(),
            Arc::new(TestClock::new(1, 0)),
            3,
            Duration::from_secs(60),
            10_000_000,
        );
        let id = CodingJobId::new();
        let path = roomy.managed_path(id);
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("foreign"), "data").unwrap();
        assert!(roomy
            .create(id, repo.path(), &commit, CancellationToken::new())
            .await
            .is_err());
    }
}

//! Startup reconciliation for retained Pi coding worktrees.

use crate::application::goal::CodingJobRecoveryRecord;
use anyhow::{bail, Context, Result};
use fabric::{Clock, CodingJobId, CodingJobStatus};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::application::agent_control::{
    AgentResourceLease, AgentResourceLeaseKind, AgentWorktreeReclaimer,
};

const QUARANTINE_DIR: &str = ".quarantine";

#[derive(Debug, Clone)]
pub struct WorktreeRecoveryConfig {
    pub base_dir: PathBuf,
    pub failed_ttl: Duration,
    pub retained_count_cap: usize,
    pub disk_budget_bytes: u64,
}

impl WorktreeRecoveryConfig {
    pub fn production(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            failed_ttl: Duration::from_secs(24 * 60 * 60),
            retained_count_cap: 16,
            disk_budget_bytes: 10 * 1024 * 1024 * 1024,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.failed_ttl.is_zero() || self.retained_count_cap == 0 || self.disk_budget_bytes == 0
        {
            bail!("worktree recovery limits must be positive");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeRecoveryOutcome {
    pub pruned_jobs: Vec<CodingJobId>,
    pub orphan_metadata: Vec<CodingJobId>,
    pub quarantined: Vec<PathBuf>,
    pub retained_count: usize,
    pub disk_usage_bytes: u64,
    pub allow_new_pi_work: bool,
    pub blocked_reason: Option<String>,
}

pub trait WorktreeCleaner: Send + Sync {
    fn remove_known(&self, path: &Path) -> Result<()>;
}

/// Production deletion gate for Agent worktrees. Metadata must name one
/// canonical direct child of the configured root, match the retained HEAD,
/// and be clean. Unsafe or ambiguous worktrees are retained for inspection.
pub struct VerifiedAgentWorktreeReclaimer {
    cleaner: Arc<dyn WorktreeCleaner>,
}

impl Default for VerifiedAgentWorktreeReclaimer {
    fn default() -> Self {
        Self {
            cleaner: Arc::new(FsWorktreeCleaner),
        }
    }
}

impl VerifiedAgentWorktreeReclaimer {
    pub fn with_cleaner(cleaner: Arc<dyn WorktreeCleaner>) -> Self {
        Self { cleaner }
    }
}

impl AgentWorktreeReclaimer for VerifiedAgentWorktreeReclaimer {
    fn reclaim(&self, lease: &AgentResourceLease) -> Result<()> {
        if lease.kind != AgentResourceLeaseKind::Worktree {
            bail!("resource lease is not a worktree lease");
        }
        let root = PathBuf::from(
            lease
                .worktree_root
                .as_deref()
                .context("missing worktree root")?,
        )
        .canonicalize()
        .context("canonicalizing expected worktree root")?;
        let path = PathBuf::from(
            lease
                .worktree_path
                .as_deref()
                .context("missing worktree path")?,
        )
        .canonicalize()
        .context("canonicalizing retained worktree")?;
        if path.parent() != Some(root.as_path()) {
            bail!("worktree is not a direct child of its verified root");
        }
        let head = std::process::Command::new("git")
            .args([
                "-C",
                path.to_str().context("non-UTF8 worktree path")?,
                "rev-parse",
                "HEAD",
            ])
            .output()
            .context("inspecting retained worktree HEAD")?;
        if !head.status.success()
            || String::from_utf8_lossy(&head.stdout).trim()
                != lease
                    .expected_head
                    .as_deref()
                    .context("missing expected worktree HEAD")?
        {
            bail!("retained worktree HEAD does not match its lease");
        }
        let status = std::process::Command::new("git")
            .args([
                "-C",
                path.to_str().context("non-UTF8 worktree path")?,
                "status",
                "--porcelain",
            ])
            .output()
            .context("inspecting retained worktree cleanliness")?;
        if !status.status.success() || !status.stdout.is_empty() {
            bail!("retained worktree is dirty or unreadable");
        }
        self.cleaner.remove_known(&path)
    }
}

#[derive(Debug, Default)]
pub struct FsWorktreeCleaner;
impl WorktreeCleaner for FsWorktreeCleaner {
    fn remove_known(&self, path: &Path) -> Result<()> {
        std::fs::remove_dir_all(path)
            .with_context(|| format!("removing retained coding worktree: {}", path.display()))
    }
}

pub struct WorktreeRecoveryService {
    config: WorktreeRecoveryConfig,
    records: Vec<CodingJobRecoveryRecord>,
    clock: Arc<dyn Clock>,
    cleaner: Arc<dyn WorktreeCleaner>,
}

impl WorktreeRecoveryService {
    pub fn new(
        config: WorktreeRecoveryConfig,
        records: Vec<CodingJobRecoveryRecord>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        Self::with_cleaner(config, records, clock, Arc::new(FsWorktreeCleaner))
    }

    pub fn with_cleaner(
        config: WorktreeRecoveryConfig,
        records: Vec<CodingJobRecoveryRecord>,
        clock: Arc<dyn Clock>,
        cleaner: Arc<dyn WorktreeCleaner>,
    ) -> Result<Self> {
        config.validate()?;
        std::fs::create_dir_all(&config.base_dir).context("creating coding worktree base")?;
        let mut config = config;
        config.base_dir = config
            .base_dir
            .canonicalize()
            .context("canonicalizing coding worktree base")?;
        Ok(Self {
            config,
            records,
            clock,
            cleaner,
        })
    }

    pub fn recover(&self) -> Result<WorktreeRecoveryOutcome> {
        let mut by_name = HashMap::new();
        let mut unsafe_metadata = Vec::new();
        for record in self.records.clone() {
            match single_component(&record.worktree_ref) {
                Some(name) => {
                    if by_name.insert(name, record.clone()).is_some() {
                        unsafe_metadata.push(record.job_id);
                    }
                }
                None => unsafe_metadata.push(record.job_id),
            }
        }

        let quarantine = self.config.base_dir.join(QUARANTINE_DIR);
        std::fs::create_dir_all(&quarantine).context("creating worktree quarantine")?;
        let mut seen = HashSet::new();
        let mut quarantined = Vec::new();
        let mut cleanup_failures = Vec::new();
        let mut pruned_jobs = Vec::new();

        for entry in std::fs::read_dir(&self.config.base_dir).context("scanning worktree base")? {
            let entry = entry?;
            let name = entry.file_name();
            if name == QUARANTINE_DIR {
                continue;
            }
            let path = entry.path();
            let Some(record) = by_name.get(&name) else {
                let destination = unique_quarantine_path(&quarantine, &name);
                std::fs::rename(&path, &destination).with_context(|| {
                    format!("quarantining unknown worktree: {}", path.display())
                })?;
                tracing::warn!(path = %destination.display(), "Quarantined unknown coding worktree for manual review");
                quarantined.push(destination);
                continue;
            };
            seen.insert(record.job_id);
            if expired_failed(record, self.clock.wall_now().0, self.config.failed_ttl) {
                match self.cleaner.remove_known(&path) {
                    Ok(()) => pruned_jobs.push(record.job_id),
                    Err(error) => cleanup_failures.push(format!("{}: {error:#}", path.display())),
                }
            }
        }

        let mut orphan_metadata: Vec<_> = by_name
            .values()
            .filter(|record| !seen.contains(&record.job_id))
            .map(|record| record.job_id)
            .collect();
        orphan_metadata.extend(unsafe_metadata);
        orphan_metadata.sort_by_key(|id| id.0);
        orphan_metadata.dedup();

        let retained_count = std::fs::read_dir(&self.config.base_dir)?
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name() != QUARANTINE_DIR)
            .count();
        let disk_usage_bytes = directory_size(&self.config.base_dir)?;
        let mut reasons = cleanup_failures;
        if retained_count > self.config.retained_count_cap {
            reasons.push(format!(
                "retained worktree count {retained_count} exceeds cap {}",
                self.config.retained_count_cap
            ));
        }
        if disk_usage_bytes > self.config.disk_budget_bytes {
            reasons.push(format!(
                "worktree disk use {disk_usage_bytes} exceeds budget {}",
                self.config.disk_budget_bytes
            ));
        }
        if !orphan_metadata.is_empty() {
            tracing::warn!(
                count = orphan_metadata.len(),
                "Coding job metadata has no retained worktree"
            );
        }
        let blocked_reason = (!reasons.is_empty()).then(|| reasons.join("; "));
        Ok(WorktreeRecoveryOutcome {
            pruned_jobs,
            orphan_metadata,
            quarantined,
            retained_count,
            disk_usage_bytes,
            allow_new_pi_work: blocked_reason.is_none(),
            blocked_reason,
        })
    }
}

fn single_component(path: &Path) -> Option<OsString> {
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None) => Some(name.to_os_string()),
        _ => None,
    }
}

fn expired_failed(record: &CodingJobRecoveryRecord, now_ms: i64, ttl: Duration) -> bool {
    let failed = matches!(
        record.status,
        CodingJobStatus::Failed | CodingJobStatus::TimedOut | CodingJobStatus::Cancelled
    );
    let ttl_ms = ttl.as_millis().min(i64::MAX as u128) as i64;
    failed && now_ms.saturating_sub(record.updated_at_ms) >= ttl_ms
}

fn unique_quarantine_path(base: &Path, name: &OsString) -> PathBuf {
    let candidate = base.join(name);
    if !candidate.exists() {
        return candidate;
    }
    for suffix in 1_u64.. {
        let candidate = base.join(format!("{}.{}", name.to_string_lossy(), suffix));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn directory_size(path: &Path) -> Result<u64> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_file() || metadata.file_type().is_symlink() {
        return Ok(metadata.len());
    }
    let mut total = metadata.len();
    for entry in std::fs::read_dir(path)? {
        total = total.saturating_add(directory_size(&entry?.path())?);
    }
    Ok(total)
}

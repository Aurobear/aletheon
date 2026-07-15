use anyhow::{bail, Result};
use executive::r#impl::goal::CodingJobRecoveryRecord;
use executive::r#impl::runtime::worktree_recovery::{
    WorktreeCleaner, WorktreeRecoveryConfig, WorktreeRecoveryService,
};
use fabric::{Clock, CodingJobId, CodingJobStatus, MonoTime, WallTime};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

struct FixedClock(i64);
impl Clock for FixedClock {
    fn wall_now(&self) -> WallTime {
        WallTime(self.0)
    }
    fn mono_now(&self) -> MonoTime {
        MonoTime(self.0.max(0) as u64)
    }
}

fn record(status: CodingJobStatus, updated_at_ms: i64) -> CodingJobRecoveryRecord {
    let job_id = CodingJobId::new();
    CodingJobRecoveryRecord {
        job_id,
        worktree_ref: PathBuf::from(format!("job-{}", job_id.0)),
        status,
        updated_at_ms,
    }
}

fn config(base: &Path) -> WorktreeRecoveryConfig {
    WorktreeRecoveryConfig {
        base_dir: base.to_owned(),
        failed_ttl: Duration::from_secs(10),
        retained_count_cap: 4,
        disk_budget_bytes: 1024 * 1024,
    }
}

fn service(base: &Path, records: Vec<CodingJobRecoveryRecord>) -> WorktreeRecoveryService {
    WorktreeRecoveryService::new(config(base), records, Arc::new(FixedClock(20_000))).unwrap()
}

#[test]
fn orphan_metadata_is_reported_without_fabricating_a_directory() {
    let temp = TempDir::new().unwrap();
    let metadata = record(CodingJobStatus::Succeeded, 19_000);
    let outcome = service(temp.path(), vec![metadata.clone()])
        .recover()
        .unwrap();
    assert_eq!(outcome.orphan_metadata, vec![metadata.job_id]);
    assert!(outcome.allow_new_pi_work);
    assert!(!temp.path().join(metadata.worktree_ref).exists());
}

#[test]
fn unknown_directory_is_quarantined_and_never_deleted() {
    let temp = TempDir::new().unwrap();
    let unknown = temp.path().join("operator-notes");
    std::fs::create_dir(&unknown).unwrap();
    std::fs::write(unknown.join("keep.txt"), "manual evidence").unwrap();
    let outcome = service(temp.path(), vec![]).recover().unwrap();
    assert_eq!(outcome.quarantined.len(), 1);
    assert!(!unknown.exists());
    assert_eq!(
        std::fs::read_to_string(outcome.quarantined[0].join("keep.txt")).unwrap(),
        "manual evidence"
    );
}

#[test]
fn expired_known_failed_job_is_pruned() {
    let temp = TempDir::new().unwrap();
    let failed = record(CodingJobStatus::Failed, 1_000);
    let path = temp.path().join(&failed.worktree_ref);
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("diff"), "failed change").unwrap();
    let outcome = service(temp.path(), vec![failed.clone()])
        .recover()
        .unwrap();
    assert_eq!(outcome.pruned_jobs, vec![failed.job_id]);
    assert!(!path.exists());
    assert!(outcome.allow_new_pi_work);
}

#[test]
fn active_and_recent_failed_jobs_are_preserved() {
    let temp = TempDir::new().unwrap();
    let active = record(CodingJobStatus::Succeeded, 1_000);
    let recent = record(CodingJobStatus::Failed, 15_000);
    for item in [&active, &recent] {
        std::fs::create_dir(temp.path().join(&item.worktree_ref)).unwrap();
    }
    let outcome = service(temp.path(), vec![active.clone(), recent.clone()])
        .recover()
        .unwrap();
    assert!(outcome.pruned_jobs.is_empty());
    assert!(temp.path().join(active.worktree_ref).exists());
    assert!(temp.path().join(recent.worktree_ref).exists());
    assert!(outcome.allow_new_pi_work);
}

#[test]
fn disk_overflow_blocks_new_pi_work_when_safe_cleanup_cannot_restore_budget() {
    let temp = TempDir::new().unwrap();
    let active = record(CodingJobStatus::Succeeded, 19_000);
    let path = temp.path().join(&active.worktree_ref);
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("large"), vec![0_u8; 4096]).unwrap();
    let mut limits = config(temp.path());
    limits.disk_budget_bytes = 32;
    let outcome = WorktreeRecoveryService::new(limits, vec![active], Arc::new(FixedClock(20_000)))
        .unwrap()
        .recover()
        .unwrap();
    assert!(!outcome.allow_new_pi_work);
    assert!(outcome.blocked_reason.unwrap().contains("exceeds budget"));
    assert!(path.exists());
}

#[test]
fn retained_count_overflow_blocks_new_pi_work_without_deleting_active_jobs() {
    let temp = TempDir::new().unwrap();
    let first = record(CodingJobStatus::Succeeded, 19_000);
    let second = record(CodingJobStatus::Retained, 19_000);
    for item in [&first, &second] {
        std::fs::create_dir(temp.path().join(&item.worktree_ref)).unwrap();
    }
    let mut limits = config(temp.path());
    limits.retained_count_cap = 1;
    let outcome = WorktreeRecoveryService::new(
        limits,
        vec![first.clone(), second.clone()],
        Arc::new(FixedClock(20_000)),
    )
    .unwrap()
    .recover()
    .unwrap();
    assert!(!outcome.allow_new_pi_work);
    assert!(outcome.blocked_reason.unwrap().contains("exceeds cap"));
    assert!(temp.path().join(first.worktree_ref).exists());
    assert!(temp.path().join(second.worktree_ref).exists());
}

struct InterruptedCleaner;
impl WorktreeCleaner for InterruptedCleaner {
    fn remove_known(&self, _path: &Path) -> Result<()> {
        bail!("simulated interrupted cleanup")
    }
}

#[test]
fn interrupted_cleanup_preserves_job_and_blocks_new_pi_work() {
    let temp = TempDir::new().unwrap();
    let failed = record(CodingJobStatus::TimedOut, 1_000);
    let path = temp.path().join(&failed.worktree_ref);
    std::fs::create_dir(&path).unwrap();
    let outcome = WorktreeRecoveryService::with_cleaner(
        config(temp.path()),
        vec![failed],
        Arc::new(FixedClock(20_000)),
        Arc::new(InterruptedCleaner),
    )
    .unwrap()
    .recover()
    .unwrap();
    assert!(!outcome.allow_new_pi_work);
    assert!(outcome
        .blocked_reason
        .unwrap()
        .contains("interrupted cleanup"));
    assert!(path.exists());
}

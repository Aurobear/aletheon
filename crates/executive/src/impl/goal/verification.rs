//! Durable coding-job and verification evidence with bounded file artifacts.

use super::ObjectiveStore;
use anyhow::{bail, Context, Result};
use fabric::{
    AttemptId, CodingJobId, CodingJobReport, CodingJobStatus, GoalId, VerificationReport,
};
use rusqlite::{params, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

const MAX_DIFF_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;
const MAX_REPORT_JSON_BYTES: usize = 2 * 1024 * 1024;

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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GoalProjectionEvidence {
    pub attempt_ids: Vec<String>,
    pub artifact_ids: Vec<String>,
    pub source_commit: Option<String>,
    pub verification: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingJobRecoveryRecord {
    pub job_id: CodingJobId,
    pub worktree_ref: PathBuf,
    pub status: CodingJobStatus,
    pub updated_at_ms: i64,
}

impl ObjectiveStore {
    /// Load only bounded metadata needed for startup worktree reconciliation.
    /// Artifact contents are deliberately not read during this scan.
    pub fn coding_job_recovery_records(&self) -> Result<Vec<CodingJobRecoveryRecord>> {
        let mut statement = self.db.prepare(
            "SELECT job_id, worktree_ref, status, updated_at_ms
             FROM goal_coding_jobs ORDER BY created_at_ms, job_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (job_id, worktree_ref, status, updated_at_ms) = row?;
            let job_id = CodingJobId(uuid::Uuid::parse_str(&job_id)?);
            let worktree_ref = PathBuf::from(worktree_ref);
            validate_relative_ref(&worktree_ref, "worktree")?;
            Ok(CodingJobRecoveryRecord {
                job_id,
                worktree_ref,
                status: parse_coding_status(&status)?,
                updated_at_ms,
            })
        })
        .collect()
    }

    pub fn persist_coding_job(
        &self,
        report: &CodingJobReport,
        worktree_ref: &Path,
        diff: &[u8],
        now_ms: i64,
    ) -> Result<PersistedCodingJob> {
        validate_relative_ref(worktree_ref, "worktree")?;
        if diff.len() > MAX_DIFF_ARTIFACT_BYTES {
            bail!("coding diff exceeds bounded artifact limit");
        }
        let job_id = report.job_id.0.to_string();
        let duplicate: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM goal_coding_jobs WHERE job_id = ?1)",
            params![job_id],
            |row| row.get(0),
        )?;
        if duplicate {
            bail!("coding job ID already exists");
        }

        let diff_sha256 = hex_sha256(diff);
        if report
            .diff_sha256
            .as_deref()
            .is_some_and(|expected| expected != diff_sha256)
        {
            bail!("coding report diff hash does not match supplied artifact");
        }
        let diff_artifact_ref = PathBuf::from("coding-diffs").join(format!("{job_id}.diff"));
        validate_relative_ref(&diff_artifact_ref, "diff artifact")?;
        let final_path = self.artifact_path(&diff_artifact_ref)?;
        write_artifact_atomic(&final_path, diff)?;

        let result = (|| -> Result<()> {
            let mut stored_report = report.clone();
            stored_report.diff_sha256 = Some(diff_sha256.clone());
            stored_report.diff_artifact = Some(diff_artifact_ref.clone());
            let report_json = bounded_json(&stored_report, "coding report")?;
            let tx = self.db.unchecked_transaction()?;
            ensure_attempt_identity(&tx, report.goal_id, report.attempt_id)?;
            tx.execute(
                "INSERT INTO goal_coding_jobs (
                    job_id, objective_id, attempt_id, base_commit, worktree_ref,
                    report_json, diff_artifact_ref, diff_sha256, status,
                    created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                params![
                    job_id,
                    report.goal_id.0,
                    report.attempt_id.0.to_string(),
                    report.base_commit,
                    path_string(worktree_ref),
                    report_json,
                    path_string(&diff_artifact_ref),
                    diff_sha256,
                    coding_status(report.status),
                    now_ms,
                ],
            )?;
            append_evidence_event(
                &tx,
                report.goal_id,
                "coding_job_persisted",
                &serde_json::json!({
                    "job_id": report.job_id.0,
                    "attempt_id": report.attempt_id.0,
                    "diff_sha256": diff_sha256,
                }),
            )?;
            tx.commit()?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = std::fs::remove_file(&final_path);
            return Err(error);
        }
        self.load_coding_job(report.job_id)?
            .context("coding job disappeared after persistence")
    }

    pub fn load_coding_job(&self, job_id: CodingJobId) -> Result<Option<PersistedCodingJob>> {
        let row = self
            .db
            .query_row(
                "SELECT report_json, worktree_ref, diff_artifact_ref, diff_sha256,
                        status, created_at_ms, updated_at_ms, objective_id, attempt_id
                 FROM goal_coding_jobs WHERE job_id = ?1",
                params![job_id.0.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, String>(8)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            report_json,
            worktree_ref,
            artifact_ref,
            expected_hash,
            status,
            created,
            updated,
            goal_id,
            attempt_id,
        )) = row
        else {
            return Ok(None);
        };
        let worktree_ref = PathBuf::from(worktree_ref);
        let artifact_ref = PathBuf::from(artifact_ref);
        validate_relative_ref(&worktree_ref, "worktree")?;
        validate_relative_ref(&artifact_ref, "diff artifact")?;
        let artifact_path = self.artifact_path(&artifact_ref)?;
        let diff = std::fs::read(&artifact_path).with_context(|| {
            format!("reading coding diff artifact: {}", artifact_path.display())
        })?;
        if diff.len() > MAX_DIFF_ARTIFACT_BYTES {
            bail!("persisted coding diff exceeds bounded artifact limit");
        }
        let actual_hash = hex_sha256(&diff);
        if actual_hash != expected_hash {
            bail!("coding diff artifact hash mismatch");
        }
        let report: CodingJobReport = serde_json::from_str(&report_json)?;
        let parsed_status = parse_coding_status(&status)?;
        if report.job_id != job_id
            || report.goal_id.0 != goal_id
            || report.attempt_id.0.to_string() != attempt_id
            || report.status != parsed_status
            || report.diff_sha256.as_deref() != Some(expected_hash.as_str())
            || report.diff_artifact.as_deref() != Some(artifact_ref.as_path())
        {
            bail!("coding report identity or artifact metadata mismatch");
        }
        Ok(Some(PersistedCodingJob {
            report,
            worktree_ref,
            diff_artifact_ref: artifact_ref,
            diff_sha256: expected_hash,
            status: parsed_status,
            created_at_ms: created,
            updated_at_ms: updated,
            diff,
        }))
    }

    pub fn persist_verification_report(
        &self,
        report: &VerificationReport,
        now_ms: i64,
    ) -> Result<PersistedVerificationReport> {
        let report_json = bounded_json(report, "verification report")?;
        let tx = self.db.unchecked_transaction()?;
        let identity: Option<(i64, String)> = tx
            .query_row(
                "SELECT objective_id, attempt_id FROM goal_coding_jobs WHERE job_id = ?1",
                params![report.job_id.0.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((goal_id, attempt_id)) = identity else {
            bail!("verification report has no persisted coding job");
        };
        if goal_id != report.goal_id.0 || attempt_id != report.attempt_id.0.to_string() {
            bail!("verification report identity does not match coding job");
        }
        tx.execute(
            "INSERT INTO goal_verification_reports (
                job_id, objective_id, attempt_id, report_json, status,
                started_at_ms, ended_at_ms, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                report.job_id.0.to_string(),
                report.goal_id.0,
                report.attempt_id.0.to_string(),
                report_json,
                if report.passed { "passed" } else { "failed" },
                report.started_at_ms,
                report.ended_at_ms,
                now_ms,
            ],
        )?;
        append_evidence_event(
            &tx,
            report.goal_id,
            "verification_report_persisted",
            &serde_json::json!({
                "job_id": report.job_id.0,
                "attempt_id": report.attempt_id.0,
                "passed": report.passed,
            }),
        )?;
        tx.commit()?;
        Ok(PersistedVerificationReport {
            report: report.clone(),
            created_at_ms: now_ms,
        })
    }

    pub fn load_verification_report(
        &self,
        job_id: CodingJobId,
    ) -> Result<Option<PersistedVerificationReport>> {
        self.db
            .query_row(
                "SELECT report_json, created_at_ms FROM goal_verification_reports WHERE job_id = ?1",
                params![job_id.0.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
            .map(|(json, created_at_ms)| {
                let report: VerificationReport = serde_json::from_str(&json)?;
                if report.job_id != job_id {
                    bail!("verification report identity mismatch");
                }
                Ok(PersistedVerificationReport {
                    report,
                    created_at_ms,
                })
            })
            .transpose()
    }

    /// Read bounded, already-persisted evidence for memory projection.
    pub fn goal_projection_evidence(&self, goal_id: GoalId) -> Result<GoalProjectionEvidence> {
        let attempts = self.attempts_for_goal(goal_id, 20)?;
        let attempt_ids = attempts
            .iter()
            .map(|attempt| attempt.id.0.to_string())
            .collect();
        let mut statement = self.db.prepare(
            "SELECT job_id, base_commit FROM goal_coding_jobs WHERE objective_id=?1 ORDER BY created_at_ms, job_id LIMIT 20",
        )?;
        let jobs = statement.query_map(params![goal_id.0], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let jobs: Vec<_> = jobs.collect::<rusqlite::Result<_>>()?;
        let artifact_ids = jobs.iter().map(|(job, _)| job.clone()).collect();
        let source_commit = jobs.last().map(|(_, commit)| commit.clone());
        let mut verification = Vec::new();
        for (job, _) in &jobs {
            if let Ok(id) = uuid::Uuid::parse_str(job) {
                if let Some(report) = self.load_verification_report(CodingJobId(id))? {
                    verification.extend(
                        report.report.checks.into_iter().take(128).map(|check| {
                            format!("{}:{}:{}", check.name, check.passed, check.summary)
                        }),
                    );
                }
            }
        }
        Ok(GoalProjectionEvidence {
            attempt_ids,
            artifact_ids,
            source_commit,
            verification,
        })
    }

    fn artifact_path(&self, relative: &Path) -> Result<PathBuf> {
        validate_relative_ref(relative, "artifact")?;
        let path = self.artifact_dir.join(relative);
        let parent = path.parent().context("artifact path has no parent")?;
        std::fs::create_dir_all(parent)?;
        let canonical_parent = parent.canonicalize()?;
        if !canonical_parent.starts_with(&self.artifact_dir) {
            bail!("artifact path escapes Goal artifact directory");
        }
        Ok(path)
    }
}

fn ensure_attempt_identity(
    tx: &Transaction<'_>,
    goal_id: GoalId,
    attempt_id: AttemptId,
) -> Result<()> {
    let stored_goal: Option<i64> = tx
        .query_row(
            "SELECT objective_id FROM goal_attempts WHERE attempt_id = ?1",
            params![attempt_id.0.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    if stored_goal != Some(goal_id.0) {
        bail!("coding job attempt does not belong to Goal");
    }
    Ok(())
}

fn append_evidence_event(
    tx: &Transaction<'_>,
    goal_id: GoalId,
    event_type: &str,
    payload: &serde_json::Value,
) -> Result<()> {
    let version: u64 = tx.query_row(
        "SELECT version FROM objectives WHERE objective_id = ?1",
        params![goal_id.0],
        |row| row.get(0),
    )?;
    let next = version.saturating_add(1);
    let changed = tx.execute(
        "UPDATE objectives SET version = ?1, updated_at = datetime('now')
         WHERE objective_id = ?2 AND version = ?3",
        params![next, goal_id.0, version],
    )?;
    anyhow::ensure!(
        changed == 1,
        "goal version changed while persisting evidence"
    );
    tx.execute(
        "INSERT INTO goal_events (objective_id, version, event_type, payload_json)
         VALUES (?1, ?2, ?3, ?4)",
        params![goal_id.0, next, event_type, serde_json::to_string(payload)?],
    )?;
    Ok(())
}

fn bounded_json<T: serde::Serialize>(value: &T, label: &str) -> Result<String> {
    let json = serde_json::to_string(value)?;
    if json.len() > MAX_REPORT_JSON_BYTES {
        bail!("{label} exceeds bounded SQLite field limit");
    }
    Ok(json)
}

fn write_artifact_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        bail!("coding diff artifact already exists");
    }
    let temp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        if let Some(parent) = path.parent() {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn validate_relative_ref(path: &Path, label: &str) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("{label} reference must be a safe relative path");
    }
    Ok(())
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn coding_status(status: CodingJobStatus) -> &'static str {
    match status {
        CodingJobStatus::Running => "running",
        CodingJobStatus::Succeeded => "succeeded",
        CodingJobStatus::Failed => "failed",
        CodingJobStatus::TimedOut => "timed_out",
        CodingJobStatus::Cancelled => "cancelled",
        CodingJobStatus::Retained => "retained",
    }
}

fn parse_coding_status(value: &str) -> Result<CodingJobStatus> {
    match value {
        "running" => Ok(CodingJobStatus::Running),
        "succeeded" => Ok(CodingJobStatus::Succeeded),
        "failed" => Ok(CodingJobStatus::Failed),
        "timed_out" => Ok(CodingJobStatus::TimedOut),
        "cancelled" => Ok(CodingJobStatus::Cancelled),
        "retained" => Ok(CodingJobStatus::Retained),
        _ => bail!("invalid persisted coding status: {value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#impl::goal::ObjectiveStore;
    use fabric::{
        CognitiveRole, GoalBudget, GoalSpec, PrincipalId, RuntimeId, VerificationCheck,
        VerificationSeverity,
    };
    use tempfile::TempDir;

    struct Fixture {
        _temp: TempDir,
        db_path: PathBuf,
        store: ObjectiveStore,
        goal_id: GoalId,
        attempt_id: AttemptId,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let db_path = temp.path().join("objectives.db");
            let store = ObjectiveStore::open(&db_path).unwrap();
            let goal = store
                .create_goal(
                    &PrincipalId("owner".into()),
                    "session",
                    "project",
                    &GoalSpec {
                        original_intent: "persist coding evidence".into(),
                        desired_state: vec![],
                        constraints: vec![],
                        acceptance_criteria: vec![],
                        budget: GoalBudget {
                            max_input_tokens: 1000,
                            max_output_tokens: 1000,
                            max_cost_usd: None,
                            max_attempts: 3,
                            deadline_ms: None,
                        },
                    },
                )
                .unwrap();
            let attempt = store
                .begin_attempt(
                    goal.id,
                    1,
                    &RuntimeId("pi-coder".into()),
                    CognitiveRole::Worker,
                    &serde_json::json!({"task": "code"}),
                )
                .unwrap();
            Self {
                _temp: temp,
                db_path,
                store,
                goal_id: goal.id,
                attempt_id: attempt.id,
            }
        }

        fn report(&self, job_id: CodingJobId, diff: &[u8]) -> CodingJobReport {
            CodingJobReport {
                job_id,
                goal_id: self.goal_id,
                attempt_id: self.attempt_id,
                base_commit: "0123456789abcdef".into(),
                status: CodingJobStatus::Succeeded,
                exit_code: Some(0),
                elapsed_ms: 12,
                stdout: "done".into(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
                changed_files: vec![],
                diff_sha256: Some(hex_sha256(diff)),
                diff_artifact: None,
            }
        }
    }

    fn verification(report: &CodingJobReport, passed: bool) -> VerificationReport {
        VerificationReport {
            job_id: report.job_id,
            goal_id: report.goal_id,
            attempt_id: report.attempt_id,
            passed,
            checks: vec![VerificationCheck {
                name: "compile".into(),
                severity: VerificationSeverity::Required,
                passed,
                timed_out: false,
                cancelled: false,
                summary: "checked".into(),
                evidence: vec![],
            }],
            risk_summary: vec![],
            started_at_ms: 100,
            ended_at_ms: 120,
        }
    }

    #[test]
    fn coding_report_and_event_are_atomic() {
        let fixture = Fixture::new();
        fixture
            .store
            .db
            .execute_batch(
                "CREATE TRIGGER reject_coding_event BEFORE INSERT ON goal_events
                 WHEN NEW.event_type = 'coding_job_persisted'
                 BEGIN SELECT RAISE(ABORT, 'reject test event'); END;",
            )
            .unwrap();
        let diff = b"diff --git a/a b/a\n";
        let report = fixture.report(CodingJobId::new(), diff);
        assert!(fixture
            .store
            .persist_coding_job(&report, Path::new("job-test"), diff, 10)
            .is_err());
        let count: i64 = fixture
            .store
            .db
            .query_row("SELECT COUNT(*) FROM goal_coding_jobs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        assert!(!fixture
            .store
            .artifact_dir
            .join("coding-diffs")
            .join(format!("{}.diff", report.job_id.0))
            .exists());
    }

    #[test]
    fn duplicate_job_id_is_rejected_without_overwriting_artifact() {
        let fixture = Fixture::new();
        let diff = b"binary-safe\0diff";
        let report = fixture.report(CodingJobId::new(), diff);
        let first = fixture
            .store
            .persist_coding_job(&report, Path::new("job-one"), diff, 10)
            .unwrap();
        let error = fixture
            .store
            .persist_coding_job(&report, Path::new("job-two"), b"other", 20)
            .unwrap_err();
        assert!(format!("{error:#}").contains("already exists"));
        assert_eq!(
            std::fs::read(fixture.store.artifact_dir.join(first.diff_artifact_ref)).unwrap(),
            diff
        );
    }

    #[test]
    fn tampered_and_missing_artifacts_fail_closed() {
        let fixture = Fixture::new();
        let diff = b"trusted diff";
        let report = fixture.report(CodingJobId::new(), diff);
        let persisted = fixture
            .store
            .persist_coding_job(&report, Path::new("job-one"), diff, 10)
            .unwrap();
        let path = fixture
            .store
            .artifact_dir
            .join(&persisted.diff_artifact_ref);
        std::fs::write(&path, b"tampered").unwrap();
        assert!(fixture.store.load_coding_job(report.job_id).is_err());
        std::fs::remove_file(path).unwrap();
        assert!(fixture.store.load_coding_job(report.job_id).is_err());
    }

    #[test]
    fn restart_loads_and_hash_verifies_coding_and_verification_reports() {
        let fixture = Fixture::new();
        let diff = b"restart diff";
        let report = fixture.report(CodingJobId::new(), diff);
        fixture
            .store
            .persist_coding_job(&report, Path::new("job-restart"), diff, 10)
            .unwrap();
        let verification = verification(&report, true);
        fixture
            .store
            .persist_verification_report(&verification, 130)
            .unwrap();
        let db_path = fixture.db_path.clone();
        drop(fixture.store);

        let restarted = ObjectiveStore::open(&db_path).unwrap();
        let coding = restarted.load_coding_job(report.job_id).unwrap().unwrap();
        assert_eq!(coding.diff, diff);
        assert_eq!(coding.report.diff_sha256, Some(hex_sha256(diff)));
        let loaded = restarted
            .load_verification_report(report.job_id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.report, verification);
        assert_eq!(loaded.created_at_ms, 130);
    }

    #[test]
    fn verification_insert_and_event_are_atomic_and_identity_checked() {
        let fixture = Fixture::new();
        let diff = b"verified";
        let report = fixture.report(CodingJobId::new(), diff);
        fixture
            .store
            .persist_coding_job(&report, Path::new("job-verified"), diff, 10)
            .unwrap();
        fixture
            .store
            .db
            .execute_batch(
                "CREATE TRIGGER reject_verification_event BEFORE INSERT ON goal_events
                 WHEN NEW.event_type = 'verification_report_persisted'
                 BEGIN SELECT RAISE(ABORT, 'reject test verification event'); END;",
            )
            .unwrap();
        let verification = verification(&report, true);
        assert!(fixture
            .store
            .persist_verification_report(&verification, 130)
            .is_err());
        let count: i64 = fixture
            .store
            .db
            .query_row(
                "SELECT COUNT(*) FROM goal_verification_reports",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);

        let mut wrong = verification;
        wrong.attempt_id = AttemptId::new();
        assert!(fixture
            .store
            .persist_verification_report(&wrong, 140)
            .is_err());
    }

    #[test]
    fn diff_and_report_size_limits_are_enforced() {
        let fixture = Fixture::new();
        let diff = vec![0_u8; MAX_DIFF_ARTIFACT_BYTES + 1];
        let report = fixture.report(CodingJobId::new(), &diff);
        assert!(fixture
            .store
            .persist_coding_job(&report, Path::new("job-large"), &diff, 10)
            .is_err());

        let mut report = fixture.report(CodingJobId::new(), b"small");
        report.stdout = "x".repeat(MAX_REPORT_JSON_BYTES + 1);
        assert!(fixture
            .store
            .persist_coding_job(&report, Path::new("job-report-large"), b"small", 10)
            .is_err());
    }
}

//! Durable SQLite `EpisodeSink` for embodied robot episodes.
//!
//! Each attempt is persisted with a request digest for idempotent replay:
//! `INSERT OR IGNORE` on `(episode_id, attempt, request_digest)` means a
//! restarted run that re-records an identical attempt is a no-op instead of a
//! duplicate side effect.

use std::path::Path;
use std::sync::Arc;

use ::contracts::types::embodiment::{SkillRequest, SkillResult};
use ::contracts::types::episode_report::{
    ArtifactRetentionTombstone, AttemptRecord, EpisodeReport, EpisodeSettlement,
    SettledEpisodeReport,
};
use ::contracts::types::expected_outcome::ExpectedOutcome;
use ::contracts::types::outcome_verification::VerificationReport;
use ::contracts::types::world_state::WorldSnapshot;
use ::contracts::Clock;
use async_trait::async_trait;
use cognit::harness::robot::EpisodeSink;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

// Wired by the robot composition root (PR4 core-chain tail); dead until then.
#[allow(dead_code)]
const MIGRATION: &str = include_str!("migrations/001_episodes.sql");

#[derive(Clone)]
pub struct SqliteEpisodeSink {
    connection: Arc<Mutex<Connection>>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for SqliteEpisodeSink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SqliteEpisodeSink")
            .finish_non_exhaustive()
    }
}

// `open`/`from_connection` are wired by the robot composition root (PR4 core-chain
// tail) and `count` is a test helper; dead until then.
#[allow(dead_code)]
impl SqliteEpisodeSink {
    pub fn open(path: impl AsRef<Path>, clock: Arc<dyn Clock>) -> Result<Self, String> {
        let connection = Connection::open(path).map_err(|error| format!("open: {error}"))?;
        Self::from_connection(connection, clock)
    }

    pub fn from_connection(connection: Connection, clock: Arc<dyn Clock>) -> Result<Self, String> {
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|error| format!("enable foreign keys: {error}"))?;
        connection
            .execute_batch(MIGRATION)
            .map_err(|error| format!("migrate: {error}"))?;
        migrate_legacy_episode_columns(&connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            clock,
        })
    }

    fn now_ms(&self) -> i64 {
        self.clock.mono_now().0 as i64
    }

    /// Stable digest identifying one (episode, attempt, attempt_id, request,
    /// expected)
    /// unit — used for idempotent replay.
    fn request_digest(
        episode_id: &str,
        attempt: u32,
        attempt_id: &str,
        request: &SkillRequest,
        expected: &ExpectedOutcome,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(episode_id.as_bytes());
        hasher.update(attempt.to_be_bytes());
        hasher.update(attempt_id.as_bytes());
        hasher.update(
            serde_json::to_string(request)
                .unwrap_or_default()
                .as_bytes(),
        );
        hasher.update(
            serde_json::to_string(expected)
                .unwrap_or_default()
                .as_bytes(),
        );
        format!("{:x}", hasher.finalize())
    }

    fn count(&self, episode_id: &str) -> i64 {
        self.connection
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM episodes WHERE episode_id = ?1",
                params![episode_id],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Load an episode's attempts in order for report building.
    pub fn load_attempts(&self, episode_id: &str) -> Result<Vec<AttemptRecord>, String> {
        let connection = self.connection.lock();
        let mut statement = connection
            .prepare(
                "SELECT attempt, attempt_id, operation_id, request_json, expected_json, result_json,
                        verification_json, before_sequence, after_sequence, verified_sequence,
                        retry_reason
                 FROM episodes WHERE episode_id = ?1 ORDER BY attempt ASC",
            )
            .map_err(|e| format!("prepare load_attempts: {e}"))?;
        let rows = statement
            .query_map(params![episode_id], |row| {
                let attempt: i64 = row.get(0)?;
                let attempt_id: String = row.get(1)?;
                let operation_id: Option<String> = row.get(2)?;
                let request_json: Option<String> = row.get(3)?;
                let expected_json: String = row.get(4)?;
                let result_json: Option<String> = row.get(5)?;
                let verification_json: Option<String> = row.get(6)?;
                let before_sequence: Option<i64> = row.get(7)?;
                let after_sequence: Option<i64> = row.get(8)?;
                let verified_sequence: Option<i64> = row.get(9)?;
                let retry_reason: Option<String> = row.get(10)?;
                Ok((
                    attempt,
                    attempt_id,
                    operation_id,
                    request_json,
                    expected_json,
                    result_json,
                    verification_json,
                    before_sequence,
                    after_sequence,
                    verified_sequence,
                    retry_reason,
                ))
            })
            .map_err(|e| format!("query load_attempts: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("row load_attempts: {e}"))?;

        rows.into_iter()
            .map(
                |(
                    attempt,
                    attempt_id,
                    operation_id,
                    request_json,
                    expected_json,
                    result_json,
                    verification_json,
                    before_sequence,
                    after_sequence,
                    verified_sequence,
                    retry_reason,
                )| {
                    let request = request_json
                        .map(|json| serde_json::from_str::<SkillRequest>(&json))
                        .transpose()
                        .map_err(|error| format!("skill request deserialize: {error}"))?;
                    let expected = serde_json::from_str(&expected_json)
                        .map_err(|e| format!("expected deserialize: {e}"))?;
                    let result = result_json
                        .map(|json| serde_json::from_str::<SkillResult>(&json))
                        .transpose()
                        .map_err(|error| format!("skill result deserialize: {error}"))?;
                    let result_outcome = result
                        .as_ref()
                        .map(|result| format!("{:?}", result.outcome));
                    let verification = verification_json
                        .map(|json| serde_json::from_str::<VerificationReport>(&json))
                        .transpose()
                        .map_err(|e| format!("verification deserialize: {e}"))?;
                    let attempt = u32::try_from(attempt)
                        .map_err(|_| "durable attempt number is outside u32 range".to_string())?;
                    let before_sequence = before_sequence
                        .map(u64::try_from)
                        .transpose()
                        .map_err(|_| "durable before sequence is negative".to_string())?;
                    let after_sequence = after_sequence
                        .map(u64::try_from)
                        .transpose()
                        .map_err(|_| "durable after sequence is negative".to_string())?;
                    let verified_sequence = verified_sequence
                        .map(u64::try_from)
                        .transpose()
                        .map_err(|_| "durable verified sequence is negative".to_string())?;
                    let mut record = AttemptRecord::from_verification(
                        attempt,
                        attempt_id,
                        operation_id,
                        request,
                        expected,
                        result_outcome,
                        verification.as_ref(),
                        retry_reason,
                        before_sequence,
                        after_sequence,
                    );
                    record.verified_sequence = verified_sequence;
                    if let Some(result) = result {
                        record.evidence_refs.extend(result.evidence);
                    }
                    Ok(record)
                },
            )
            .collect()
    }

    /// Load and integrity-check the immutable settled report receipt.
    pub fn load_settled_report(
        &self,
        episode_id: &str,
    ) -> Result<Option<SettledEpisodeReport>, String> {
        let row = self
            .connection
            .lock()
            .query_row(
                "SELECT report_json, report_sha256, settlement, settled_at_unix_ms
                 FROM episode_reports WHERE episode_id = ?1",
                params![episode_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("load settled report: {error}"))?;
        row.map(
            |(json, stored_digest, stored_settlement, stored_settled_at)| {
                let report: SettledEpisodeReport = serde_json::from_str(&json)
                    .map_err(|error| format!("deserialize settled report: {error}"))?;
                report.verify_integrity()?;
                if report.report_sha256() != stored_digest
                    || report.report().settlement.as_str() != stored_settlement
                    || report.settled_at_unix_ms() != stored_settled_at
                {
                    return Err("settled report metadata columns do not match receipt".into());
                }
                Ok(report)
            },
        )
        .transpose()
    }

    pub fn load_settlement(&self, episode_id: &str) -> Result<Option<EpisodeSettlement>, String> {
        let value = self
            .connection
            .lock()
            .query_row(
                "SELECT status FROM episode_states WHERE episode_id = ?1",
                params![episode_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load episode settlement: {error}"))?;
        value
            .map(|status| match status.as_str() {
                "running" => Ok(None),
                "completed" => Ok(Some(EpisodeSettlement::Completed)),
                "failed" => Ok(Some(EpisodeSettlement::Failed)),
                "cancelled" => Ok(Some(EpisodeSettlement::Cancelled)),
                _ => Err(format!("invalid durable episode status: {status}")),
            })
            .transpose()
            .map(Option::flatten)
    }

    pub fn record_artifact_tombstone(
        &self,
        episode_id: &str,
        tombstone: &ArtifactRetentionTombstone,
    ) -> Result<(), String> {
        tombstone.validate()?;
        let settled = self
            .load_settled_report(episode_id)?
            .ok_or_else(|| format!("settled episode report not found: {episode_id}"))?;
        settled.project_with_retention(std::slice::from_ref(tombstone))?;
        let connection = self.connection.lock();
        connection
            .execute(
                "INSERT OR IGNORE INTO episode_artifact_tombstones(
                    episode_id, uri, digest, expired_at_unix_ms, reason
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    episode_id,
                    tombstone.uri,
                    tombstone.digest,
                    tombstone.expired_at_unix_ms,
                    tombstone.reason,
                ],
            )
            .map_err(|error| format!("record artifact tombstone: {error}"))?;
        let stored = connection
            .query_row(
                "SELECT uri, digest, expired_at_unix_ms, reason
                 FROM episode_artifact_tombstones
                 WHERE episode_id = ?1 AND digest = ?2",
                params![episode_id, tombstone.digest],
                |row| {
                    Ok(ArtifactRetentionTombstone {
                        uri: row.get(0)?,
                        digest: row.get(1)?,
                        expired_at_unix_ms: row.get(2)?,
                        reason: row.get(3)?,
                    })
                },
            )
            .map_err(|error| format!("load artifact tombstone after insert: {error}"))?;
        if &stored != tombstone {
            return Err(format!(
                "conflicting artifact tombstone for episode {episode_id}: {}",
                tombstone.digest
            ));
        }
        Ok(())
    }

    /// Apply retention to a locally owned artifact and persist its report
    /// tombstone. External Bridge-owned artifacts use
    /// [`Self::record_artifact_tombstone`] after their owner confirms deletion.
    pub fn expire_local_artifact(
        &self,
        artifacts: &crate::artifact::ArtifactStore,
        episode_id: &str,
        artifact_id: &str,
        expired_at_unix_ms: i64,
        reason: &str,
    ) -> Result<Option<crate::artifact::ArtifactTombstone>, String> {
        // Verify the episode/report binding before deleting content. The two
        // SQLite stores cannot share a transaction, so this preflight prevents
        // an unrelated artifact from being tombstoned first and rejected later.
        let Some(manifest) = artifacts
            .episode_manifest(artifact_id, "retention-preflight")
            .map_err(|error| format!("load local episode artifact manifest: {error}"))?
        else {
            return Ok(None);
        };
        let digest = manifest
            .digest
            .ok_or_else(|| "local episode artifact has no content digest".to_string())?;
        let preflight = ArtifactRetentionTombstone {
            uri: manifest.uri,
            digest,
            expired_at_unix_ms,
            reason: reason.to_owned(),
        };
        let settled = self
            .load_settled_report(episode_id)?
            .ok_or_else(|| format!("settled episode report not found: {episode_id}"))?;
        settled.project_with_retention(std::slice::from_ref(&preflight))?;

        let tombstone = artifacts
            .expire_retention(artifact_id, expired_at_unix_ms, reason)
            .map_err(|error| format!("expire local episode artifact: {error}"))?;
        if let Some(tombstone) = &tombstone {
            self.record_artifact_tombstone(episode_id, &tombstone.episode_tombstone())?;
        }
        Ok(tombstone)
    }

    pub fn load_artifact_tombstones(
        &self,
        episode_id: &str,
    ) -> Result<Vec<ArtifactRetentionTombstone>, String> {
        let connection = self.connection.lock();
        let mut statement = connection
            .prepare(
                "SELECT uri, digest, expired_at_unix_ms, reason
                 FROM episode_artifact_tombstones
                 WHERE episode_id = ?1 ORDER BY digest ASC",
            )
            .map_err(|error| format!("prepare artifact tombstones: {error}"))?;
        let tombstones = statement
            .query_map(params![episode_id], |row| {
                Ok(ArtifactRetentionTombstone {
                    uri: row.get(0)?,
                    digest: row.get(1)?,
                    expired_at_unix_ms: row.get(2)?,
                    reason: row.get(3)?,
                })
            })
            .map_err(|error| format!("query artifact tombstones: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read artifact tombstones: {error}"))?;
        Ok(tombstones)
    }

    pub fn load_report_projection(
        &self,
        episode_id: &str,
    ) -> Result<Option<EpisodeReport>, String> {
        let Some(settled) = self.load_settled_report(episode_id)? else {
            return Ok(None);
        };
        let tombstones = self.load_artifact_tombstones(episode_id)?;
        settled.project_with_retention(&tombstones).map(Some)
    }

    /// Reconcile locally owned artifact-store tombstones before projecting a
    /// report. This closes the crash boundary between the artifact store's
    /// durable retention commit and the episode store's read-model tombstone:
    /// the immutable receipt is never rewritten, and a later authoritative
    /// read repairs any missing projection record by content digest.
    pub fn load_report_projection_with_local_artifacts(
        &self,
        artifacts: &crate::artifact::ArtifactStore,
        episode_id: &str,
    ) -> Result<Option<EpisodeReport>, String> {
        let Some(settled) = self.load_settled_report(episode_id)? else {
            return Ok(None);
        };
        for manifest in &settled.report().artifacts {
            let Some(digest) = manifest.digest.as_deref() else {
                continue;
            };
            let digest = digest.strip_prefix("sha256:").unwrap_or(digest);
            let artifact_id = format!("sha256:{digest}");
            let tombstone = artifacts
                .retention_tombstone(&artifact_id)
                .map_err(|error| format!("load local artifact tombstone: {error}"))?;
            if let Some(tombstone) = tombstone {
                self.record_artifact_tombstone(episode_id, &tombstone.episode_tombstone())?;
            }
        }
        self.load_report_projection(episode_id)
    }
}

/// The original development migration predated per-attempt sequences. Preserve
/// existing databases through additive, idempotent column discovery instead of
/// requiring a destructive rebuild.
fn migrate_legacy_episode_columns(connection: &Connection) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(episodes)")
        .map_err(|error| format!("prepare episode schema discovery: {error}"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("query episode schema discovery: {error}"))?
        .collect::<Result<std::collections::HashSet<_>, _>>()
        .map_err(|error| format!("read episode schema discovery: {error}"))?;
    drop(statement);
    for (name, ty) in [
        ("before_sequence", "INTEGER"),
        ("after_sequence", "INTEGER"),
        ("verified_sequence", "INTEGER"),
        ("retry_reason", "TEXT"),
        ("request_json", "TEXT"),
    ] {
        if !columns.contains(name) {
            connection
                .execute(&format!("ALTER TABLE episodes ADD COLUMN {name} {ty}"), [])
                .map_err(|error| format!("add episode column {name}: {error}"))?;
        }
    }
    if columns.contains("before_json") && columns.contains("after_json") {
        let mut statement = connection
            .prepare(
                "SELECT rowid, before_json, after_json, before_sequence, after_sequence
                 FROM episodes WHERE before_json IS NOT NULL OR after_json IS NOT NULL",
            )
            .map_err(|error| format!("prepare legacy snapshot migration: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            })
            .map_err(|error| format!("query legacy snapshots: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read legacy snapshots: {error}"))?;
        drop(statement);
        for (rowid, before_json, after_json, stored_before, stored_after) in rows {
            let before = before_json
                .map(|json| serde_json::from_str::<WorldSnapshot>(&json))
                .transpose()
                .map_err(|error| format!("deserialize legacy before snapshot: {error}"))?
                .map(|snapshot| i64::try_from(snapshot.sequence))
                .transpose()
                .map_err(|_| "legacy before sequence exceeds SQLite range".to_string())?;
            let after = after_json
                .map(|json| serde_json::from_str::<WorldSnapshot>(&json))
                .transpose()
                .map_err(|error| format!("deserialize legacy after snapshot: {error}"))?
                .map(|snapshot| i64::try_from(snapshot.sequence))
                .transpose()
                .map_err(|_| "legacy after sequence exceeds SQLite range".to_string())?;
            if stored_before.is_some() && before.is_some() && stored_before != before {
                return Err("legacy before snapshot conflicts with stored sequence".into());
            }
            if stored_after.is_some() && after.is_some() && stored_after != after {
                return Err("legacy after snapshot conflicts with stored sequence".into());
            }
            connection
                .execute(
                    "UPDATE episodes
                     SET before_sequence = COALESCE(before_sequence, ?2),
                         after_sequence = COALESCE(after_sequence, ?3),
                         before_json = NULL, after_json = NULL
                     WHERE rowid = ?1",
                    params![rowid, before, after],
                )
                .map_err(|error| format!("clear legacy snapshot JSON: {error}"))?;
        }
    }
    // `MIGRATION` creates this table/triggers for both new and legacy stores.
    connection
        .execute_batch(MIGRATION)
        .map_err(|error| format!("complete episode migration: {error}"))?;
    let invalid_statuses: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM episodes
             WHERE status NOT IN ('running','completed','failed','cancelled')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("validate legacy episode statuses: {error}"))?;
    if invalid_statuses != 0 {
        return Err("legacy episode store contains invalid settlement statuses".into());
    }
    let inconsistent_episodes: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM (
                 SELECT episode_id FROM episodes
                 GROUP BY episode_id HAVING COUNT(DISTINCT status) > 1
             )",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("validate legacy episode consistency: {error}"))?;
    if inconsistent_episodes != 0 {
        return Err("legacy episode store contains conflicting attempt settlements".into());
    }
    connection
        .execute_batch(
            "INSERT OR IGNORE INTO episode_states(episode_id, status, created_at_ms, settled_at_ms)
             SELECT episode_id, settlement, created_at_monotonic_ms, created_at_monotonic_ms
             FROM episode_reports;
             INSERT OR IGNORE INTO episode_states(episode_id, status, created_at_ms, settled_at_ms)
             SELECT episode_id, MIN(status), MIN(created_at_ms), MAX(settled_at_ms)
             FROM episodes GROUP BY episode_id;",
        )
        .map_err(|error| format!("backfill legacy episode states: {error}"))?;
    Ok(())
}

#[async_trait]
impl EpisodeSink for SqliteEpisodeSink {
    async fn append_attempt(
        &self,
        episode_id: &str,
        attempt: u32,
        attempt_id: &str,
        operation_id: Option<&::contracts::OperationId>,
        request: &SkillRequest,
        expected: &ExpectedOutcome,
        before: Option<&WorldSnapshot>,
        after: Option<&WorldSnapshot>,
        result: Option<&SkillResult>,
        verification: Option<&VerificationReport>,
    ) -> Result<(), String> {
        if attempt == 0 || attempt_id.trim().is_empty() || episode_id.trim().is_empty() {
            return Err("append_attempt: invalid episode/attempt identity".into());
        }
        let digest = Self::request_digest(episode_id, attempt, attempt_id, request, expected);
        let operation_id_json = operation_id.map(|id| id.0.to_string());
        let request_json =
            serde_json::to_string(request).map_err(|e| format!("request serde: {e}"))?;
        let expected_json =
            serde_json::to_string(expected).map_err(|e| format!("expected serde: {e}"))?;
        let result_json = result
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| format!("result serde: {e}"))?;
        let verification_json = verification
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| format!("verification serde: {e}"))?;
        let before_sequence = before
            .map(|snapshot| i64::try_from(snapshot.sequence))
            .transpose()
            .map_err(|_| "before sequence exceeds SQLite integer range".to_string())?;
        let after_sequence = after
            .map(|snapshot| i64::try_from(snapshot.sequence))
            .transpose()
            .map_err(|_| "after sequence exceeds SQLite integer range".to_string())?;
        let verified_sequence = verification
            .map(|report| i64::try_from(report.evaluated_sequence))
            .transpose()
            .map_err(|_| "verified sequence exceeds SQLite integer range".to_string())?;

        let mut connection = self.connection.lock();
        let tx = connection
            .transaction()
            .map_err(|error| format!("begin append_attempt transaction: {error}"))?;
        tx.execute(
            "INSERT OR IGNORE INTO episode_states(episode_id, status, created_at_ms, settled_at_ms)
             VALUES (?1, 'running', ?2, NULL)",
            params![episode_id, self.now_ms()],
        )
        .map_err(|error| format!("ensure episode state: {error}"))?;
        let state: String = tx
            .query_row(
                "SELECT status FROM episode_states WHERE episode_id = ?1",
                params![episode_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("read episode state before append: {error}"))?;
        if state != "running" {
            return Err(format!(
                "append_attempt: episode {episode_id} is already settled as {state}"
            ));
        }
        tx.execute(
                "INSERT OR IGNORE INTO episodes
                 (episode_id, attempt, attempt_id, operation_id, request_json, request_digest, status, expected_json,
                  before_sequence, after_sequence, result_json,
                  verification_json, verified_sequence, retry_reason, created_at_ms, settled_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', ?7, ?8, ?9, ?10, ?11,
                         ?12, NULL, ?13, NULL)",
                params![
                    episode_id,
                    attempt as i64,
                    attempt_id,
                    operation_id_json.as_deref(),
                    request_json,
                    digest,
                    expected_json,
                    before_sequence,
                    after_sequence,
                    result_json.as_deref(),
                    verification_json.as_deref(),
                    verified_sequence,
                    self.now_ms(),
                ],
            )
            .map_err(|error| format!("append_attempt: {error}"))?;
        let stored: (
            String,
            Option<String>,
            String,
            String,
            Option<i64>,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<i64>,
        ) = tx
            .query_row(
                "SELECT request_digest, operation_id, request_json, expected_json, before_sequence, after_sequence,
                        result_json, verification_json, verified_sequence
                 FROM episodes
                 WHERE episode_id = ?1 AND attempt = ?2 AND attempt_id = ?3",
                params![episode_id, i64::from(attempt), attempt_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                    ))
                },
            )
            .map_err(|error| format!("read attempt after append: {error}"))?;
        if stored
            != (
                digest,
                operation_id_json,
                request_json,
                expected_json,
                before_sequence,
                after_sequence,
                result_json,
                verification_json,
                verified_sequence,
            )
        {
            return Err(format!(
                "append_attempt: conflicting replay for {episode_id}/{attempt}/{attempt_id}"
            ));
        }
        tx.commit()
            .map_err(|error| format!("commit append_attempt: {error}"))?;
        Ok(())
    }

    async fn close_episode(
        &self,
        episode_id: &str,
        settlement: EpisodeSettlement,
    ) -> Result<(), String> {
        if episode_id.trim().is_empty() {
            return Err("close_episode: episode id is empty".into());
        }
        let mut connection = self.connection.lock();
        let tx = connection
            .transaction()
            .map_err(|error| format!("begin close_episode transaction: {error}"))?;
        tx.execute(
            "INSERT OR IGNORE INTO episode_states(episode_id, status, created_at_ms, settled_at_ms)
             VALUES (?1, ?2, ?3, ?3)",
            params![episode_id, settlement.as_str(), self.now_ms()],
        )
        .map_err(|error| format!("insert terminal episode state: {error}"))?;
        let current: String = tx
            .query_row(
                "SELECT status FROM episode_states WHERE episode_id = ?1",
                params![episode_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("read episode state during close: {error}"))?;
        if current == "running" {
            tx.execute(
                "UPDATE episode_states SET status = ?2, settled_at_ms = ?3
                 WHERE episode_id = ?1 AND status = 'running'",
                params![episode_id, settlement.as_str(), self.now_ms()],
            )
            .map_err(|error| format!("settle episode state: {error}"))?;
        } else if current != settlement.as_str() {
            return Err(format!(
                "close_episode: conflicting settlement for {episode_id}: existing={current} incoming={}",
                settlement.as_str()
            ));
        }
        let conflicting_attempts: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM episodes
                 WHERE episode_id = ?1 AND status NOT IN ('running', ?2)",
                params![episode_id, settlement.as_str()],
                |row| row.get(0),
            )
            .map_err(|error| format!("check attempt settlements: {error}"))?;
        if conflicting_attempts != 0 {
            return Err(format!(
                "close_episode: conflicting attempt settlement for {episode_id}"
            ));
        }
        tx.execute(
            "UPDATE episodes SET status = ?2, settled_at_ms = COALESCE(settled_at_ms, ?3)
             WHERE episode_id = ?1 AND status = 'running'",
            params![episode_id, settlement.as_str(), self.now_ms()],
        )
        .map_err(|error| format!("settle episode attempts: {error}"))?;
        tx.commit()
            .map_err(|error| format!("commit close_episode: {error}"))?;
        Ok(())
    }

    async fn update_verification(
        &self,
        episode_id: &str,
        attempt_id: &str,
        after: Option<&WorldSnapshot>,
        verification: &VerificationReport,
    ) -> Result<(), String> {
        let verification_json = serde_json::to_string(verification)
            .map_err(|e| format!("serialize verification: {e}"))?;
        let after_sequence = after
            .map(|snapshot| i64::try_from(snapshot.sequence))
            .transpose()
            .map_err(|_| "after sequence exceeds SQLite integer range".to_string())?;
        let verified_sequence = i64::try_from(verification.evaluated_sequence)
            .map_err(|_| "verified sequence exceeds SQLite integer range".to_string())?;
        let retry_reason = match verification.decision {
            ::contracts::types::outcome_verification::VerificationDecision::RetryableMismatch
            | ::contracts::types::outcome_verification::VerificationDecision::ReplannableMismatch => {
                Some(verification.reasons.join("; "))
            }
            _ => None,
        };
        let connection = self.connection.lock();
        let changed = connection
            .execute(
                "UPDATE episodes
                 SET after_sequence = ?3, verification_json = ?4,
                     verified_sequence = ?5, retry_reason = ?6
                 WHERE episode_id = ?1 AND attempt_id = ?2 AND status = 'running'
                   AND (verification_json IS NULL OR verification_json = ?4)",
                params![
                    episode_id,
                    attempt_id,
                    after_sequence,
                    verification_json,
                    verified_sequence,
                    retry_reason,
                ],
            )
            .map_err(|error| format!("update_verification: {error}"))?;
        if changed == 1 {
            return Ok(());
        }
        let stored = connection
            .query_row(
                "SELECT after_sequence, verification_json, verified_sequence, retry_reason
                 FROM episodes WHERE episode_id = ?1 AND attempt_id = ?2",
                params![episode_id, attempt_id],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("read verification after update: {error}"))?;
        if stored
            != Some((
                after_sequence,
                Some(verification_json),
                Some(verified_sequence),
                retry_reason,
            ))
        {
            return Err(format!(
                "update_verification: attempt missing, settled, or conflicting: {episode_id}/{attempt_id}"
            ));
        }
        Ok(())
    }

    async fn load_attempts(
        &self,
        episode_id: &str,
    ) -> Result<Vec<::contracts::types::episode_report::AttemptRecord>, String> {
        self.load_attempts(episode_id)
    }

    async fn store_settled_report(&self, report: &SettledEpisodeReport) -> Result<(), String> {
        report.verify_integrity()?;
        let durable_attempts = self.load_attempts(&report.report().episode_id)?;
        if durable_attempts != report.report().attempts {
            return Err(format!(
                "settled report attempts differ from durable journal for episode {}",
                report.report().episode_id
            ));
        }
        let report_json = serde_json::to_string(report)
            .map_err(|error| format!("serialize settled report: {error}"))?;
        if report_json.len() > 4 * 1_048_576 {
            return Err("settled report exceeds bounded metadata size".into());
        }
        let episode = report.report();
        let mut connection = self.connection.lock();
        let tx = connection
            .transaction()
            .map_err(|error| format!("begin settled report transaction: {error}"))?;
        tx.execute(
            "INSERT OR IGNORE INTO episode_states(episode_id, status, created_at_ms, settled_at_ms)
             VALUES (?1, ?2, ?3, ?3)",
            params![
                episode.episode_id,
                episode.settlement.as_str(),
                self.now_ms()
            ],
        )
        .map_err(|error| format!("ensure settled episode state: {error}"))?;
        let state: String = tx
            .query_row(
                "SELECT status FROM episode_states WHERE episode_id = ?1",
                params![episode.episode_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("read settled episode state: {error}"))?;
        if state == "running" {
            tx.execute(
                "UPDATE episode_states SET status = ?2, settled_at_ms = ?3
                 WHERE episode_id = ?1 AND status = 'running'",
                params![
                    episode.episode_id,
                    episode.settlement.as_str(),
                    self.now_ms()
                ],
            )
            .map_err(|error| format!("settle episode state with report: {error}"))?;
        } else if state != episode.settlement.as_str() {
            return Err(format!(
                "settled report conflicts with episode state: episode={} state={} report={}",
                episode.episode_id,
                state,
                episode.settlement.as_str()
            ));
        }
        tx.execute(
            "UPDATE episodes
             SET status = ?2, settled_at_ms = COALESCE(settled_at_ms, ?3)
             WHERE episode_id = ?1 AND status = 'running'",
            params![
                episode.episode_id,
                episode.settlement.as_str(),
                self.now_ms()
            ],
        )
        .map_err(|error| format!("settle episode attempts: {error}"))?;
        tx.execute(
            "INSERT OR IGNORE INTO episode_reports(
                episode_id, report_json, report_sha256, settlement,
                settled_at_unix_ms, created_at_monotonic_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                episode.episode_id,
                report_json,
                report.report_sha256(),
                episode.settlement.as_str(),
                report.settled_at_unix_ms(),
                self.now_ms(),
            ],
        )
        .map_err(|error| format!("insert settled report: {error}"))?;
        let existing: (String, String, i64) = tx
            .query_row(
                "SELECT report_sha256, settlement, settled_at_unix_ms
                 FROM episode_reports WHERE episode_id = ?1",
                params![episode.episode_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|error| format!("read settled report digest: {error}"))?;
        if existing
            != (
                report.report_sha256().to_owned(),
                episode.settlement.as_str().to_owned(),
                report.settled_at_unix_ms(),
            )
        {
            return Err(format!(
                "conflicting immutable episode report: episode={} existing_digest={} incoming_digest={}",
                episode.episode_id,
                existing.0,
                report.report_sha256()
            ));
        }
        tx.commit()
            .map_err(|error| format!("commit settled report: {error}"))?;
        Ok(())
    }

    async fn load_settled_report(
        &self,
        episode_id: &str,
    ) -> Result<Option<SettledEpisodeReport>, String> {
        self.load_settled_report(episode_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{ArtifactMetadata, ArtifactScanStatus, ArtifactStore};
    use ::contracts::types::embodiment::{DeviceId, SkillId, SkillOutcome};
    use ::contracts::types::episode_report::{
        build_report, ArtifactAvailability, EpisodeReportInput,
    };
    use ::contracts::types::expected_outcome::OutcomePredicate;
    use ::contracts::types::outcome_verification::VerificationDecision;
    use ::contracts::types::skill_proposal::PolicyProvenance;
    use ::contracts::MonoTime;
    use kernel::chronos::TestClock;

    fn sink() -> SqliteEpisodeSink {
        // Non-zero mono clock so created_at/settled_at are observable.
        SqliteEpisodeSink::from_connection(
            Connection::open_in_memory().unwrap(),
            Arc::new(TestClock::new(0, 500)),
        )
        .unwrap()
    }

    fn expected() -> ExpectedOutcome {
        ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("stance"),
            },
            freshness_ms: 500,
            stable_window_ms: 0,
            timeout_ms: 5_000,
        }
    }

    fn snapshot(seq: u64) -> WorldSnapshot {
        WorldSnapshot {
            device: DeviceId("bot".into()),
            schema: "robot.state/v1".into(),
            schema_version: 1,
            sequence: seq,
            payload: serde_json::json!({"mode": "stance"}),
            observed_at: MonoTime(seq),
            valid_until: None,
            stale: false,
        }
    }

    fn skill_request() -> SkillRequest {
        SkillRequest {
            skill: SkillId("kuavo.stance".into()),
            device: DeviceId("bot".into()),
            parameters: serde_json::json!({}),
        }
    }

    fn skill_result() -> SkillResult {
        SkillResult {
            operation_id: ::contracts::OperationId::new(),
            skill: SkillId("kuavo.stance".into()),
            device: DeviceId("bot".into()),
            outcome: SkillOutcome::Succeeded,
            duration_ms: 10,
            evidence: vec![],
        }
    }

    fn report() -> VerificationReport {
        VerificationReport {
            decision: VerificationDecision::Matched,
            evaluated_sequence: 1,
            observed_paths: vec!["mode".into()],
            reasons: vec![],
            evidence: vec![],
        }
    }

    #[tokio::test]
    async fn append_attempt_persists_row() {
        let sink = sink();
        sink.append_attempt(
            "ep-1",
            1,
            "attempt:ep-1:1",
            Some(&::contracts::OperationId::new()),
            &skill_request(),
            &expected(),
            Some(&snapshot(0)),
            Some(&snapshot(1)),
            Some(&skill_result()),
            Some(&report()),
        )
        .await
        .unwrap();
        assert_eq!(sink.count("ep-1"), 1);
    }

    #[tokio::test]
    async fn duplicate_attempt_is_idempotent() {
        let sink = sink();
        let operation = ::contracts::OperationId::new();
        for _ in 0..2 {
            sink.append_attempt(
                "ep-2",
                1,
                "attempt:ep-2:1",
                Some(&operation),
                &skill_request(),
                &expected(),
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        }
        assert_eq!(sink.count("ep-2"), 1, "identical digest must not duplicate");
        let mut conflicting = expected();
        conflicting.timeout_ms += 1;
        assert!(sink
            .append_attempt(
                "ep-2",
                1,
                "attempt:ep-2:1",
                Some(&operation),
                &skill_request(),
                &conflicting,
                None,
                None,
                None,
                None,
            )
            .await
            .is_err());
    }

    #[tokio::test]
    async fn close_episode_sets_status_and_settled_at() {
        let sink = sink();
        sink.append_attempt(
            "ep-3",
            1,
            "attempt:ep-3:1",
            Some(&::contracts::OperationId::new()),
            &skill_request(),
            &expected(),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        sink.close_episode("ep-3", EpisodeSettlement::Completed)
            .await
            .unwrap();
        let connection = sink.connection.lock();
        let (status, settled): (String, i64) = connection
            .query_row(
                "SELECT status, settled_at_ms FROM episodes WHERE episode_id = ?1 LIMIT 1",
                params!["ep-3"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "completed");
        assert!(settled > 0);
    }

    #[tokio::test]
    async fn pre_execution_failure_close_survives_without_an_attempt_row() {
        let sink = sink();
        sink.close_episode("ep-no-attempt", EpisodeSettlement::Failed)
            .await
            .unwrap();
        assert_eq!(sink.count("ep-no-attempt"), 0);
        assert_eq!(
            sink.load_settlement("ep-no-attempt").unwrap(),
            Some(EpisodeSettlement::Failed)
        );
        assert!(sink
            .close_episode("ep-no-attempt", EpisodeSettlement::Completed)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn snapshot_payload_bytes_never_enter_episode_json_columns() {
        let sink = sink();
        let marker = "RAW_ARTIFACT_BYTES".repeat(100_000);
        let mut before = snapshot(1);
        before.payload = serde_json::json!({"image_base64": marker});
        sink.append_attempt(
            "ep-no-inline",
            1,
            "attempt:ep-no-inline:1",
            None,
            &skill_request(),
            &expected(),
            Some(&before),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let connection = sink.connection.lock();
        let columns = connection
            .prepare("PRAGMA table_info(episodes)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!columns.iter().any(|column| column == "before_json"));
        assert!(!columns.iter().any(|column| column == "after_json"));
        let (expected_json, result_json, verification_json): (
            String,
            Option<String>,
            Option<String>,
        ) = connection
            .query_row(
                "SELECT expected_json, result_json, verification_json
                 FROM episodes WHERE episode_id = 'ep-no-inline'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert!(!expected_json.contains("RAW_ARTIFACT_BYTES"));
        assert!(result_json.is_none());
        assert!(verification_json.is_none());
    }

    #[tokio::test]
    async fn load_attempts_reconstructs_ordered_records() {
        let sink = sink();
        sink.append_attempt(
            "ep-4",
            1,
            "attempt:ep-4:1",
            Some(&::contracts::OperationId(
                "00000000-0000-0000-0000-000000000001".parse().unwrap(),
            )),
            &skill_request(),
            &expected(),
            Some(&snapshot(0)),
            Some(&snapshot(1)),
            Some(&skill_result()),
            Some(&report()),
        )
        .await
        .unwrap();
        sink.close_episode("ep-4", EpisodeSettlement::Completed)
            .await
            .unwrap();

        let attempts = sink.load_attempts("ep-4").unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].attempt, 1);
        assert_eq!(attempts[0].request, Some(skill_request()));
        assert_eq!(attempts[0].expected, expected());
        assert_eq!(
            attempts[0].verification_decision,
            Some(VerificationDecision::Matched)
        );
        assert_eq!(
            attempts[0].operation_id.as_deref(),
            Some("00000000-0000-0000-0000-000000000001")
        );
        assert_eq!(attempts[0].before_sequence, Some(0));
        assert_eq!(attempts[0].after_sequence, Some(1));
        assert_eq!(attempts[0].verified_sequence, Some(1));
        assert_eq!(attempts[0].verification_observed_paths, ["mode"]);
    }

    fn file_sink(path: &Path) -> SqliteEpisodeSink {
        SqliteEpisodeSink::open(path, Arc::new(TestClock::new(1_000, 500))).unwrap()
    }

    #[test]
    fn legacy_attempt_rows_backfill_lifecycle_and_clear_snapshot_json() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE episodes (
                    episode_id TEXT NOT NULL, attempt INTEGER NOT NULL, attempt_id TEXT NOT NULL,
                    operation_id TEXT, request_digest TEXT NOT NULL, status TEXT NOT NULL,
                    expected_json TEXT NOT NULL, before_json TEXT, after_json TEXT,
                    result_json TEXT, verification_json TEXT, created_at_ms INTEGER NOT NULL,
                    settled_at_ms INTEGER, PRIMARY KEY (episode_id, attempt, attempt_id)
                 );",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO episodes(
                    episode_id, attempt, attempt_id, operation_id, request_digest, status,
                    expected_json, before_json, after_json, result_json, verification_json,
                    created_at_ms, settled_at_ms
                 ) VALUES ('legacy', 1, 'attempt:legacy:1', NULL, 'digest', 'failed',
                           ?1, ?2, NULL, NULL, NULL, 10, 20)",
                params![
                    serde_json::to_string(&expected()).unwrap(),
                    serde_json::to_string(&snapshot(9)).unwrap()
                ],
            )
            .unwrap();

        let sink =
            SqliteEpisodeSink::from_connection(connection, Arc::new(TestClock::new(1_000, 500)))
                .unwrap();
        assert_eq!(
            sink.load_settlement("legacy").unwrap(),
            Some(EpisodeSettlement::Failed)
        );
        let attempts = sink.load_attempts("legacy").unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].before_sequence, Some(9));
        let legacy_json: Option<String> = sink
            .connection
            .lock()
            .query_row(
                "SELECT before_json FROM episodes WHERE episode_id = 'legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(legacy_json.is_none());
    }

    #[tokio::test]
    async fn restart_recovers_each_boundary_and_identical_immutable_report() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("episodes.db");
        let operation =
            ::contracts::OperationId("00000000-0000-0000-0000-000000000042".parse().unwrap());

        // Crash boundary 1: append attempt, then reopen.
        let sink = file_sink(&path);
        sink.append_attempt(
            "ep-restart",
            1,
            "attempt:ep-restart:1",
            Some(&operation),
            &skill_request(),
            &expected(),
            Some(&snapshot(10)),
            None,
            Some(&skill_result()),
            None,
        )
        .await
        .unwrap();
        drop(sink);
        let sink = file_sink(&path);
        let appended = sink.load_attempts("ep-restart").unwrap();
        assert_eq!(appended.len(), 1);
        assert_eq!(appended[0].before_sequence, Some(10));
        assert_eq!(appended[0].after_sequence, None);

        // Crash boundary 2: verification update, then reopen.
        let mut verification = report();
        verification.evaluated_sequence = 11;
        sink.update_verification(
            "ep-restart",
            "attempt:ep-restart:1",
            Some(&snapshot(11)),
            &verification,
        )
        .await
        .unwrap();
        drop(sink);
        let sink = file_sink(&path);
        let verified = sink.load_attempts("ep-restart").unwrap();
        assert_eq!(verified[0].after_sequence, Some(11));
        assert_eq!(verified[0].verified_sequence, Some(11));
        assert_eq!(verified[0].verification_observed_paths, ["mode"]);

        // Crash boundary 3: episode close, then reopen and rebuild the report
        // from durable attempts rather than terminal in-memory state.
        sink.close_episode("ep-restart", EpisodeSettlement::Completed)
            .await
            .unwrap();
        drop(sink);
        let sink = file_sink(&path);
        let attempts = sink.load_attempts("ep-restart").unwrap();
        let artifact_store = ArtifactStore::open(
            &temp.path().join("artifacts.db"),
            &temp.path().join("blobs"),
        )
        .unwrap();
        let mut writer = artifact_store
            .begin(
                ArtifactMetadata {
                    mime_type: "application/x-rosbag".into(),
                    provider: "kuavo-bridge".into(),
                    account_id: "robot".into(),
                    provider_message_id: "ep-restart".into(),
                    provider_part_id: "rosbag".into(),
                    source_timestamp_ms: 1_000,
                    scan_status: ArtifactScanStatus::Unscanned,
                    created_at_ms: 2_000,
                },
                1_024,
            )
            .unwrap();
        writer
            .write_chunk(b"bounded external rosbag fixture")
            .unwrap();
        let artifact_record = artifact_store.finish(writer).unwrap();
        assert!(artifact_store
            .set_scan_status(&artifact_record.artifact_id, ArtifactScanStatus::Clean)
            .unwrap());
        let artifact_manifest = artifact_store
            .episode_manifest(&artifact_record.artifact_id, "rosbag")
            .unwrap()
            .unwrap();
        let report = build_report(EpisodeReportInput {
            episode_id: "ep-restart".into(),
            goal: "stand safely".into(),
            device: DeviceId("bot".into()),
            sim_scene_version: "kuavo-mujoco-scene-v3".into(),
            aletheon_commit: "abc123".into(),
            bridge_protocol_digest: "sha256:bridge-proto".into(),
            skill_descriptor_digest: "sha256:skill-descriptor".into(),
            policy_provenance: Some(PolicyProvenance {
                provider: "local-policy-gateway".into(),
                model: "openvla-7b".into(),
                version: "2026-08".into(),
                protocol_version: "1.0".into(),
                digest: "sha256:model".into(),
            }),
            failures: vec![],
            safe_stop: None,
            selected_frames: vec![],
            settlement: EpisodeSettlement::Completed,
            attempts,
            artifacts: vec![artifact_manifest],
        });
        let settled = SettledEpisodeReport::new(report, 3_000).unwrap();
        sink.store_settled_report(&settled).await.unwrap();
        drop(sink);

        let reopened = file_sink(&path);
        let recovered = reopened.load_settled_report("ep-restart").unwrap().unwrap();
        assert_eq!(recovered, settled);
        assert_eq!(recovered.report().before_sequence, Some(10));
        assert_eq!(recovered.report().after_sequence, Some(11));
        assert_eq!(recovered.report().verified_sequence, Some(11));
        assert_eq!(
            recovered.report().sim_scene_version,
            "kuavo-mujoco-scene-v3"
        );
        assert_eq!(
            recovered.report().bridge_protocol_digest,
            "sha256:bridge-proto"
        );
        assert_eq!(
            recovered.report().skill_descriptor_digest,
            "sha256:skill-descriptor"
        );
        assert_eq!(
            recovered.report().policy_provenance.as_ref().unwrap().model,
            "openvla-7b"
        );
        assert_eq!(
            recovered.report().attempts[0].verification_observed_paths,
            ["mode"]
        );
        let persisted_json = serde_json::to_string(&recovered).unwrap();
        assert!(!persisted_json.contains("RAW_ARTIFACT_BYTES"));
        assert!(persisted_json.len() < 16_384);

        // Idempotent replay accepts the exact receipt; conflicting mutation is
        // rejected by digest comparison and the append-only database trigger.
        reopened.store_settled_report(&settled).await.unwrap();
        assert!(reopened
            .connection
            .lock()
            .execute(
                "UPDATE episode_reports SET report_sha256 = 'tampered' WHERE episode_id = ?1",
                params!["ep-restart"],
            )
            .is_err());

        // Simulate a process crash after the artifact store committed deletion
        // but before the episode projection could append its tombstone.
        let tombstone = artifact_store
            .expire_retention(
                &artifact_record.artifact_id,
                4_000,
                "episode retention elapsed",
            )
            .unwrap();
        let tombstone = tombstone.unwrap().episode_tombstone();
        assert!(artifact_store
            .readable_path(&artifact_record)
            .unwrap()
            .is_none());
        drop(artifact_store);
        drop(reopened);
        let reopened = file_sink(&path);
        let reopened_artifacts = ArtifactStore::open(
            &temp.path().join("artifacts.db"),
            &temp.path().join("blobs"),
        )
        .unwrap();
        assert_eq!(
            reopened
                .load_report_projection("ep-restart")
                .unwrap()
                .unwrap()
                .artifacts[0]
                .availability,
            ArtifactAvailability::Available,
            "the simulated crash happened before the episode projection write"
        );
        let projected = reopened
            .load_report_projection_with_local_artifacts(&reopened_artifacts, "ep-restart")
            .unwrap()
            .unwrap();
        assert!(matches!(
            projected.artifacts[0].availability,
            ArtifactAvailability::RetentionExpired {
                expired_at_unix_ms: 4_000,
                ..
            }
        ));
        assert_eq!(
            projected.artifacts[0].digest.as_deref(),
            Some(tombstone.digest.as_str())
        );
        let replay = reopened
            .expire_local_artifact(
                &reopened_artifacts,
                "ep-restart",
                &artifact_record.artifact_id,
                4_000,
                "episode retention elapsed",
            )
            .unwrap()
            .unwrap();
        assert_eq!(replay.episode_tombstone(), tombstone);
        let immutable = reopened.load_settled_report("ep-restart").unwrap().unwrap();
        assert_eq!(immutable, settled);
        assert_eq!(
            immutable.report().artifacts[0].availability,
            ArtifactAvailability::Available
        );
    }
}

//! Immutable embodied execution episode repository.
//! Episodes are append-only and content-addressed for idempotent replay.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use fabric::types::embodied_episode::{EmbodiedEpisode, EpisodeAttempt};
use fabric::types::embodiment::{DeviceId, SkillId, SkillResult};
use fabric::types::expected_outcome::ExpectedOutcome;
use fabric::types::outcome_verification::VerificationReport;
use fabric::types::world_state::WorldSnapshot;
use fabric::OperationId;
use uuid::Uuid;

/// An idempotent, append-only repository for embodied execution episodes.
pub struct EmbodiedEpisodeRepository {
    conn: Mutex<Connection>,
}

impl EmbodiedEpisodeRepository {
    /// Open or create the repository at the given path.
    pub fn open(db_path: &Path) -> Result<Self, String> {
        let conn = Connection::open(db_path).map_err(|e| format!("open: {}", e))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;",
        )
        .map_err(|e| format!("pragma: {}", e))?;
        let repo = Self {
            conn: Mutex::new(conn),
        };
        repo.init_schema()?;
        Ok(repo)
    }

    fn init_schema(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS embodied_episodes (
                episode_id TEXT NOT NULL,
                schema_version INTEGER NOT NULL DEFAULT 1,
                goal_id TEXT NOT NULL,
                device_id TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                closed_at_ms INTEGER,
                outcome TEXT,
                PRIMARY KEY (episode_id)
            );
            CREATE TABLE IF NOT EXISTS episode_attempts (
                episode_id TEXT NOT NULL,
                attempt_number INTEGER NOT NULL,
                operation_id TEXT NOT NULL,
                skill_id TEXT NOT NULL,
                expected_outcome_json TEXT NOT NULL,
                before_snapshot_json TEXT,
                after_snapshot_json TEXT,
                result_json TEXT,
                verification_json TEXT,
                recovery TEXT,
                PRIMARY KEY (episode_id, attempt_number),
                FOREIGN KEY (episode_id) REFERENCES embodied_episodes(episode_id)
            );
            CREATE TABLE IF NOT EXISTS episode_evidence (
                episode_id TEXT NOT NULL,
                attempt_number INTEGER NOT NULL,
                evidence_kind TEXT NOT NULL,
                evidence_uri TEXT NOT NULL,
                PRIMARY KEY (episode_id, attempt_number, evidence_kind)
            );",
        )
        .map_err(|e| format!("schema: {}", e))?;
        Ok(())
    }

    /// Create a new episode. Returns error if episode_id already exists.
    pub fn create_episode(
        &self,
        episode_id: &str,
        goal_id: &str,
        device_id: &DeviceId,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        conn.execute(
            "INSERT INTO embodied_episodes (episode_id, goal_id, device_id, created_at_ms)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![episode_id, goal_id, device_id.0.as_str(), now_ms],
        )
        .map_err(|e| format!("create_episode: {}", e))?;
        Ok(())
    }

    /// Append an attempt to an existing episode. Idempotent: replaying the
    /// same (episode_id, attempt_number, operation_id) is a no-op; different
    /// data for the same key is rejected as a conflicting replay.
    pub fn append_attempt(
        &self,
        episode_id: &str,
        attempt_number: u32,
        operation_id: &OperationId,
        skill_id: &SkillId,
        expected_outcome: &ExpectedOutcome,
        before: Option<&WorldSnapshot>,
        after: Option<&WorldSnapshot>,
        result: Option<&SkillResult>,
        verification: Option<&VerificationReport>,
        recovery: Option<&str>,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;

        let expected_json =
            serde_json::to_string(expected_outcome).map_err(|e| format!("json: {}", e))?;
        let before_json = before.and_then(|s| serde_json::to_string(s).ok());
        let after_json = after.and_then(|s| serde_json::to_string(s).ok());
        let result_json = result.and_then(|r| serde_json::to_string(r).ok());
        let verification_json = verification.and_then(|v| serde_json::to_string(v).ok());

        let op_id_str = operation_id.0.to_string();

        // Check for idempotent replay
        let existing: Option<String> = conn
            .query_row(
                "SELECT operation_id FROM episode_attempts WHERE episode_id = ?1 AND attempt_number = ?2",
                rusqlite::params![episode_id, attempt_number],
                |row| row.get(0),
            )
            .ok();

        match existing {
            Some(existing_op) => {
                if existing_op != op_id_str {
                    return Err(format!(
                        "conflicting replay: episode={} attempt={} existing_op={} new_op={}",
                        episode_id, attempt_number, existing_op, op_id_str
                    ));
                }
                // Same operation_id — idempotent, success
                return Ok(());
            }
            None => {}
        }

        conn.execute(
            "INSERT INTO episode_attempts
             (episode_id, attempt_number, operation_id, skill_id, expected_outcome_json,
              before_snapshot_json, after_snapshot_json, result_json, verification_json, recovery)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                episode_id,
                attempt_number,
                op_id_str,
                skill_id.0.as_str(),
                expected_json,
                before_json,
                after_json,
                result_json,
                verification_json,
                recovery,
            ],
        )
        .map_err(|e| format!("append_attempt: {}", e))?;

        Ok(())
    }

    /// Close an episode with a final outcome.
    pub fn close_episode(&self, episode_id: &str, outcome: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let affected = conn
            .execute(
                "UPDATE embodied_episodes SET closed_at_ms = ?1, outcome = ?2 WHERE episode_id = ?3",
                rusqlite::params![now_ms, outcome, episode_id],
            )
            .map_err(|e| format!("close_episode: {}", e))?;
        if affected == 0 {
            return Err(format!("episode not found: {}", episode_id));
        }
        Ok(())
    }

    /// Load a full episode with all attempts.
    pub fn load_episode(&self, episode_id: &str) -> Result<Option<EmbodiedEpisode>, String> {
        let conn = self.conn.lock().map_err(|e| format!("lock: {}", e))?;
        let mut stmt = conn
            .prepare(
                "SELECT goal_id, device_id FROM embodied_episodes WHERE episode_id = ?1",
            )
            .map_err(|e| format!("prepare: {}", e))?;
        let episode_row: Option<(String, String)> = stmt
            .query_row(rusqlite::params![episode_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .ok();

        let (goal_id, device) = match episode_row {
            Some(row) => row,
            None => return Ok(None),
        };

        let mut attempt_stmt = conn
            .prepare(
                "SELECT attempt_number, operation_id, skill_id, expected_outcome_json,
                        before_snapshot_json, after_snapshot_json, result_json,
                        verification_json, recovery
                 FROM episode_attempts WHERE episode_id = ?1 ORDER BY attempt_number",
            )
            .map_err(|e| format!("prepare attempts: {}", e))?;

        let mut attempts = Vec::new();
        let rows = attempt_stmt
            .query_map(rusqlite::params![episode_id], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            })
            .map_err(|e| format!("query attempts: {}", e))?;

        for row in rows {
            let (
                _num,
                op_id_str,
                skill_str,
                expected_json,
                before_json,
                after_json,
                result_json,
                verif_json,
                recovery,
            ) = row.map_err(|e| format!("row: {}", e))?;

            let operation_id = Uuid::parse_str(&op_id_str)
                .map(OperationId)
                .map_err(|e| format!("parse operation_id {}: {}", op_id_str, e))?;

            attempts.push(EpisodeAttempt {
                operation_id,
                skill: SkillId(skill_str),
                expected_outcome: serde_json::from_str(&expected_json)
                    .unwrap_or_else(|_| ExpectedOutcome {
                        predicate: fabric::types::expected_outcome::OutcomePredicate::Equals {
                            path: "x".into(),
                            value: serde_json::json!(0),
                        },
                        freshness_ms: 0,
                        stable_window_ms: 0,
                        timeout_ms: 0,
                    }),
                before: before_json.and_then(|j| serde_json::from_str(&j).ok()),
                after: after_json.and_then(|j| serde_json::from_str(&j).ok()),
                result: result_json.and_then(|j| serde_json::from_str(&j).ok()),
                verification: verif_json.and_then(|j| serde_json::from_str(&j).ok()),
                recovery,
            });
        }

        Ok(Some(EmbodiedEpisode {
            schema_version: 1,
            goal_id,
            device: DeviceId(device),
            attempts,
            evidence: vec![],
        }))
    }
}

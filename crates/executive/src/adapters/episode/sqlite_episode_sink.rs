//! Durable SQLite `EpisodeSink` for embodied robot episodes.
//!
//! Each attempt is persisted with a request digest for idempotent replay:
//! `INSERT OR IGNORE` on `(episode_id, attempt, request_digest)` means a
//! restarted run that re-records an identical attempt is a no-op instead of a
//! duplicate side effect.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use cognit::harness::robot::EpisodeSink;
use fabric::types::embodiment::SkillResult;
use fabric::types::expected_outcome::ExpectedOutcome;
use fabric::types::outcome_verification::VerificationReport;
use fabric::types::world_state::WorldSnapshot;
use fabric::Clock;
use parking_lot::Mutex;
use rusqlite::{params, Connection};
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
            .execute_batch(MIGRATION)
            .map_err(|error| format!("migrate: {error}"))?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            clock,
        })
    }

    fn now_ms(&self) -> i64 {
        self.clock.mono_now().0 as i64
    }

    /// Stable digest identifying one (episode, attempt, operation, expected)
    /// unit — used for idempotent replay.
    fn request_digest(
        episode_id: &str,
        attempt: u32,
        operation_id: &str,
        expected: &ExpectedOutcome,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(episode_id.as_bytes());
        hasher.update(attempt.to_be_bytes());
        hasher.update(operation_id.as_bytes());
        hasher.update(serde_json::to_string(expected).unwrap_or_default().as_bytes());
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
}

#[async_trait]
impl EpisodeSink for SqliteEpisodeSink {
    async fn append_attempt(
        &self,
        episode_id: &str,
        attempt: u32,
        operation_id: &str,
        expected: &ExpectedOutcome,
        before: Option<&WorldSnapshot>,
        after: Option<&WorldSnapshot>,
        result: Option<&SkillResult>,
        verification: Option<&VerificationReport>,
    ) -> Result<(), String> {
        let digest = Self::request_digest(episode_id, attempt, operation_id, expected);
        let expected_json =
            serde_json::to_string(expected).map_err(|e| format!("expected serde: {e}"))?;
        let before_json = before
            .map(|snapshot| serde_json::to_string(snapshot))
            .transpose()
            .map_err(|e| format!("before serde: {e}"))?;
        let after_json = after
            .map(|snapshot| serde_json::to_string(snapshot))
            .transpose()
            .map_err(|e| format!("after serde: {e}"))?;
        let result_json = result
            .map(|snapshot| serde_json::to_string(snapshot))
            .transpose()
            .map_err(|e| format!("result serde: {e}"))?;
        let verification_json = verification
            .map(|report| serde_json::to_string(report))
            .transpose()
            .map_err(|e| format!("verification serde: {e}"))?;

        self.connection
            .lock()
            .execute(
                "INSERT OR IGNORE INTO episodes
                 (episode_id, attempt, operation_id, request_digest, status, expected_json,
                  before_json, after_json, result_json, verification_json, created_at_ms, settled_at_ms)
                 VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6, ?7, ?8, ?9, ?10, NULL)",
                params![
                    episode_id,
                    attempt as i64,
                    operation_id,
                    digest,
                    expected_json,
                    before_json.as_deref(),
                    after_json.as_deref(),
                    result_json.as_deref(),
                    verification_json.as_deref(),
                    self.now_ms(),
                ],
            )
            .map_err(|error| format!("append_attempt: {error}"))?;
        Ok(())
    }

    async fn close_episode(&self, episode_id: &str, outcome: &str) -> Result<(), String> {
        self.connection
            .lock()
            .execute(
                "UPDATE episodes SET status = ?2, settled_at_ms = ?3 WHERE episode_id = ?1",
                params![episode_id, outcome, self.now_ms()],
            )
            .map_err(|error| format!("close_episode: {error}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::types::embodiment::{DeviceId, SkillId, SkillOutcome};
    use fabric::types::expected_outcome::OutcomePredicate;
    use fabric::types::outcome_verification::VerificationDecision;
    use fabric::MonoTime;
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
            sequence: seq,
            payload: serde_json::json!({"mode": "stance"}),
            observed_at: MonoTime(seq),
            stale: false,
        }
    }

    fn skill_result() -> SkillResult {
        SkillResult {
            operation_id: fabric::OperationId::new(),
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
            observed_paths: vec![],
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
            "op-1",
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
        for _ in 0..2 {
            sink.append_attempt("ep-2", 1, "op-1", &expected(), None, None, None, None)
                .await
                .unwrap();
        }
        assert_eq!(sink.count("ep-2"), 1, "identical digest must not duplicate");
    }

    #[tokio::test]
    async fn close_episode_sets_status_and_settled_at() {
        let sink = sink();
        sink.append_attempt("ep-3", 1, "op-1", &expected(), None, None, None, None)
            .await
            .unwrap();
        sink.close_episode("ep-3", "completed").await.unwrap();
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
}

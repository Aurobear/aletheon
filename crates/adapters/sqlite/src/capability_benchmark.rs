//! SQLite persistence for the Application capability-rollup projection.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use application::capability_benchmark::{
    CapabilityReceiptRollup, CapabilityRollupKey,
    CapabilityRollupProjectionSink as InMemoryCapabilityRollupProjection,
    CapabilitySelectionObservation,
};
use application::evaluation_projection::{EvaluationProjectionRecord, EvaluationProjectionSink};
use async_trait::async_trait;
use rusqlite::{params, Connection};

/// Durable SQLite adapter around Application's I/O-free rollup projection.
pub struct SqliteCapabilityRollupProjectionSink {
    projection: InMemoryCapabilityRollupProjection,
    durable: Option<Mutex<Connection>>,
}

impl Default for SqliteCapabilityRollupProjectionSink {
    fn default() -> Self {
        Self {
            projection: InMemoryCapabilityRollupProjection::default(),
            durable: None,
        }
    }
}

impl SqliteCapabilityRollupProjectionSink {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS evaluation_rollup_inputs (
               receipt_id TEXT PRIMARY KEY NOT NULL,
               record_json TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL
             );",
        )?;
        let records = {
            let mut statement = connection.prepare(
                "SELECT record_json FROM evaluation_rollup_inputs
                 ORDER BY created_at_ms, receipt_id",
            )?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let sink = Self {
            projection: InMemoryCapabilityRollupProjection::default(),
            durable: Some(Mutex::new(connection)),
        };
        for encoded in records {
            let record = serde_json::from_str::<EvaluationProjectionRecord>(&encoded)?;
            sink.projection.observe_record(&record);
        }
        Ok(sink)
    }

    pub fn snapshot(&self) -> HashMap<CapabilityRollupKey, CapabilityReceiptRollup> {
        self.projection.snapshot()
    }

    pub fn selection_input(&self, profile_id: &str) -> Vec<CapabilitySelectionObservation> {
        self.projection.selection_input(profile_id)
    }

    pub fn preferred_runtime<'a>(
        &self,
        profile_id: &str,
        eligible: impl IntoIterator<Item = &'a str>,
    ) -> Option<String> {
        self.projection.preferred_runtime(profile_id, eligible)
    }

    /// Return exact durable evidence records rather than a lossy rollup.
    pub fn evidence_for_session(
        &self,
        session_id: &str,
        rubric_id: &str,
    ) -> anyhow::Result<Vec<EvaluationProjectionRecord>> {
        let Some(connection) = &self.durable else {
            return Ok(Vec::new());
        };
        let connection = connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut statement = connection.prepare(
            "SELECT record_json FROM evaluation_rollup_inputs ORDER BY created_at_ms, receipt_id",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut records = Vec::new();
        for encoded in rows {
            let record: EvaluationProjectionRecord = serde_json::from_str(&encoded?)?;
            if record.context.session_id == session_id && record.context.rubric_id == rubric_id {
                records.push(record);
            }
        }
        Ok(records)
    }
}

impl runtime::RuntimePreferenceHistory for SqliteCapabilityRollupProjectionSink {
    fn preferred_runtime(&self, profile_id: &str, eligible: &[String]) -> Option<String> {
        self.preferred_runtime(profile_id, eligible.iter().map(String::as_str))
    }
}

#[async_trait]
impl EvaluationProjectionSink for SqliteCapabilityRollupProjectionSink {
    fn name(&self) -> &'static str {
        "capability_rollup"
    }

    async fn project(&self, record: &EvaluationProjectionRecord) -> anyhow::Result<()> {
        if let Some(connection) = &self.durable {
            let encoded = serde_json::to_string(record)?;
            let inserted = connection
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .execute(
                    "INSERT OR IGNORE INTO evaluation_rollup_inputs
                     (receipt_id, record_json, created_at_ms) VALUES (?1, ?2, ?3)",
                    params![
                        record.receipt.receipt_id.0.to_string(),
                        encoded,
                        record.receipt.created_at_ms
                    ],
                )?;
            if inserted == 0 {
                return Ok(());
            }
        }
        self.projection.observe_record(record);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::evaluation_projection::{
        EvaluationProjectionContext, EvaluationProjectionMetrics,
    };
    use contracts::{
        EvaluationContractId, EvaluationDecision, EvaluationReceiptId, EvaluationReceiptRef,
        ProcessId, TurnId, EVALUATION_SCHEMA_V1,
    };

    fn record() -> EvaluationProjectionRecord {
        EvaluationProjectionRecord {
            receipt: EvaluationReceiptRef {
                schema_version: EVALUATION_SCHEMA_V1,
                receipt_id: EvaluationReceiptId::new(),
                contract_id: EvaluationContractId::new(),
                subject_kind: "turn".into(),
                subject_id: TurnId::new().0.to_string(),
                decision: EvaluationDecision::ObservedPass,
                weighted_total_millis: Some(82_000),
                evidence_coverage_millis: 750,
                confidence_millis: 880,
                failed_gates: Vec::new(),
                created_at_ms: 1,
            },
            context: EvaluationProjectionContext {
                session_id: "session".into(),
                runtime_id: "native".into(),
                profile_id: "code-agent".into(),
                effective_model_id: "provider/model".into(),
                model_display_name: "model".into(),
                workspace_boundary_sha256: "workspace-digest".into(),
                verification_selection_sha256: "verification-digest".into(),
                rubric_id: "coding-v2".into(),
                rubric_version: 2,
                process_id: ProcessId::new(),
                metrics: EvaluationProjectionMetrics {
                    inference_rounds: Some(3),
                    provider_retries: Some(1),
                    tool_calls: Some(5),
                    ..Default::default()
                },
            },
        }
    }

    #[tokio::test]
    async fn durable_projection_is_idempotent_and_replays_after_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capability-rollup.sqlite3");
        let record = record();

        let sink = SqliteCapabilityRollupProjectionSink::open(&path).unwrap();
        sink.project(&record).await.unwrap();
        sink.project(&record).await.unwrap();
        drop(sink);

        let reopened = SqliteCapabilityRollupProjectionSink::open(&path).unwrap();
        let rollup = reopened.snapshot().into_values().next().unwrap();
        assert_eq!(rollup.receipt_count, 1);
        assert_eq!(rollup.pass_count, 1);
        assert_eq!(rollup.inference_rounds.total, 3);
        assert_eq!(rollup.provider_retries.total, 1);
        assert_eq!(rollup.tool_calls.total, 5);
        assert_eq!(
            reopened
                .evidence_for_session("session", "coding-v2")
                .unwrap(),
            vec![record]
        );
    }
}

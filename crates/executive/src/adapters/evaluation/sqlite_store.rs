//! Durable authority for evaluation contracts, evidence snapshots, and receipts.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use async_trait::async_trait;
use fabric::{
    EvaluationEvidenceSnapshot, EvaluationReceipt, EvaluationReceiptId, EvaluationSnapshotId,
    EvaluationSubject, TaskEvaluationContract,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

use crate::application::evaluation::EvaluationReceiptStore;

pub struct SqliteEvaluationStore {
    connection: Mutex<Connection>,
}

impl SqliteEvaluationStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path).context("open evaluation sqlite")?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS evaluation_contracts (
               contract_id TEXT PRIMARY KEY,
               subject_kind TEXT NOT NULL,
               subject_id TEXT NOT NULL,
               mode TEXT NOT NULL,
               rubric_id TEXT NOT NULL,
               rubric_version INTEGER NOT NULL,
               body_json TEXT NOT NULL,
               body_sha256 TEXT NOT NULL,
               issued_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS evaluation_evidence (
               evidence_id TEXT PRIMARY KEY,
               contract_id TEXT NOT NULL REFERENCES evaluation_contracts(contract_id),
               kind TEXT NOT NULL,
               trust TEXT NOT NULL,
               producer TEXT NOT NULL,
               payload_json TEXT NOT NULL,
               body_json TEXT NOT NULL,
               body_sha256 TEXT NOT NULL,
               sha256 TEXT NOT NULL,
               captured_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS evaluation_snapshots (
               snapshot_id TEXT PRIMARY KEY,
               contract_id TEXT NOT NULL REFERENCES evaluation_contracts(contract_id),
               snapshot_sha256 TEXT NOT NULL,
               ordered_evidence_ids_json TEXT NOT NULL,
               body_json TEXT NOT NULL,
               body_sha256 TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS evaluation_receipts (
               receipt_id TEXT PRIMARY KEY,
               contract_id TEXT NOT NULL REFERENCES evaluation_contracts(contract_id),
               operation_id TEXT NOT NULL,
               decision TEXT NOT NULL,
               score_millis INTEGER,
               coverage_millis INTEGER NOT NULL,
               confidence_millis INTEGER NOT NULL,
               body_json TEXT NOT NULL,
               body_sha256 TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS evaluation_contract_subject
               ON evaluation_contracts(subject_kind, subject_id, issued_at_ms DESC);
             CREATE INDEX IF NOT EXISTS evaluation_contract_rubric
               ON evaluation_contracts(rubric_id, rubric_version, issued_at_ms DESC);
             CREATE INDEX IF NOT EXISTS evaluation_receipt_contract_time
               ON evaluation_receipts(contract_id, created_at_ms DESC);
             CREATE INDEX IF NOT EXISTS evaluation_receipt_time
               ON evaluation_receipts(created_at_ms DESC);",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn load_contract(
        transaction: &Transaction<'_>,
        contract_id: &str,
    ) -> Result<TaskEvaluationContract> {
        let body: String = transaction
            .query_row(
                "SELECT body_json FROM evaluation_contracts WHERE contract_id = ?1",
                params![contract_id],
                |row| row.get(0),
            )
            .context("evaluation contract was not persisted before evidence")?;
        serde_json::from_str(&body).context("decode persisted evaluation contract")
    }

    fn insert_idempotent(
        transaction: &Transaction<'_>,
        insert_sql: &str,
        insert_params: impl rusqlite::Params,
        digest_sql: &str,
        id: &str,
        expected_digest: &str,
        entity: &str,
    ) -> Result<()> {
        transaction.execute(insert_sql, insert_params)?;
        let stored: String = transaction.query_row(digest_sql, params![id], |row| row.get(0))?;
        anyhow::ensure!(
            stored == expected_digest,
            "conflicting {entity} body for id {id}"
        );
        Ok(())
    }
}

#[async_trait]
impl EvaluationReceiptStore for SqliteEvaluationStore {
    async fn append_contract(&self, contract: &TaskEvaluationContract) -> Result<()> {
        contract.validate()?;
        let body = serde_json::to_string(contract)?;
        let body_sha = digest(body.as_bytes());
        let (subject_kind, subject_id) = contract.subject.kind_and_id();
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        connection.execute(
            "INSERT INTO evaluation_contracts
             (contract_id, subject_kind, subject_id, mode, rubric_id, rubric_version,
              body_json, body_sha256, issued_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(contract_id) DO NOTHING",
            params![
                contract.contract_id.0.to_string(),
                subject_kind,
                subject_id,
                enum_text(&contract.mode)?,
                contract.rubric.0,
                contract.rubric_version,
                body,
                body_sha,
                contract.issued_at_ms,
            ],
        )?;
        let stored: String = connection.query_row(
            "SELECT body_sha256 FROM evaluation_contracts WHERE contract_id = ?1",
            params![contract.contract_id.0.to_string()],
            |row| row.get(0),
        )?;
        anyhow::ensure!(stored == body_sha, "conflicting evaluation contract body");
        Ok(())
    }

    async fn append_evaluation(
        &self,
        snapshot: &EvaluationEvidenceSnapshot,
        receipt: &EvaluationReceipt,
    ) -> Result<()> {
        snapshot.validate()?;
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let contract_id = snapshot.contract_id.0.to_string();
        let contract = Self::load_contract(&transaction, &contract_id)?;
        receipt.validate(&contract, snapshot)?;

        for item in &snapshot.evidence {
            let body = serde_json::to_string(item)?;
            let body_sha = digest(body.as_bytes());
            Self::insert_idempotent(
                &transaction,
                "INSERT INTO evaluation_evidence
                 (evidence_id, contract_id, kind, trust, producer, payload_json,
                  body_json, body_sha256, sha256, captured_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(evidence_id) DO NOTHING",
                params![
                    item.evidence_id.0,
                    contract_id,
                    enum_text(&item.kind)?,
                    enum_text(&item.trust)?,
                    item.producer,
                    serde_json::to_string(&item.payload)?,
                    body,
                    body_sha,
                    item.sha256,
                    item.captured_at_ms,
                ],
                "SELECT body_sha256 FROM evaluation_evidence WHERE evidence_id = ?1",
                &item.evidence_id.0,
                &body_sha,
                "evaluation evidence",
            )?;
        }

        let snapshot_body = serde_json::to_string(snapshot)?;
        let snapshot_body_sha = digest(snapshot_body.as_bytes());
        let ordered_ids = snapshot
            .evidence
            .iter()
            .map(|item| item.evidence_id.0.as_str())
            .collect::<Vec<_>>();
        Self::insert_idempotent(
            &transaction,
            "INSERT INTO evaluation_snapshots
             (snapshot_id, contract_id, snapshot_sha256, ordered_evidence_ids_json,
              body_json, body_sha256, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(snapshot_id) DO NOTHING",
            params![
                snapshot.snapshot_id.0.to_string(),
                contract_id,
                snapshot.sha256,
                serde_json::to_string(&ordered_ids)?,
                snapshot_body,
                snapshot_body_sha,
                snapshot.created_at_ms,
            ],
            "SELECT body_sha256 FROM evaluation_snapshots WHERE snapshot_id = ?1",
            &snapshot.snapshot_id.0.to_string(),
            &snapshot_body_sha,
            "evaluation snapshot",
        )?;

        let receipt_body = serde_json::to_string(receipt)?;
        let receipt_body_sha = digest(receipt_body.as_bytes());
        Self::insert_idempotent(
            &transaction,
            "INSERT INTO evaluation_receipts
             (receipt_id, contract_id, operation_id, decision, score_millis,
              coverage_millis, confidence_millis, body_json, body_sha256, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(receipt_id) DO NOTHING",
            params![
                receipt.receipt_id.0.to_string(),
                contract_id,
                receipt.evaluation_operation_id.0.to_string(),
                enum_text(&receipt.decision)?,
                receipt.report.weighted_total_millis,
                receipt.report.evidence_coverage_millis,
                receipt.report.confidence_millis,
                receipt_body,
                receipt_body_sha,
                receipt.created_at_ms,
            ],
            "SELECT body_sha256 FROM evaluation_receipts WHERE receipt_id = ?1",
            &receipt.receipt_id.0.to_string(),
            &receipt_body_sha,
            "evaluation receipt",
        )?;
        transaction.commit()?;
        Ok(())
    }

    async fn get_receipt(&self, id: &EvaluationReceiptId) -> Result<Option<EvaluationReceipt>> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let body: Option<String> = connection
            .query_row(
                "SELECT body_json FROM evaluation_receipts WHERE receipt_id = ?1",
                params![id.0.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        body.map(|body| serde_json::from_str(&body).map_err(Into::into))
            .transpose()
    }

    async fn get_snapshot(
        &self,
        id: &EvaluationSnapshotId,
    ) -> Result<Option<EvaluationEvidenceSnapshot>> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let body: Option<String> = connection
            .query_row(
                "SELECT body_json FROM evaluation_snapshots WHERE snapshot_id = ?1",
                params![id.0.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        body.map(|body| serde_json::from_str(&body).map_err(Into::into))
            .transpose()
    }

    async fn get_snapshot_for_receipt(
        &self,
        id: &EvaluationReceiptId,
    ) -> Result<Option<EvaluationEvidenceSnapshot>> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let receipt_body: Option<String> = connection
            .query_row(
                "SELECT body_json FROM evaluation_receipts WHERE receipt_id = ?1",
                params![id.0.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let Some(receipt_body) = receipt_body else {
            return Ok(None);
        };
        let receipt: EvaluationReceipt = serde_json::from_str(&receipt_body)?;
        let snapshot_body: Option<String> = connection
            .query_row(
                "SELECT body_json FROM evaluation_snapshots
                 WHERE contract_id = ?1 AND snapshot_sha256 = ?2
                 ORDER BY created_at_ms DESC, snapshot_id DESC
                 LIMIT 1",
                params![
                    receipt.contract_id.0.to_string(),
                    receipt.evidence_snapshot_sha256
                ],
                |row| row.get(0),
            )
            .optional()?;
        snapshot_body
            .map(|body| serde_json::from_str(&body).map_err(Into::into))
            .transpose()
    }

    async fn latest_for_subject(
        &self,
        subject: &EvaluationSubject,
    ) -> Result<Option<EvaluationReceipt>> {
        let (kind, id) = subject.kind_and_id();
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let body: Option<String> = connection
            .query_row(
                "SELECT receipt.body_json
                 FROM evaluation_receipts receipt
                 JOIN evaluation_contracts contract
                   ON contract.contract_id = receipt.contract_id
                 WHERE contract.subject_kind = ?1 AND contract.subject_id = ?2
                 ORDER BY receipt.created_at_ms DESC, receipt.receipt_id DESC
                 LIMIT 1",
                params![kind, id],
                |row| row.get(0),
            )
            .optional()?;
        body.map(|body| serde_json::from_str(&body).map_err(Into::into))
            .transpose()
    }
}

fn enum_text(value: &impl serde::Serialize) -> Result<String> {
    let value = serde_json::to_value(value)?;
    value
        .as_str()
        .map(ToOwned::to_owned)
        .context("evaluation enum did not serialize as a string")
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::types::metacognition_evaluation::{EvaluationReport, GateResult, RubricId};
    use fabric::{
        EvaluationContractId, EvaluationDecision, EvaluationMode, EvaluationReceiptId,
        EvaluationThresholds, EvidenceRef, OperationId, RequiredGate, TaskKind, TurnId,
        EVALUATION_SCHEMA_V1,
    };

    fn contract() -> TaskEvaluationContract {
        TaskEvaluationContract {
            schema_version: EVALUATION_SCHEMA_V1,
            contract_id: EvaluationContractId::new(),
            task_kind: TaskKind::Coding,
            subject: EvaluationSubject::Turn {
                turn_id: TurnId::new(),
                operation_id: OperationId::new(),
            },
            rubric: RubricId("coding-v2".into()),
            rubric_version: 2,
            mode: EvaluationMode::Shadow,
            objective_ref: EvidenceRef("turn:user-message".into()),
            requirement_refs: vec![],
            required_evidence: vec![],
            required_gates: vec![RequiredGate {
                name: "required_verification_passed".into(),
            }],
            thresholds: EvaluationThresholds {
                min_score_millis: 70_000,
                min_evidence_coverage_millis: 600,
                min_confidence_millis: 700,
            },
            issued_by: "test".into(),
            issued_at_ms: 1,
        }
    }

    fn receipt(
        contract: &TaskEvaluationContract,
        snapshot: &EvaluationEvidenceSnapshot,
    ) -> EvaluationReceipt {
        EvaluationReceipt {
            schema_version: EVALUATION_SCHEMA_V1,
            receipt_id: EvaluationReceiptId::new(),
            contract_id: contract.contract_id,
            evaluation_operation_id: OperationId::new(),
            subject: contract.subject.clone(),
            evidence_snapshot_sha256: snapshot.sha256.clone(),
            report: EvaluationReport {
                rubric: contract.rubric.clone(),
                rubric_version: contract.rubric_version,
                dimensions: vec![],
                gates: vec![GateResult {
                    name: "required_verification_passed".into(),
                    passed: false,
                    evidence: vec![],
                }],
                weighted_total_millis: None,
                evidence_coverage_millis: 0,
                confidence_millis: 0,
                eligible: false,
            },
            decision: EvaluationDecision::ObservedFail,
            failed_gates: vec!["required_verification_passed".into()],
            evaluator: "test-evaluator".into(),
            created_at_ms: 2,
        }
    }

    #[tokio::test]
    async fn sqlite_evaluation_store_persisted_receipt_survives_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("evaluations.db");
        let contract = contract();
        let snapshot = EvaluationEvidenceSnapshot::new(contract.contract_id, vec![], 2).unwrap();
        let receipt = receipt(&contract, &snapshot);
        {
            let store = SqliteEvaluationStore::open(&path).unwrap();
            store.append_contract(&contract).await.unwrap();
            store.append_evaluation(&snapshot, &receipt).await.unwrap();
        }
        let reopened = SqliteEvaluationStore::open(&path).unwrap();
        assert_eq!(
            reopened.get_receipt(&receipt.receipt_id).await.unwrap(),
            Some(receipt.clone())
        );
        assert_eq!(
            reopened
                .get_snapshot_for_receipt(&receipt.receipt_id)
                .await
                .unwrap()
                .map(|value| value.sha256),
            Some(snapshot.sha256)
        );
        assert_eq!(
            reopened
                .latest_for_subject(&contract.subject)
                .await
                .unwrap(),
            Some(receipt)
        );
    }

    #[tokio::test]
    async fn sqlite_evaluation_store_conflict_rolls_back_new_snapshot() {
        let store = SqliteEvaluationStore::in_memory().unwrap();
        let contract = contract();
        let snapshot = EvaluationEvidenceSnapshot::new(contract.contract_id, vec![], 2).unwrap();
        let receipt = receipt(&contract, &snapshot);
        store.append_contract(&contract).await.unwrap();
        store.append_evaluation(&snapshot, &receipt).await.unwrap();

        let snapshot2 = EvaluationEvidenceSnapshot::new(contract.contract_id, vec![], 3).unwrap();
        let mut conflicting = receipt.clone();
        conflicting.evaluator = "different-evaluator".into();
        assert!(store
            .append_evaluation(&snapshot2, &conflicting)
            .await
            .is_err());
        assert!(store
            .get_snapshot(&snapshot2.snapshot_id)
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            store.get_receipt(&receipt.receipt_id).await.unwrap(),
            Some(receipt)
        );
    }
}

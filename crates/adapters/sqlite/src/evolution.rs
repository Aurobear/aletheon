//! SQLite projection store for governed evolution proposals.

use std::path::Path;
use std::sync::Mutex;

use application::evolution::EvolutionProposalStore;
use contracts::ApprovalId;
use rusqlite::{params, Connection, OptionalExtension};

pub struct SqliteEvolutionProposalStore {
    connection: Mutex<Connection>,
}

impl SqliteEvolutionProposalStore {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS evolution_proposals (
               mutation_id TEXT PRIMARY KEY NOT NULL,
               status TEXT NOT NULL, reason TEXT, approval_id TEXT,
               evidence_digest TEXT, updated_at_ms INTEGER NOT NULL
             );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
}

impl EvolutionProposalStore for SqliteEvolutionProposalStore {
    fn pending_approval(&self, mutation_id: uuid::Uuid) -> anyhow::Result<Option<ApprovalId>> {
        let id: Option<Option<String>> = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .query_row(
                "SELECT approval_id FROM evolution_proposals WHERE mutation_id=?1 AND status='pending_approval'",
                params![mutation_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        id.flatten()
            .map(|value| Ok(ApprovalId(uuid::Uuid::parse_str(&value)?)))
            .transpose()
    }

    fn park_insufficient_evidence(
        &self,
        mutation_id: uuid::Uuid,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        self.connection.lock().unwrap_or_else(|error| error.into_inner()).execute(
            "INSERT INTO evolution_proposals (mutation_id,status,reason,approval_id,evidence_digest,updated_at_ms)
             VALUES (?1,'parked','insufficient_evidence',NULL,NULL,?2)
             ON CONFLICT(mutation_id) DO UPDATE SET status='parked',reason='insufficient_evidence',updated_at_ms=excluded.updated_at_ms",
            params![mutation_id.to_string(), now_ms],
        )?;
        Ok(())
    }

    fn record_pending_approval(
        &self,
        mutation_id: uuid::Uuid,
        approval_id: ApprovalId,
        evidence_digest: &str,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        self.connection.lock().unwrap_or_else(|error| error.into_inner()).execute(
            "INSERT INTO evolution_proposals (mutation_id,status,reason,approval_id,evidence_digest,updated_at_ms)
             VALUES (?1,'pending_approval',NULL,?2,?3,?4)
             ON CONFLICT(mutation_id) DO UPDATE SET status='pending_approval',reason=NULL,approval_id=excluded.approval_id,evidence_digest=excluded.evidence_digest,updated_at_ms=excluded.updated_at_ms",
            params![mutation_id.to_string(), approval_id.0.to_string(), evidence_digest, now_ms],
        )?;
        Ok(())
    }
}

//! SelfFieldStore — SQLite persistence for NarrativeLayer and AttentionLayer.

use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// Durable R2 audit record for one conscious-field care modulation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CareModulationTrace {
    pub session_id: String,
    pub baseline: f64,
    pub effective: f64,
    pub delta: f64,
    pub precision: f32,
    pub observed_at_ms: i64,
}

/// Persistent store backed by SQLite.
pub struct SelfFieldStore {
    conn: Mutex<Connection>,
}

impl SelfFieldStore {
    /// Open (or create) the SQLite database and ensure the required tables exist.
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open SQLite database at {db_path:?}"))?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS narrative_entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event TEXT NOT NULL,
                reason TEXT NOT NULL,
                action TEXT,
                verdict TEXT,
                timestamp TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS attention_topics (
                topic TEXT PRIMARY KEY,
                priority REAL NOT NULL,
                started_at TEXT NOT NULL,
                last_updated TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS care_entries (
                topic TEXT PRIMARY KEY,
                weight REAL NOT NULL,
                description TEXT NOT NULL,
                keywords TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS boundary_rules (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                action_pattern TEXT NOT NULL,
                source_filter TEXT,
                action TEXT NOT NULL,
                risk_level TEXT NOT NULL,
                description TEXT NOT NULL,
                immutable INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS identity_current (
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                version TEXT NOT NULL,
                created_at TEXT NOT NULL,
                last_mutation TEXT
            );
            CREATE TABLE IF NOT EXISTS identity_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                version TEXT NOT NULL,
                created_at TEXT NOT NULL,
                last_mutation TEXT,
                mutated_at TEXT NOT NULL,
                reason TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS dasein_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS self_events (
                seq INTEGER PRIMARY KEY,
                event_id TEXT NOT NULL UNIQUE,
                previous_version INTEGER NOT NULL,
                next_version INTEGER NOT NULL UNIQUE,
                request_json TEXT NOT NULL,
                previous_checksum TEXT NOT NULL,
                checksum TEXT NOT NULL,
                observed_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS self_snapshots (
                version INTEGER PRIMARY KEY,
                last_event_seq INTEGER NOT NULL,
                event_prefix_json TEXT NOT NULL,
                checksum TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS self_lineage (
                version TEXT PRIMARY KEY,
                parent_version TEXT,
                mutation_id TEXT,
                approval_id TEXT,
                checksum TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS care_modulation_traces (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                baseline REAL NOT NULL,
                effective REAL NOT NULL,
                delta REAL NOT NULL,
                precision REAL NOT NULL,
                observed_at_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_care_modulation_session_time
                ON care_modulation_traces(session_id, observed_at_ms, id);
            ",
        )
        .context("Failed to create tables")?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Access the underlying connection (locked).
    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("store mutex poisoned")
    }

    pub fn append_care_modulation(&self, trace: &CareModulationTrace) -> Result<()> {
        self.conn().execute(
            "INSERT INTO care_modulation_traces
             (session_id, baseline, effective, delta, precision, observed_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                trace.session_id,
                trace.baseline,
                trace.effective,
                trace.delta,
                trace.precision,
                trace.observed_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn care_modulations(&self, session_id: &str) -> Result<Vec<CareModulationTrace>> {
        let connection = self.conn();
        let mut statement = connection.prepare(
            "SELECT session_id, baseline, effective, delta, precision, observed_at_ms
             FROM care_modulation_traces WHERE session_id = ?1
             ORDER BY observed_at_ms, id",
        )?;
        let rows = statement.query_map([session_id], |row| {
            Ok(CareModulationTrace {
                session_id: row.get(0)?,
                baseline: row.get(1)?,
                effective: row.get(2)?,
                delta: row.get(3)?,
                precision: row.get(4)?,
                observed_at_ms: row.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

/// Unified persistence interface for self-field layers.
pub trait Persistable {
    /// SQLite table name this layer persists to.
    fn table_name(&self) -> &str;
    /// Write current state to the store.
    fn save_to_store(&self, store: &SelfFieldStore) -> anyhow::Result<()>;
    /// Load state from the store, overwriting in-memory data.
    fn load_from_store(&mut self, store: &SelfFieldStore) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn care_modulation_trace_is_durable_and_ordered() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("self-field.db");
        {
            let store = SelfFieldStore::new(path.clone()).unwrap();
            for observed_at_ms in [20, 10] {
                store
                    .append_care_modulation(&CareModulationTrace {
                        session_id: "session".into(),
                        baseline: 0.2,
                        effective: 0.4,
                        delta: 0.2,
                        precision: 0.8,
                        observed_at_ms,
                    })
                    .unwrap();
            }
        }
        let reopened = SelfFieldStore::new(path).unwrap();
        let traces = reopened.care_modulations("session").unwrap();
        assert_eq!(traces.len(), 2);
        assert_eq!(traces[0].observed_at_ms, 10);
        assert_eq!(traces[1].observed_at_ms, 20);
        assert!(reopened.care_modulations("other").unwrap().is_empty());
    }
}

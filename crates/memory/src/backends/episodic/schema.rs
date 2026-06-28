//! EpisodicMemory schema, constructors, and Subsystem lifecycle.

use std::path::PathBuf;
use std::sync::Mutex;

use base::{
    Subsystem, SubsystemContext, SubsystemHealth, Version,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use rusqlite::Connection;

use crate::ops::schema;

/// Episodic memory backend — stores events, actions, observations.
pub struct EpisodicMemory {
    pub(crate) db_path: PathBuf,
    pub(crate) conn: Mutex<Option<Connection>>,
}

impl EpisodicMemory {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            db_path,
            conn: Mutex::new(None),
        }
    }

    pub(crate) fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
        let guard = self.conn.lock().unwrap();
        let conn = guard
            .as_ref()
            .expect("EpisodicMemory not initialized — call init() first");
        f(conn)
    }
}

#[async_trait]
impl Subsystem for EpisodicMemory {
    fn name(&self) -> &str {
        "episodic_memory"
    }

    async fn init(&mut self, _ctx: &SubsystemContext) -> Result<()> {
        let conn = Connection::open(&self.db_path)
            .with_context(|| format!("Failed to open {}", self.db_path.display()))?;
        schema::init_base_table(&conn)?;
        schema::init_awareness_table(&conn)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS episodic_events (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_id   TEXT NOT NULL,
                session_id  TEXT NOT NULL DEFAULT '',
                event_type  TEXT NOT NULL DEFAULT '',
                summary     TEXT NOT NULL DEFAULT '',
                raw_content BLOB,
                context     TEXT NOT NULL DEFAULT '{}',
                created_at  TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS reflection_events (
                id              TEXT PRIMARY KEY,
                memory_id       TEXT NOT NULL,
                trigger_type    TEXT NOT NULL,
                task_summary    TEXT NOT NULL,
                outcome         TEXT NOT NULL,
                what_worked     TEXT NOT NULL DEFAULT '[]',
                what_failed     TEXT NOT NULL DEFAULT '[]',
                learned         TEXT NOT NULL DEFAULT '[]',
                behavior_changes TEXT NOT NULL DEFAULT '[]',
                confidence      REAL NOT NULL DEFAULT 0.0,
                created_at      TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS evolution_log_events (
                id              TEXT PRIMARY KEY,
                trigger         TEXT NOT NULL,
                basis           TEXT NOT NULL DEFAULT '[]',
                patterns        TEXT NOT NULL DEFAULT '[]',
                adjustments     TEXT NOT NULL DEFAULT '[]',
                created_at      TEXT NOT NULL
            );",
        )?;
        self.conn = Mutex::new(Some(conn));
        tracing::info!(path = %self.db_path.display(), "EpisodicMemory initialized");
        Ok(())
    }

    async fn health(&self) -> SubsystemHealth {
        let guard = self.conn.lock().unwrap();
        if guard.is_some() {
            SubsystemHealth::Healthy
        } else {
            SubsystemHealth::Degraded {
                reason: "not initialized".into(),
            }
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        let mut guard = self.conn.lock().unwrap();
        *guard = None;
        Ok(())
    }

    fn version(&self) -> Version {
        Version::new(0, 1, 0)
    }
}

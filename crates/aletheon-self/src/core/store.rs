//! SelfFieldStore — SQLite persistence for NarrativeLayer and AttentionLayer.

use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Persistent store backed by SQLite.
pub struct SelfFieldStore {
    conn: Mutex<Connection>,
}

impl SelfFieldStore {
    /// Open (or create) the SQLite database and ensure the required tables exist.
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open SQLite database at {:?}", db_path))?;

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
}

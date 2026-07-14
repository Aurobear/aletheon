//! Versioned schema migrations for the objective store.
//!
//! Uses `PRAGMA user_version` for idempotent migration. Each migration
//! runs inside a single transaction.

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Current schema version.
const CURRENT_VERSION: u32 = 2;

/// Migration 1 schema — the original `objectives` table without extended goal columns.
const MIGRATION_1: &str = "
CREATE TABLE IF NOT EXISTS objectives (
    objective_id INTEGER PRIMARY KEY AUTOINCREMENT,
    description  TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'in_progress'
                 CHECK(status IN ('in_progress','completed','failed','adjusted')),
    parent_id    INTEGER REFERENCES objectives(objective_id) ON DELETE CASCADE,
    session_id   TEXT NOT NULL DEFAULT '',
    scope        TEXT NOT NULL DEFAULT 'session'
                 CHECK(scope IN ('session','project','global')),
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_objectives_status ON objectives(status);
CREATE INDEX IF NOT EXISTS idx_objectives_parent ON objectives(parent_id);
CREATE INDEX IF NOT EXISTS idx_objectives_session ON objectives(session_id);
CREATE INDEX IF NOT EXISTS idx_objectives_scope ON objectives(scope);
";

/// Migration 2 — adds M2 Goal columns and event/ledger tables.
const MIGRATION_2: &str = "
-- Add new goal columns to objectives (only if absent).
ALTER TABLE objectives ADD COLUMN owner_id TEXT NOT NULL DEFAULT 'local-owner';
ALTER TABLE objectives ADD COLUMN goal_state TEXT NOT NULL DEFAULT 'ready';
ALTER TABLE objectives ADD COLUMN spec_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE objectives ADD COLUMN wait_json TEXT;
ALTER TABLE objectives ADD COLUMN process_id TEXT;
ALTER TABLE objectives ADD COLUMN version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE objectives ADD COLUMN deadline_ms INTEGER;

-- Event log: one row per version bump (append-only audit).
CREATE TABLE IF NOT EXISTS goal_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    objective_id INTEGER NOT NULL REFERENCES objectives(objective_id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(objective_id, version)
);

-- Budget ledger: reservations + settlements for token/cost accounting.
CREATE TABLE IF NOT EXISTS goal_budget_ledger (
    ledger_id INTEGER PRIMARY KEY AUTOINCREMENT,
    objective_id INTEGER NOT NULL REFERENCES objectives(objective_id) ON DELETE CASCADE,
    reservation_id TEXT NOT NULL UNIQUE,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cost_usd REAL NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL CHECK(status IN ('reserved','settled','revoked')),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    settled_at TEXT
);
";

/// Run all pending migrations inside a transaction.
pub fn run_migrations(db: &Connection) -> Result<()> {
    let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;

    if version < 1 {
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        let tx = db
            .unchecked_transaction()
            .context("begin migration 1 transaction")?;
        tx.execute_batch(MIGRATION_1)?;
        tx.pragma_update(None, "user_version", 1)?;
        tx.commit()?;
    }

    if version < 2 {
        let tx = db
            .unchecked_transaction()
            .context("begin migration 2 transaction")?;
        tx.execute_batch(MIGRATION_2)?;
        tx.pragma_update(None, "user_version", 2)?;
        tx.commit()?;
    }

    // Verify we're at the expected version.
    let current: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    anyhow::ensure!(
        current == CURRENT_VERSION,
        "objective store schema at version {current}, expected {CURRENT_VERSION}"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn open_db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        run_migrations(&db).unwrap();
        db
    }

    #[test]
    fn legacy_schema_migrates_to_v2() {
        // Manually create the legacy V1 schema.
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(MIGRATION_1).unwrap();
        db.pragma_update(None, "user_version", 1).unwrap();

        // Insert a legacy row.
        db.execute(
            "INSERT INTO objectives (description, session_id, scope)
             VALUES (?1, ?2, ?3)",
            rusqlite::params!["legacy goal", "sess-1", "project"],
        )
        .unwrap();

        // Re-open / migrate to V2.
        run_migrations(&db).unwrap();

        // Row still readable.
        let desc: String = db
            .query_row(
                "SELECT description FROM objectives WHERE objective_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(desc, "legacy goal");

        // New columns have defaults.
        let owner: String = db
            .query_row(
                "SELECT owner_id FROM objectives WHERE objective_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(owner, "local-owner");
    }

    #[test]
    fn migration_is_idempotent() {
        let db = open_db();
        let v1: u32 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v1, 2);

        // Running again is a no-op.
        run_migrations(&db).unwrap();
        let v2: u32 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v2, 2);
    }

    #[test]
    fn fresh_db_has_all_tables() {
        let db = open_db();
        let tables: Vec<String> = db
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert!(tables.iter().any(|n| n == "objectives"));
        assert!(tables.iter().any(|n| n == "goal_events"));
        assert!(tables.iter().any(|n| n == "goal_budget_ledger"));
    }

    #[test]
    fn goal_events_enforce_unique_version() {
        let db = open_db();
        db.execute(
            "INSERT INTO objectives (description, goal_state) VALUES (?1, ?2)",
            rusqlite::params!["test", "draft"],
        )
        .unwrap();
        db.execute(
            "INSERT INTO goal_events (objective_id, version, event_type, payload_json)
             VALUES (1, 0, 'created', '{}')",
            [],
        )
        .unwrap();
        let err = db
            .execute(
                "INSERT INTO goal_events (objective_id, version, event_type, payload_json)
                 VALUES (1, 0, 'duplicate', '{}')",
                [],
            )
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("UNIQUE") || msg.contains("unique"));
    }
}

//! Versioned schema migrations for the objective store.
//!
//! Uses `PRAGMA user_version` for idempotent migration. Each migration
//! runs inside a single transaction.

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Current schema version.
const CURRENT_VERSION: u32 = 7;

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

/// Migration 3 — durable runtime attempts and immutable attempt identity.
const MIGRATION_3: &str = "
CREATE TABLE IF NOT EXISTS goal_attempts (
    attempt_id TEXT PRIMARY KEY,
    objective_id INTEGER NOT NULL REFERENCES objectives(objective_id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    runtime_id TEXT NOT NULL,
    role TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('running','succeeded','failed','cancelled')),
    input_json TEXT NOT NULL,
    output_json TEXT,
    failure_json TEXT,
    evidence_json TEXT NOT NULL DEFAULT '[]',
    usage_json TEXT NOT NULL DEFAULT '{}',
    started_at TEXT NOT NULL,
    ended_at TEXT,
    UNIQUE(objective_id, sequence)
);
CREATE INDEX IF NOT EXISTS idx_goal_attempts_objective_sequence
    ON goal_attempts(objective_id, sequence DESC);
CREATE INDEX IF NOT EXISTS idx_goal_attempts_status ON goal_attempts(status);

CREATE TRIGGER IF NOT EXISTS goal_attempts_immutable_identity
BEFORE UPDATE OF objective_id, sequence, runtime_id, role, input_json, started_at
ON goal_attempts
BEGIN
    SELECT RAISE(ABORT, 'goal attempt identity is immutable');
END;
";

/// Migration 4 — immutable coding jobs and their deterministic verification evidence.
const MIGRATION_4: &str = "
CREATE TABLE IF NOT EXISTS goal_coding_jobs (
    job_id TEXT PRIMARY KEY,
    objective_id INTEGER NOT NULL REFERENCES objectives(objective_id) ON DELETE CASCADE,
    attempt_id TEXT NOT NULL UNIQUE REFERENCES goal_attempts(attempt_id) ON DELETE CASCADE,
    base_commit TEXT NOT NULL,
    worktree_ref TEXT NOT NULL,
    report_json TEXT NOT NULL,
    diff_artifact_ref TEXT NOT NULL,
    diff_sha256 TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('running','succeeded','failed','timed_out','cancelled','retained')),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_goal_coding_jobs_objective
    ON goal_coding_jobs(objective_id, created_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_goal_coding_jobs_status
    ON goal_coding_jobs(status, created_at_ms);

CREATE TABLE IF NOT EXISTS goal_verification_reports (
    job_id TEXT PRIMARY KEY REFERENCES goal_coding_jobs(job_id) ON DELETE CASCADE,
    objective_id INTEGER NOT NULL REFERENCES objectives(objective_id) ON DELETE CASCADE,
    attempt_id TEXT NOT NULL UNIQUE REFERENCES goal_attempts(attempt_id) ON DELETE CASCADE,
    report_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('passed','failed')),
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_goal_verification_reports_objective
    ON goal_verification_reports(objective_id, created_at_ms DESC);

CREATE TRIGGER IF NOT EXISTS goal_coding_jobs_immutable_identity
BEFORE UPDATE OF job_id, objective_id, attempt_id, base_commit, worktree_ref,
                 diff_artifact_ref, diff_sha256, created_at_ms
ON goal_coding_jobs
BEGIN
    SELECT RAISE(ABORT, 'coding job identity is immutable');
END;

CREATE TRIGGER IF NOT EXISTS goal_verification_reports_immutable
BEFORE UPDATE ON goal_verification_reports
BEGIN
    SELECT RAISE(ABORT, 'verification report is immutable');
END;
";

/// Migration 5 — restart-safe protected-operation approvals and audit events.
const MIGRATION_5: &str = "
CREATE TABLE IF NOT EXISTS approval_requests (
    approval_id TEXT PRIMARY KEY,
    objective_id INTEGER NOT NULL REFERENCES objectives(objective_id) ON DELETE CASCADE,
    attempt_id TEXT REFERENCES goal_attempts(attempt_id) ON DELETE CASCADE,
    job_id TEXT REFERENCES goal_coding_jobs(job_id) ON DELETE CASCADE,
    owner_id TEXT NOT NULL,
    category TEXT NOT NULL,
    risk TEXT NOT NULL,
    subject_json TEXT NOT NULL,
    subject_hash TEXT NOT NULL,
    summary TEXT NOT NULL,
    artifacts_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending','approved','rejected','expired','consumed')),
    version INTEGER NOT NULL DEFAULT 0,
    resolution_principal TEXT,
    resolution_channel TEXT,
    resolution_time_ms INTEGER,
    resolution_reason TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_approval_active_subject
    ON approval_requests(category, subject_hash)
    WHERE status IN ('pending','approved');
CREATE INDEX IF NOT EXISTS idx_approval_pending_owner
    ON approval_requests(owner_id, status, expires_at_ms);

CREATE TABLE IF NOT EXISTS approval_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    approval_id TEXT NOT NULL REFERENCES approval_requests(approval_id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    UNIQUE(approval_id, version)
);

CREATE TRIGGER IF NOT EXISTS approval_requests_immutable_subject
BEFORE UPDATE OF approval_id, objective_id, attempt_id, job_id, owner_id, category,
                 risk, subject_json, subject_hash, summary, artifacts_json,
                 created_at_ms, expires_at_ms
ON approval_requests
BEGIN
    SELECT RAISE(ABORT, 'approval subject is immutable');
END;

CREATE TRIGGER IF NOT EXISTS approval_events_append_only_update
BEFORE UPDATE ON approval_events BEGIN
    SELECT RAISE(ABORT, 'approval events are append-only');
END;
CREATE TRIGGER IF NOT EXISTS approval_events_append_only_delete
BEFORE DELETE ON approval_events BEGIN
    SELECT RAISE(ABORT, 'approval events are append-only');
END;
";

/// Migration 6 — delivery correlation for durable approval notifications.
const MIGRATION_6: &str = "
CREATE TABLE IF NOT EXISTS approval_deliveries (
    approval_id TEXT NOT NULL REFERENCES approval_requests(approval_id) ON DELETE CASCADE,
    channel TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    correlation_id TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK(status IN ('pending','sent','failed')),
    provider_message_id TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(approval_id, channel)
);
";

/// Migration 7 — one durable apply claim and receipt per approved request.
const MIGRATION_7: &str = "
CREATE TABLE IF NOT EXISTS approval_apply_operations (
    approval_id TEXT PRIMARY KEY REFERENCES approval_requests(approval_id) ON DELETE CASCADE,
    operation_id TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK(status IN ('running','succeeded','failed','cancelled')),
    started_at_ms INTEGER NOT NULL,
    finished_at_ms INTEGER,
    error TEXT
);
CREATE TABLE IF NOT EXISTS approval_apply_receipts (
    approval_id TEXT PRIMARY KEY REFERENCES approval_requests(approval_id) ON DELETE CASCADE,
    operation_id TEXT NOT NULL UNIQUE REFERENCES approval_apply_operations(operation_id),
    receipt_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
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

    if version < 3 {
        let tx = db
            .unchecked_transaction()
            .context("begin migration 3 transaction")?;
        tx.execute_batch(MIGRATION_3)?;
        tx.pragma_update(None, "user_version", 3)?;
        tx.commit()?;
    }

    if version < 4 {
        let tx = db
            .unchecked_transaction()
            .context("begin migration 4 transaction")?;
        tx.execute_batch(MIGRATION_4)?;
        tx.pragma_update(None, "user_version", 4)?;
        tx.commit()?;
    }

    if version < 5 {
        let tx = db
            .unchecked_transaction()
            .context("begin migration 5 transaction")?;
        tx.execute_batch(MIGRATION_5)?;
        tx.pragma_update(None, "user_version", 5)?;
        tx.commit()?;
    }

    if version < 6 {
        let tx = db
            .unchecked_transaction()
            .context("begin migration 6 transaction")?;
        tx.execute_batch(MIGRATION_6)?;
        tx.pragma_update(None, "user_version", 6)?;
        tx.commit()?;
    }

    if version < 7 {
        let tx = db
            .unchecked_transaction()
            .context("begin migration 7 transaction")?;
        tx.execute_batch(MIGRATION_7)?;
        tx.pragma_update(None, "user_version", 7)?;
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
    fn legacy_schema_migrates_to_latest() {
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

        // Re-open / migrate to the current schema.
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
        assert_eq!(v1, 7);

        // Running again is a no-op.
        run_migrations(&db).unwrap();
        let v2: u32 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v2, 7);
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
        assert!(tables.iter().any(|n| n == "goal_attempts"));
        assert!(tables.iter().any(|n| n == "goal_coding_jobs"));
        assert!(tables.iter().any(|n| n == "goal_verification_reports"));
        assert!(tables.iter().any(|n| n == "approval_requests"));
        assert!(tables.iter().any(|n| n == "approval_events"));
        assert!(tables.iter().any(|n| n == "approval_deliveries"));
        assert!(tables.iter().any(|n| n == "approval_apply_operations"));
        assert!(tables.iter().any(|n| n == "approval_apply_receipts"));
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

//! Shared SQLite schema helpers.

use anyhow::Result;
use rusqlite::Connection;

/// Base table shared by all backends — one row per memory entry.
pub fn init_base_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS aletheon_memory (
            id           TEXT PRIMARY KEY,
            memory_type  TEXT NOT NULL,
            content      BLOB NOT NULL,
            tags         TEXT NOT NULL DEFAULT '[]',
            created_at   TEXT NOT NULL,
            access_count INTEGER NOT NULL DEFAULT 0,
            importance   REAL NOT NULL DEFAULT 0.5,
            decay_rate   REAL NOT NULL DEFAULT 0.0,
            associations TEXT NOT NULL DEFAULT '[]'
        );",
    )?;
    Ok(())
}

/// Initialize the awareness_events table.
///
/// This table stores SelfAwareness entries alongside episodic events.
/// Each entry is linked to an episodic event via memory_id.
pub fn init_awareness_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS awareness_events (
            id              TEXT PRIMARY KEY,
            memory_id       TEXT NOT NULL,
            action          TEXT NOT NULL,
            aware           INTEGER NOT NULL DEFAULT 1,
            extensions      TEXT NOT NULL DEFAULT '[]',
            created_at      TEXT NOT NULL,
            FOREIGN KEY (memory_id) REFERENCES aletheon_memory(id)
        );

        CREATE INDEX IF NOT EXISTS idx_awareness_memory_id
            ON awareness_events(memory_id);

        CREATE INDEX IF NOT EXISTS idx_awareness_created_at
            ON awareness_events(created_at);"
    )?;
    Ok(())
}

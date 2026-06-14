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

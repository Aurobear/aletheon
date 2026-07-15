//! SQLite schema for the durable GBrain delivery spool.

use rusqlite::Connection;

pub(crate) const SCHEMA_VERSION: i64 = 1;

pub(crate) fn migrate(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS gbrain_pages (
           record_id TEXT PRIMARY KEY,
           slug TEXT NOT NULL UNIQUE,
           content TEXT NOT NULL,
           content_hash TEXT NOT NULL,
           payload_bytes INTEGER NOT NULL CHECK(payload_bytes >= 0),
           created_ms INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS gbrain_queue (
           record_id TEXT PRIMARY KEY REFERENCES gbrain_pages(record_id) ON DELETE CASCADE,
           state TEXT NOT NULL CHECK(state IN ('pending','leased')),
           attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
           next_attempt_ms INTEGER NOT NULL,
           lease_owner TEXT,
           lease_until_ms INTEGER,
           updated_ms INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_gbrain_queue_due
           ON gbrain_queue(state, next_attempt_ms, lease_until_ms);
         CREATE TABLE IF NOT EXISTS gbrain_attempts (
           attempt_id INTEGER PRIMARY KEY AUTOINCREMENT,
           record_id TEXT NOT NULL,
           attempt_no INTEGER NOT NULL,
           started_ms INTEGER NOT NULL,
           completed_ms INTEGER,
           outcome TEXT,
           error_category TEXT
         );
         CREATE TABLE IF NOT EXISTS gbrain_delivery_receipts (
           record_id TEXT PRIMARY KEY,
           slug TEXT NOT NULL,
           content_hash TEXT NOT NULL,
           delivered_ms INTEGER NOT NULL,
           remote_receipt TEXT
         );
         CREATE TABLE IF NOT EXISTS gbrain_dead_letters (
           record_id TEXT PRIMARY KEY,
           slug TEXT NOT NULL UNIQUE,
           content TEXT NOT NULL,
           content_hash TEXT NOT NULL,
           payload_bytes INTEGER NOT NULL,
           attempts INTEGER NOT NULL,
           created_ms INTEGER NOT NULL,
           failed_ms INTEGER NOT NULL,
           reason_category TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS gbrain_spool_meta (
           key TEXT PRIMARY KEY,
           value TEXT NOT NULL
         );",
    )?;
    connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

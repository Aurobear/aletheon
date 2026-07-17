//! SQLite schema for the durable GBrain delivery spool.

use rusqlite::Connection;

pub(crate) const SCHEMA_VERSION: i64 = 2;

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
    add_column(
        connection,
        "gbrain_pages",
        "logical_page_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column(
        connection,
        "gbrain_pages",
        "operation_kind",
        "TEXT NOT NULL DEFAULT 'upsert'",
    )?;
    add_column(
        connection,
        "gbrain_pages",
        "schema_version",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    add_column(
        connection,
        "gbrain_delivery_receipts",
        "logical_page_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column(connection, "gbrain_delivery_receipts", "remote_id", "TEXT")?;
    add_column(
        connection,
        "gbrain_delivery_receipts",
        "operation_kind",
        "TEXT NOT NULL DEFAULT 'upsert'",
    )?;
    add_column(
        connection,
        "gbrain_delivery_receipts",
        "schema_version",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    add_column(
        connection,
        "gbrain_delivery_receipts",
        "synced_at_ms",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column(
        connection,
        "gbrain_dead_letters",
        "logical_page_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column(
        connection,
        "gbrain_dead_letters",
        "operation_kind",
        "TEXT NOT NULL DEFAULT 'upsert'",
    )?;
    add_column(
        connection,
        "gbrain_dead_letters",
        "schema_version",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    connection.execute_batch(
        "UPDATE gbrain_pages SET logical_page_id=slug WHERE logical_page_id='';
         UPDATE gbrain_delivery_receipts SET logical_page_id=slug WHERE logical_page_id='';
         UPDATE gbrain_delivery_receipts SET remote_id=remote_receipt WHERE remote_id IS NULL;
         UPDATE gbrain_delivery_receipts SET synced_at_ms=delivered_ms WHERE synced_at_ms=0;
         UPDATE gbrain_dead_letters SET logical_page_id=slug WHERE logical_page_id='';",
    )?;
    connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

fn add_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == column);
    if !exists {
        connection.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}

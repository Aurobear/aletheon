//! Transactional SQLite schema for the durable GBrain delivery spool.

use rusqlite::{Connection, Transaction, TransactionBehavior};

pub(crate) const SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MigrationStep {
    BaseSchema,
    PageLogicalId,
    PageOperationKind,
    PageSchemaVersion,
    ReceiptLogicalId,
    ReceiptRemoteId,
    ReceiptOperationKind,
    ReceiptSchemaVersion,
    ReceiptSyncedAt,
    DeadLetterLogicalId,
    DeadLetterOperationKind,
    DeadLetterSchemaVersion,
    Backfill,
    Version,
}

pub(crate) fn migrate(connection: &Connection) -> rusqlite::Result<()> {
    migrate_with_step_hook(connection, |_| Ok(()))
}

fn migrate_with_step_hook(
    connection: &Connection,
    mut after_step: impl FnMut(MigrationStep) -> rusqlite::Result<()>,
) -> rusqlite::Result<()> {
    connection.execute_batch("PRAGMA foreign_keys=ON;")?;
    let current: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current > SCHEMA_VERSION {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "GBrain spool schema version {current} is newer than supported {SCHEMA_VERSION}"
        )));
    }

    // SQLite DDL is transactional. Keeping every table, column, backfill, and
    // user_version change in one IMMEDIATE transaction guarantees that process
    // interruption or statement failure leaves either the prior schema or v2.
    let tx = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS gbrain_pages (
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
    after_step(MigrationStep::BaseSchema)?;

    add_column(
        &tx,
        "gbrain_pages",
        "logical_page_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    after_step(MigrationStep::PageLogicalId)?;
    add_column(
        &tx,
        "gbrain_pages",
        "operation_kind",
        "TEXT NOT NULL DEFAULT 'upsert'",
    )?;
    after_step(MigrationStep::PageOperationKind)?;
    add_column(
        &tx,
        "gbrain_pages",
        "schema_version",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    after_step(MigrationStep::PageSchemaVersion)?;
    add_column(
        &tx,
        "gbrain_delivery_receipts",
        "logical_page_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    after_step(MigrationStep::ReceiptLogicalId)?;
    add_column(&tx, "gbrain_delivery_receipts", "remote_id", "TEXT")?;
    after_step(MigrationStep::ReceiptRemoteId)?;
    add_column(
        &tx,
        "gbrain_delivery_receipts",
        "operation_kind",
        "TEXT NOT NULL DEFAULT 'upsert'",
    )?;
    after_step(MigrationStep::ReceiptOperationKind)?;
    add_column(
        &tx,
        "gbrain_delivery_receipts",
        "schema_version",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    after_step(MigrationStep::ReceiptSchemaVersion)?;
    add_column(
        &tx,
        "gbrain_delivery_receipts",
        "synced_at_ms",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    after_step(MigrationStep::ReceiptSyncedAt)?;
    add_column(
        &tx,
        "gbrain_dead_letters",
        "logical_page_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    after_step(MigrationStep::DeadLetterLogicalId)?;
    add_column(
        &tx,
        "gbrain_dead_letters",
        "operation_kind",
        "TEXT NOT NULL DEFAULT 'upsert'",
    )?;
    after_step(MigrationStep::DeadLetterOperationKind)?;
    add_column(
        &tx,
        "gbrain_dead_letters",
        "schema_version",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    after_step(MigrationStep::DeadLetterSchemaVersion)?;

    tx.execute_batch(
        "UPDATE gbrain_pages SET logical_page_id=slug WHERE logical_page_id='';
         UPDATE gbrain_delivery_receipts SET logical_page_id=slug WHERE logical_page_id='';
         UPDATE gbrain_delivery_receipts SET remote_id=remote_receipt WHERE remote_id IS NULL;
         UPDATE gbrain_delivery_receipts SET synced_at_ms=delivered_ms WHERE synced_at_ms=0;
         UPDATE gbrain_dead_letters SET logical_page_id=slug WHERE logical_page_id='';",
    )?;
    after_step(MigrationStep::Backfill)?;
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    after_step(MigrationStep::Version)?;
    tx.commit()
}

fn add_column(
    transaction: &Transaction<'_>,
    table: &str,
    column: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    let mut statement = transaction.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == column);
    if !exists {
        transaction.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEPS: [MigrationStep; 14] = [
        MigrationStep::BaseSchema,
        MigrationStep::PageLogicalId,
        MigrationStep::PageOperationKind,
        MigrationStep::PageSchemaVersion,
        MigrationStep::ReceiptLogicalId,
        MigrationStep::ReceiptRemoteId,
        MigrationStep::ReceiptOperationKind,
        MigrationStep::ReceiptSchemaVersion,
        MigrationStep::ReceiptSyncedAt,
        MigrationStep::DeadLetterLogicalId,
        MigrationStep::DeadLetterOperationKind,
        MigrationStep::DeadLetterSchemaVersion,
        MigrationStep::Backfill,
        MigrationStep::Version,
    ];

    fn previous_schema(connection: &Connection) {
        connection.execute_batch(
            "CREATE TABLE gbrain_pages(record_id TEXT PRIMARY KEY,slug TEXT NOT NULL UNIQUE,content TEXT NOT NULL,content_hash TEXT NOT NULL,payload_bytes INTEGER NOT NULL,created_ms INTEGER NOT NULL);
             CREATE TABLE gbrain_queue(record_id TEXT PRIMARY KEY REFERENCES gbrain_pages(record_id) ON DELETE CASCADE,state TEXT NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,next_attempt_ms INTEGER NOT NULL,lease_owner TEXT,lease_until_ms INTEGER,updated_ms INTEGER NOT NULL);
             CREATE TABLE gbrain_attempts(attempt_id INTEGER PRIMARY KEY AUTOINCREMENT,record_id TEXT NOT NULL,attempt_no INTEGER NOT NULL,started_ms INTEGER NOT NULL,completed_ms INTEGER,outcome TEXT,error_category TEXT);
             CREATE TABLE gbrain_delivery_receipts(record_id TEXT PRIMARY KEY,slug TEXT NOT NULL,content_hash TEXT NOT NULL,delivered_ms INTEGER NOT NULL,remote_receipt TEXT);
             CREATE TABLE gbrain_dead_letters(record_id TEXT PRIMARY KEY,slug TEXT NOT NULL UNIQUE,content TEXT NOT NULL,content_hash TEXT NOT NULL,payload_bytes INTEGER NOT NULL,attempts INTEGER NOT NULL,created_ms INTEGER NOT NULL,failed_ms INTEGER NOT NULL,reason_category TEXT NOT NULL);
             CREATE TABLE gbrain_spool_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
             INSERT INTO gbrain_pages VALUES('record','page','body','hash',4,10);
             INSERT INTO gbrain_delivery_receipts VALUES('record','page','hash',20,'remote');
             PRAGMA user_version=1;",
        ).unwrap();
    }

    #[test]
    fn every_step_failure_rolls_back_and_reopen_completes() {
        for failed_step in STEPS {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("spool.db");
            let connection = Connection::open(&path).unwrap();
            previous_schema(&connection);
            let error = migrate_with_step_hook(&connection, |step| {
                if step == failed_step {
                    Err(rusqlite::Error::ExecuteReturnedResults)
                } else {
                    Ok(())
                }
            });
            assert!(error.is_err(), "step {failed_step:?} did not fail");
            assert_eq!(
                connection
                    .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                1,
                "version advanced at {failed_step:?}"
            );
            drop(connection);

            let reopened = Connection::open(&path).unwrap();
            migrate(&reopened).unwrap();
            assert_eq!(
                reopened
                    .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                SCHEMA_VERSION
            );
            let migrated: (String, String, i64) = reopened
                .query_row(
                    "SELECT logical_page_id,operation_kind,schema_version FROM gbrain_pages WHERE record_id='record'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(migrated, ("page".into(), "upsert".into(), 1));
        }
    }

    #[test]
    fn newer_schema_fails_closed_without_mutation() {
        let connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
        assert!(migrate(&connection).is_err());
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            99
        );
    }
}

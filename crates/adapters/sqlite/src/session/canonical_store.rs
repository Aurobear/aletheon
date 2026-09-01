//! Canonical transactional Session/Turn/Item history store.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use ::contracts::{
    AppendOutcome, ItemId, ItemRecord, PrincipalId, SessionId, SessionReadStore, SessionRecord,
    SESSION_SCHEMA_VERSION,
};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};

use runtime::session_projection::SessionProjectionStore;

pub struct CanonicalSessionStore {
    locator: DatabaseLocator,
    /// Keeps a shared in-memory SQLite database alive. Production file-backed
    /// operations never acquire this mutex; each operation opens its own
    /// bounded-busy connection, so unrelated sessions have no application-wide
    /// connection lock.
    _memory_keeper: Option<Mutex<Connection>>,
}

enum DatabaseLocator {
    File(PathBuf),
    SharedMemory(String),
}

// Keep the database migration marker aligned with the newest record protocol
// migration. Version 5 adds Host-authored TaskProjection Session items; older JSON
// payloads are structurally compatible but must have their explicit record
// version advanced before event-spine reconciliation compares them.
const DATABASE_SCHEMA_VERSION: i64 = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MigrationStep {
    Schema,
    Version,
}

pub fn default_session_db_path() -> std::path::PathBuf {
    ::contracts::paths::xdg_data_dir().join("sessions-v1.db")
}

/// Canonical session database for an explicitly owned user state root.
pub fn session_db_path(state_root: &Path) -> std::path::PathBuf {
    state_root.join("sessions-v1.db")
}

impl CanonicalSessionStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path == Path::new(":memory:") {
            let uri = format!(
                "file:aletheon-session-{}?mode=memory&cache=shared",
                uuid::Uuid::new_v4()
            );
            let connection = open_shared_memory(&uri)?;
            configure_connection(&connection, false)?;
            migrate(&connection)?;
            Ok(Self {
                locator: DatabaseLocator::SharedMemory(uri),
                _memory_keeper: Some(Mutex::new(connection)),
            })
        } else {
            let path = path.to_path_buf();
            let connection = Connection::open(&path)?;
            configure_connection(&connection, true)?;
            migrate(&connection)?;
            Ok(Self {
                locator: DatabaseLocator::File(path),
                _memory_keeper: None,
            })
        }
    }

    fn open_connection(&self) -> Result<Connection> {
        let (connection, file_backed) = match &self.locator {
            DatabaseLocator::File(path) => (Connection::open(path)?, true),
            DatabaseLocator::SharedMemory(uri) => (open_shared_memory(uri)?, false),
        };
        configure_connection(&connection, file_backed)?;
        Ok(connection)
    }

    fn validate_session(session: &SessionRecord) -> Result<()> {
        if session.schema_version != SESSION_SCHEMA_VERSION {
            bail!(
                "unsupported session schema version {}",
                session.schema_version
            );
        }
        Ok(())
    }

    fn validate_item(session: &SessionId, expected: u64, item: &ItemRecord) -> Result<()> {
        if item.schema_version != SESSION_SCHEMA_VERSION {
            bail!("unsupported item schema version {}", item.schema_version);
        }
        if &item.session_id != session {
            bail!("item session does not match append target");
        }
        if item.sequence != expected {
            bail!(
                "item sequence {} does not match expected {}",
                item.sequence,
                expected
            );
        }
        if matches!(
            &item.payload,
            ::contracts::ItemPayload::TaskProjection { fact }
                if fact.schema_version != ::contracts::TASK_PROJECTION_FACT_SCHEMA_VERSION
        ) {
            bail!("unsupported Task projection fact schema version");
        }
        Ok(())
    }
}

fn open_shared_memory(uri: &str) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_URI,
    )
}

fn configure_connection(connection: &Connection, file_backed: bool) -> Result<()> {
    connection.busy_timeout(Duration::from_secs(2))?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    if file_backed {
        connection.pragma_update(None, "journal_mode", "WAL")?;
    }
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

fn migrate(connection: &Connection) -> Result<()> {
    migrate_with_step_hook(connection, |_| Ok(()))
}

fn migrate_with_step_hook(
    connection: &Connection,
    mut after_step: impl FnMut(MigrationStep) -> Result<()>,
) -> Result<()> {
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    let current: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current > DATABASE_SCHEMA_VERSION {
        bail!(
            "session database schema version {current} is newer than supported {DATABASE_SCHEMA_VERSION}"
        );
    }

    if current < DATABASE_SCHEMA_VERSION {
        // SQLite DDL and the version marker share one transaction. An
        // interruption therefore leaves either the previous database or a
        // complete v1 schema, never a version that overstates its structure.
        let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS sessions(
               session_id TEXT PRIMARY KEY,
               schema_version INTEGER NOT NULL,
               record_json TEXT NOT NULL,
               next_sequence INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS session_items(
               session_id TEXT NOT NULL,
               sequence INTEGER NOT NULL,
               item_id TEXT NOT NULL UNIQUE,
               turn_id TEXT NOT NULL,
               item_json TEXT NOT NULL,
               PRIMARY KEY(session_id, sequence),
               FOREIGN KEY(session_id) REFERENCES sessions(session_id)
             );
             CREATE TABLE IF NOT EXISTS recovered_turns(
               session_id TEXT NOT NULL,
               turn_id TEXT NOT NULL,
               classification TEXT NOT NULL,
               PRIMARY KEY(session_id, turn_id),
               FOREIGN KEY(session_id) REFERENCES sessions(session_id)
             );
             CREATE TABLE IF NOT EXISTS session_principals(
               session_id TEXT PRIMARY KEY,
               principal_id TEXT NOT NULL,
               FOREIGN KEY(session_id) REFERENCES sessions(session_id)
             );",
        )?;
        if current >= 1 {
            // Session protocols v2/v3 add new typed item variants. Existing
            // payloads remain wire-compatible, so migrate their explicit
            // record versions atomically with the database version marker.
            tx.execute(
                "UPDATE sessions
                 SET schema_version=?1,
                     record_json=json_set(record_json, '$.schema_version', ?1)",
                params![SESSION_SCHEMA_VERSION],
            )
            .context("session database v1 sessions schema is incomplete")?;
            tx.execute(
                "UPDATE session_items
                 SET item_json=json_set(item_json, '$.schema_version', ?1)",
                params![SESSION_SCHEMA_VERSION],
            )
            .context("session database v1 session_items schema is incomplete")?;
        }
        after_step(MigrationStep::Schema)?;
        tx.pragma_update(None, "user_version", DATABASE_SCHEMA_VERSION)?;
        after_step(MigrationStep::Version)?;
        tx.commit()?;
    }

    // A database that claims a supported version but has a partial or foreign
    // layout must fail closed before it is placed on a production path.
    connection
        .prepare("SELECT session_id,schema_version,record_json,next_sequence FROM sessions LIMIT 0")
        .context("session database v1 sessions schema is incomplete")?;
    connection
        .prepare("SELECT session_id,sequence,item_id,turn_id,item_json FROM session_items LIMIT 0")
        .context("session database v1 session_items schema is incomplete")?;
    connection
        .prepare("SELECT session_id,turn_id,classification FROM recovered_turns LIMIT 0")
        .context("session database v1 recovered_turns schema is incomplete")?;
    connection
        .prepare("SELECT session_id,principal_id FROM session_principals LIMIT 0")
        .context("session database v5 session_principals schema is incomplete")?;
    Ok(())
}

#[async_trait]
impl SessionProjectionStore for CanonicalSessionStore {
    async fn create(&self, session: SessionRecord) -> Result<()> {
        Self::validate_session(&session)?;
        let json = serde_json::to_string(&session)?;
        let connection = self.open_connection()?;
        let existing = connection
            .query_row(
                "SELECT record_json FROM sessions WHERE session_id=?1",
                params![session.id.0],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            if existing != json {
                bail!("session creation conflicts with persisted content");
            }
            return Ok(());
        }
        connection
            .execute(
                "INSERT INTO sessions(session_id,schema_version,record_json,next_sequence) VALUES(?1,?2,?3,1)",
                params![session.id.0, session.schema_version, json],
            )
            .context("create canonical session")?;
        Ok(())
    }

    async fn next_sequence(&self, session: &SessionId) -> Result<Option<u64>> {
        let connection = self.open_connection()?;
        connection
            .query_row(
                "SELECT next_sequence FROM sessions WHERE session_id=?1",
                params![session.0],
                |row| row.get::<_, u64>(0),
            )
            .optional()
            .map_err(anyhow::Error::from)
    }

    async fn item_by_id(&self, session: &SessionId, id: &ItemId) -> Result<Option<ItemRecord>> {
        let connection = self.open_connection()?;
        let json = connection
            .query_row(
                "SELECT item_json FROM session_items WHERE item_id=?1 AND session_id=?2",
                params![id.0.to_string(), session.0],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("read canonical session item by id")?;
        match json {
            Some(json) => {
                let item: ItemRecord = serde_json::from_str(&json)?;
                Ok(Some(item))
            }
            None => Ok(None),
        }
    }

    async fn append(
        &self,
        session: &SessionId,
        expected_sequence: u64,
        item: ItemRecord,
    ) -> Result<AppendOutcome> {
        Self::validate_item(session, expected_sequence, &item)?;
        let recovery_status = match &item.payload {
            ::contracts::ItemPayload::TurnRecovery {
                classification: ::contracts::TurnRecoveryClassification::Interrupted,
            } => Some(::contracts::SessionStatus::Interrupted),
            ::contracts::ItemPayload::TurnRecovery {
                classification: ::contracts::TurnRecoveryClassification::Failed,
            } => Some(::contracts::SessionStatus::Failed),
            _ => None,
        };
        let item_json = serde_json::to_string(&item)?;
        let mut connection = self.open_connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing_json) = tx
            .query_row(
                "SELECT item_json FROM session_items WHERE item_id=?1",
                params![item.id.0.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            // Compare the versioned record rather than its historical JSON
            // spelling. Compatible protocol additions can deserialize a missing
            // field to its typed default (for example execution_target=General),
            // so byte comparison would reject an otherwise identical replay and
            // keep the daemon in a restart loop. Rewrite successful retries to
            // today's canonical JSON so subsequent reads no longer depend on the
            // legacy representation.
            let mut existing: ItemRecord = serde_json::from_str(&existing_json)
                .with_context(|| format!("decode persisted Session item {}", item.id.0))?;
            if !(1..=SESSION_SCHEMA_VERSION).contains(&existing.schema_version) {
                bail!(
                    "persisted item {} has unsupported schema version {}",
                    item.id.0,
                    existing.schema_version
                );
            }
            existing.schema_version = SESSION_SCHEMA_VERSION;
            if existing != item {
                bail!(
                    "item id {} retry conflicts with persisted content",
                    item.id.0
                );
            }
            if existing_json != item_json {
                tx.execute(
                    "UPDATE session_items SET item_json=?2 WHERE item_id=?1",
                    params![item.id.0.to_string(), item_json],
                )?;
            }
            tx.commit()?;
            return Ok(AppendOutcome::AlreadyPresent);
        }
        let next: u64 = tx
            .query_row(
                "SELECT next_sequence FROM sessions WHERE session_id=?1",
                params![session.0],
                |row| row.get(0),
            )
            .context("canonical session not found")?;
        if next != expected_sequence {
            bail!("sequence conflict: expected {expected_sequence}, current {next}");
        }
        tx.execute(
            "INSERT INTO session_items(session_id,sequence,item_id,turn_id,item_json) VALUES(?1,?2,?3,?4,?5)",
            params![session.0, item.sequence, item.id.0.to_string(), item.turn_id.0.to_string(), item_json],
        )?;
        tx.execute(
            "UPDATE sessions SET next_sequence=?2 WHERE session_id=?1",
            params![session.0, next + 1],
        )?;
        if let Some(status) = recovery_status {
            let json: String = tx.query_row(
                "SELECT record_json FROM sessions WHERE session_id=?1",
                params![session.0],
                |row| row.get(0),
            )?;
            let mut record: SessionRecord = serde_json::from_str(&json)?;
            // Failed is the stronger aggregate state when more than one
            // incomplete turn is recovered from the same Session.
            if record.status != ::contracts::SessionStatus::Failed
                || status == ::contracts::SessionStatus::Failed
            {
                record.status = status;
                tx.execute(
                    "UPDATE sessions SET record_json=?2 WHERE session_id=?1",
                    params![session.0, serde_json::to_string(&record)?],
                )?;
            }
        }
        tx.commit()?;
        Ok(AppendOutcome::Appended)
    }

    async fn fork(
        &self,
        parent: &SessionId,
        through_sequence: u64,
        child: SessionRecord,
    ) -> Result<()> {
        Self::validate_session(&child)?;
        let expected_parent = child
            .parent
            .as_ref()
            .context("fork child missing parent metadata")?;
        if &expected_parent.session_id != parent
            || expected_parent.through_sequence != through_sequence
        {
            bail!("fork metadata does not match request");
        }
        let mut connection = self.open_connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let parent_next: u64 = tx.query_row(
            "SELECT next_sequence FROM sessions WHERE session_id=?1",
            params![parent.0],
            |r| r.get(0),
        )?;
        if through_sequence >= parent_next {
            bail!("parent sequence {through_sequence} does not exist");
        }
        tx.execute(
            "INSERT INTO sessions(session_id,schema_version,record_json,next_sequence) VALUES(?1,?2,?3,?4)",
            params![child.id.0, child.schema_version, serde_json::to_string(&child)?, through_sequence + 1],
        )?;
        let mut stmt = tx.prepare("SELECT item_json FROM session_items WHERE session_id=?1 AND sequence<=?2 ORDER BY sequence")?;
        let rows: Vec<String> = stmt
            .query_map(params![parent.0, through_sequence], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        for json in rows {
            let mut item: ItemRecord = serde_json::from_str(&json)?;
            item.id = ItemId::new();
            item.session_id = child.id.clone();
            tx.execute(
                "INSERT INTO session_items(session_id,sequence,item_id,turn_id,item_json) VALUES(?1,?2,?3,?4,?5)",
                params![child.id.0, item.sequence, item.id.0.to_string(), item.turn_id.0.to_string(), serde_json::to_string(&item)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    async fn bind_principal(&self, session: &SessionId, principal: &PrincipalId) -> Result<()> {
        let connection = self.open_connection()?;
        connection.execute(
            "INSERT INTO session_principals(session_id,principal_id) VALUES(?1,?2)
             ON CONFLICT(session_id) DO UPDATE SET principal_id=excluded.principal_id
             WHERE session_principals.principal_id=excluded.principal_id",
            params![session.0, principal.0],
        )?;
        let owner: String = connection.query_row(
            "SELECT principal_id FROM session_principals WHERE session_id=?1",
            params![session.0],
            |row| row.get(0),
        )?;
        if owner != principal.0 {
            bail!("session is already owned by another principal");
        }
        Ok(())
    }
}

#[async_trait]
impl SessionReadStore for CanonicalSessionStore {
    async fn load_session(&self, session: &SessionId) -> Result<Option<SessionRecord>> {
        let connection = self.open_connection()?;
        let json = connection
            .query_row(
                "SELECT record_json FROM sessions WHERE session_id=?1",
                params![session.0],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        json.map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }

    async fn load_items(&self, session: &SessionId, after: Option<u64>) -> Result<Vec<ItemRecord>> {
        let connection = self.open_connection()?;
        let mut stmt = connection.prepare(
            "SELECT item_json FROM session_items WHERE session_id=?1 AND sequence>?2 ORDER BY sequence"
        )?;
        let items = stmt
            .query_map(params![session.0, after.unwrap_or(0)], |r| {
                r.get::<_, String>(0)
            })?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect();
        items
    }

    async fn load_items_page(
        &self,
        session: &SessionId,
        after: Option<u64>,
        limit: usize,
    ) -> Result<Vec<ItemRecord>> {
        let connection = self.open_connection()?;
        let mut stmt = connection.prepare(
            "SELECT item_json FROM session_items
             WHERE session_id=?1 AND sequence>?2
             ORDER BY sequence LIMIT ?3",
        )?;
        let items = stmt
            .query_map(
                params![session.0, after.unwrap_or(0), limit.clamp(1, 10_000)],
                |row| row.get::<_, String>(0),
            )?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect();
        items
    }

    async fn list_sessions(&self, limit: usize) -> Result<Vec<SessionRecord>> {
        let connection = self.open_connection()?;
        let mut statement =
            connection.prepare("SELECT record_json FROM sessions ORDER BY rowid DESC LIMIT ?1")?;
        let rows = statement
            .query_map(params![limit.clamp(1, 1_000)], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|json| serde_json::from_str(&json).map_err(Into::into))
            .collect()
    }

    async fn list_sessions_with_principal(
        &self,
        limit: usize,
    ) -> Result<Vec<(SessionRecord, Option<PrincipalId>)>> {
        let connection = self.open_connection()?;
        let mut statement = connection.prepare(
            "SELECT s.record_json, p.principal_id
             FROM sessions s
             LEFT JOIN session_principals p ON p.session_id=s.session_id
             ORDER BY s.rowid DESC LIMIT ?1",
        )?;
        let rows = statement
            .query_map(params![limit.clamp(1, 1_000)], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(record, principal)| {
                Ok((serde_json::from_str(&record)?, principal.map(PrincipalId)))
            })
            .collect()
    }

    async fn list_session_ids(&self) -> Result<Vec<SessionId>> {
        let connection = self.open_connection()?;
        let mut statement =
            connection.prepare("SELECT session_id FROM sessions ORDER BY session_id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| row.map(SessionId).map_err(Into::into))
            .collect()
    }

    async fn principal_for(&self, session: &SessionId) -> Result<Option<PrincipalId>> {
        let connection = self.open_connection()?;
        let owner = connection
            .query_row(
                "SELECT principal_id FROM session_principals WHERE session_id=?1",
                params![session.0],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(owner.map(PrincipalId))
    }
}

pub use ::runtime::session_projection::project_messages;

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ::contracts::{ContentBlock, ItemPayload, Role};
    use runtime::turn_recovery::RecoveryClassification;

    const MIGRATION_STEPS: [MigrationStep; 2] = [MigrationStep::Schema, MigrationStep::Version];

    fn legacy_schema(connection: &Connection) {
        connection
            .execute_batch(
                "CREATE TABLE sessions(
                   session_id TEXT PRIMARY KEY,
                   schema_version INTEGER NOT NULL,
                   record_json TEXT NOT NULL,
                   next_sequence INTEGER NOT NULL
                 );
                 CREATE TABLE session_items(
                   session_id TEXT NOT NULL,
                   sequence INTEGER NOT NULL,
                   item_id TEXT NOT NULL UNIQUE,
                   turn_id TEXT NOT NULL,
                   item_json TEXT NOT NULL,
                   PRIMARY KEY(session_id, sequence),
                   FOREIGN KEY(session_id) REFERENCES sessions(session_id)
                 );
                 CREATE TABLE recovered_turns(
                   session_id TEXT NOT NULL,
                   turn_id TEXT NOT NULL,
                   classification TEXT NOT NULL,
                   PRIMARY KEY(session_id, turn_id),
                   FOREIGN KEY(session_id) REFERENCES sessions(session_id)
                 );",
            )
            .unwrap();
    }

    #[test]
    fn every_database_migration_step_failure_rolls_back_and_reopen_completes() {
        for failed_step in MIGRATION_STEPS {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("sessions.db");
            let connection = Connection::open(&path).unwrap();
            let error = migrate_with_step_hook(&connection, |step| {
                if step == failed_step {
                    bail!("injected failure at {step:?}");
                }
                Ok(())
            });
            assert!(error.is_err(), "step {failed_step:?} did not fail");
            assert_eq!(
                connection
                    .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                0,
                "version advanced at {failed_step:?}"
            );
            drop(connection);

            let reopened = CanonicalSessionStore::open(&path).unwrap();
            assert_eq!(
                reopened
                    .open_connection()
                    .unwrap()
                    .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                DATABASE_SCHEMA_VERSION
            );
        }
    }

    #[tokio::test]
    async fn legacy_unversioned_database_fixture_is_upgraded_without_record_changes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sessions.db");
        let connection = Connection::open(&path).unwrap();
        legacy_schema(&connection);
        let session = SessionRecord {
            schema_version: SESSION_SCHEMA_VERSION,
            id: SessionId("legacy-session".into()),
            parent: None,
            created_at_ms: 17,
            status: ::contracts::SessionStatus::Active,
        };
        connection
            .execute(
                "INSERT INTO sessions(session_id,schema_version,record_json,next_sequence) VALUES(?1,?2,?3,1)",
                params![session.id.0, session.schema_version, serde_json::to_string(&session).unwrap()],
            )
            .unwrap();
        drop(connection);

        let store = CanonicalSessionStore::open(&path).unwrap();
        assert_eq!(
            store
                .open_connection()
                .unwrap()
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            DATABASE_SCHEMA_VERSION
        );
        assert_eq!(
            store.load_session(&session.id).await.unwrap(),
            Some(session)
        );
    }

    #[tokio::test]
    async fn prior_session_records_are_atomically_upgraded_to_current_protocol() {
        for legacy_version in [1, 2, 3] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("sessions.db");
            let connection = Connection::open(&path).unwrap();
            legacy_schema(&connection);
            connection
                .pragma_update(None, "user_version", legacy_version)
                .unwrap();
            let session_id = SessionId(format!("v{legacy_version}-session"));
            let session_json = serde_json::json!({
                "schema_version": legacy_version,
                "id": session_id.0,
                "parent": null,
                "created_at_ms": 17,
                "status": "active"
            });
            let item_id = ::contracts::ItemId::new();
            let turn_id = ::contracts::TurnId::new();
            let item_json = serde_json::json!({
                "schema_version": legacy_version,
                "id": item_id,
                "session_id": session_id.0,
                "turn_id": turn_id,
                "sequence": 1,
                "created_at_ms": 18,
                "payload": {"type": "user_message", "data": {"content": "legacy"}}
            });
            connection.execute(
                "INSERT INTO sessions(session_id,schema_version,record_json,next_sequence) VALUES(?1,?2,?3,2)",
                params![session_id.0, legacy_version, session_json.to_string()],
            ).unwrap();
            connection.execute(
                "INSERT INTO session_items(session_id,sequence,item_id,turn_id,item_json) VALUES(?1,1,?2,?3,?4)",
                params![session_id.0, item_id.0.to_string(), turn_id.0.to_string(), item_json.to_string()],
            ).unwrap();
            drop(connection);

            let store = CanonicalSessionStore::open(&path).unwrap();
            let session = store.load_session(&session_id).await.unwrap().unwrap();
            let items = store.load_items(&session_id, None).await.unwrap();
            assert_eq!(session.schema_version, SESSION_SCHEMA_VERSION);
            assert_eq!(items[0].schema_version, SESSION_SCHEMA_VERSION);
            assert!(matches!(items[0].payload, ItemPayload::UserMessage { .. }));
        }
    }

    #[tokio::test]
    async fn compatible_defaulted_item_retry_is_idempotent_and_canonicalized() {
        let store = CanonicalSessionStore::open(":memory:").unwrap();
        let session_id = SessionId("defaulted-retry".into());
        store
            .create(SessionRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id: session_id.clone(),
                parent: None,
                created_at_ms: 17,
                status: ::contracts::SessionStatus::Active,
            })
            .await
            .unwrap();
        let item = ItemRecord {
            schema_version: SESSION_SCHEMA_VERSION,
            id: ItemId::new(),
            session_id: session_id.clone(),
            turn_id: ::contracts::TurnId::new(),
            sequence: 1,
            created_at_ms: 18,
            payload: ItemPayload::UserMessage {
                content: "legacy General turn".into(),
                execution_target: ::contracts::ExecutionTargetSelection::default(),
            },
        };
        let mut legacy_json = serde_json::to_value(&item).unwrap();
        legacy_json["payload"]["data"]
            .as_object_mut()
            .unwrap()
            .remove("execution_target");
        {
            let connection = store.open_connection().unwrap();
            connection
                .execute(
                    "INSERT INTO session_items(session_id,sequence,item_id,turn_id,item_json) \
                     VALUES(?1,1,?2,?3,?4)",
                    params![
                        session_id.0,
                        item.id.0.to_string(),
                        item.turn_id.0.to_string(),
                        legacy_json.to_string()
                    ],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE sessions SET next_sequence=2 WHERE session_id=?1",
                    params![session_id.0],
                )
                .unwrap();
        }

        assert_eq!(
            store.append(&session_id, 1, item.clone()).await.unwrap(),
            AppendOutcome::AlreadyPresent
        );
        let canonical_json: String = store
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT item_json FROM session_items WHERE item_id=?1",
                params![item.id.0.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<ItemRecord>(&canonical_json).unwrap(),
            item
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&canonical_json)
                .unwrap()
                .pointer("/payload/data/execution_target")
                .cloned(),
            Some(serde_json::json!({
                "target": {"kind": "general"},
                "source": "default"
            }))
        );

        let conflicting = ItemRecord {
            payload: ItemPayload::UserMessage {
                content: "different content".into(),
                execution_target: ::contracts::ExecutionTargetSelection::default(),
            },
            ..item
        };
        let error = store.append(&session_id, 1, conflicting).await.unwrap_err();
        assert!(error.to_string().contains("retry conflicts"));
    }

    #[tokio::test]
    async fn record_schema_validation_remains_independent_from_database_version() {
        let store = CanonicalSessionStore::open(":memory:").unwrap();
        let session_id = SessionId("unsupported-record".into());
        let error = store
            .create(SessionRecord {
                schema_version: SESSION_SCHEMA_VERSION + 1,
                id: session_id.clone(),
                parent: None,
                created_at_ms: 0,
                status: ::contracts::SessionStatus::Active,
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unsupported session schema"));

        store
            .create(SessionRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id: session_id.clone(),
                parent: None,
                created_at_ms: 0,
                status: ::contracts::SessionStatus::Active,
            })
            .await
            .unwrap();
        let error = store
            .append(
                &session_id,
                1,
                ItemRecord {
                    schema_version: SESSION_SCHEMA_VERSION + 1,
                    id: ItemId::new(),
                    session_id: session_id.clone(),
                    turn_id: ::contracts::TurnId::new(),
                    sequence: 1,
                    created_at_ms: 0,
                    payload: ItemPayload::UserMessage {
                        content: "unsupported".into(),
                        execution_target: ::contracts::ExecutionTargetSelection::default(),
                    },
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unsupported item schema"));
    }

    #[test]
    fn newer_or_incomplete_claimed_database_schema_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let newer = temp.path().join("newer.db");
        let connection = Connection::open(&newer).unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
        drop(connection);
        let error = match CanonicalSessionStore::open(&newer) {
            Ok(_) => panic!("newer database unexpectedly opened"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("newer than supported"));

        let incomplete = temp.path().join("incomplete.db");
        let connection = Connection::open(&incomplete).unwrap();
        connection
            .execute_batch("CREATE TABLE sessions(session_id TEXT); PRAGMA user_version=1;")
            .unwrap();
        drop(connection);
        let error = match CanonicalSessionStore::open(&incomplete) {
            Ok(_) => panic!("incomplete database unexpectedly opened"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("sessions schema is incomplete"));
    }

    async fn create_incomplete_turn(
        store: &dyn ::contracts::SessionAppendStore,
        session_id: &str,
    ) -> ::contracts::TurnId {
        let session_id = SessionId(session_id.into());
        store
            .create(SessionRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id: session_id.clone(),
                parent: None,
                created_at_ms: 0,
                status: ::contracts::SessionStatus::Active,
            })
            .await
            .unwrap();
        let turn_id = ::contracts::TurnId::new();
        store
            .append(
                &session_id,
                1,
                ItemRecord {
                    schema_version: SESSION_SCHEMA_VERSION,
                    id: ItemId::new(),
                    session_id: session_id.clone(),
                    turn_id,
                    sequence: 1,
                    created_at_ms: 0,
                    payload: ItemPayload::UserMessage {
                        content: "started".into(),
                        execution_target: ::contracts::ExecutionTargetSelection::default(),
                    },
                },
            )
            .await
            .unwrap();
        turn_id
    }

    #[tokio::test]
    async fn recovery_fact_persists_through_the_single_session_authority() {
        let read_model = std::sync::Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
        let store = crate::session::event_sourced_store::EventSourcedSessionStore::in_memory(
            read_model.clone(),
        );
        let session_id = SessionId("recovery-test".into());
        create_incomplete_turn(store.as_ref(), &session_id.0).await;
        let report = runtime::turn_recovery::scan_incomplete_turns(store.as_ref(), true)
            .await
            .unwrap();

        assert_eq!(report.incomplete_turns.len(), 1);
        assert_eq!(
            read_model
                .load_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            ::contracts::SessionStatus::Interrupted
        );
        let items = store.load_items(&session_id, None).await.unwrap();
        assert!(matches!(
            items.last().map(|item| &item.payload),
            Some(ItemPayload::TurnRecovery {
                classification: ::contracts::TurnRecoveryClassification::Interrupted
            })
        ));
    }

    #[tokio::test]
    async fn startup_recovery_enumerates_all_durable_sessions() {
        let read_model = std::sync::Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
        let store = crate::session::event_sourced_store::EventSourcedSessionStore::in_memory(
            read_model.clone(),
        );
        create_incomplete_turn(store.as_ref(), "session-a").await;
        create_incomplete_turn(store.as_ref(), "session-b").await;
        let report = runtime::turn_recovery::scan_incomplete_turns(store.as_ref(), true)
            .await
            .unwrap();

        assert_eq!(report.sessions_scanned, 2);
        assert_eq!(report.turns_scanned, 2);
        assert_eq!(report.incomplete_turns.len(), 2);
        for session in ["session-a", "session-b"] {
            assert_eq!(
                read_model
                    .load_session(&SessionId(session.into()))
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                ::contracts::SessionStatus::Interrupted
            );
        }
    }

    #[test]
    fn projection_hides_orphan_tool_result() {
        let item = ItemRecord {
            schema_version: SESSION_SCHEMA_VERSION,
            id: ItemId::new(),
            session_id: SessionId("s".into()),
            turn_id: ::contracts::TurnId::new(),
            sequence: 1,
            created_at_ms: 0,
            payload: ItemPayload::ToolResult {
                call_id: "missing".into(),
                content: "output".into(),
                is_error: false,
                permit_id: None,
                audit_id: None,
            },
        };
        let messages = project_messages(&[item]).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::System);
    }

    #[derive(Clone, Copy)]
    enum CrashBoundary {
        Streaming,
        Tool,
        Compaction,
        TerminalPersist,
    }

    async fn persist_until_crash(path: &Path, boundary: CrashBoundary, session: &str) {
        let store = CanonicalSessionStore::open(path).unwrap();
        let session_id = SessionId(session.into());
        store
            .create(SessionRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id: session_id.clone(),
                parent: None,
                created_at_ms: 0,
                status: ::contracts::SessionStatus::Active,
            })
            .await
            .unwrap();
        let turn_id = ::contracts::TurnId::new();
        let mut payloads = vec![ItemPayload::UserMessage {
            content: "started".into(),
            execution_target: ::contracts::ExecutionTargetSelection::default(),
        }];
        match boundary {
            CrashBoundary::Streaming => {}
            CrashBoundary::Tool => payloads.push(ItemPayload::ToolCall {
                call_id: "call-1".into(),
                name: "bash".into(),
                input: serde_json::json!({"command":"true"}),
            }),
            CrashBoundary::Compaction => payloads.push(ItemPayload::ContextProjection {
                space: "workspace".into(),
                broadcast_epoch: Some(7),
                workspace_version: Some(3),
                dasein_version: 4,
                content_ids: vec!["fragment-1".into()],
            }),
            // The writer received a result but crashed before its terminal
            // append. Keep the deliberately orphaned result durable so reopen
            // also proves the model projection cannot expose it as a tool result.
            CrashBoundary::TerminalPersist => payloads.push(ItemPayload::ToolResult {
                call_id: "missing-call".into(),
                content: "must-not-be-exposed".into(),
                is_error: false,
                permit_id: None,
                audit_id: None,
            }),
        }
        for (index, payload) in payloads.into_iter().enumerate() {
            let sequence = index as u64 + 1;
            store
                .append(
                    &session_id,
                    sequence,
                    ItemRecord {
                        schema_version: SESSION_SCHEMA_VERSION,
                        id: ItemId::new(),
                        session_id: session_id.clone(),
                        turn_id,
                        sequence,
                        created_at_ms: sequence,
                        payload,
                    },
                )
                .await
                .unwrap();
        }
        // Dropping the only connection is the deterministic crash boundary.
    }

    #[tokio::test]
    async fn reopen_recovers_each_m4_crash_boundary_without_false_terminal() {
        for (index, boundary) in [
            CrashBoundary::Streaming,
            CrashBoundary::Tool,
            CrashBoundary::Compaction,
            CrashBoundary::TerminalPersist,
        ]
        .into_iter()
        .enumerate()
        {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("sessions.db");
            let session = format!("crash-{index}");
            persist_until_crash(&path, boundary, &session).await;

            let reopened = std::sync::Arc::new(CanonicalSessionStore::open(&path).unwrap());
            let authority =
                crate::session::event_sourced_store::EventSourcedSessionStore::in_memory(
                    reopened.clone(),
                );
            let report = runtime::turn_recovery::scan_incomplete_turns(authority.as_ref(), true)
                .await
                .unwrap();
            assert_eq!(report.incomplete_turns.len(), 1);
            let expected = if matches!(boundary, CrashBoundary::Tool) {
                RecoveryClassification::Failed
            } else {
                RecoveryClassification::Interrupted
            };
            assert_eq!(report.incomplete_turns[0].classification, expected);
            assert_ne!(
                reopened
                    .load_session(&SessionId(session.clone()))
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                ::contracts::SessionStatus::Active
            );

            let items = reopened
                .load_items(&SessionId(session), None)
                .await
                .unwrap();
            assert!(!items.iter().any(|item| matches!(
                item.payload,
                ItemPayload::AssistantMessage { .. } | ItemPayload::SystemNotice { .. }
            )));
            if matches!(boundary, CrashBoundary::TerminalPersist) {
                let projected = project_messages(&items).unwrap();
                assert!(!projected.iter().any(|message| {
                    message.content.iter().any(|block| {
                        matches!(block, ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "missing-call")
                    })
                }));
            }
        }
    }
}

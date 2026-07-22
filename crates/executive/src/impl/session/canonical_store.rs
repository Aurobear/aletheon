//! Canonical transactional Session/Turn/Item history store.

use std::{path::Path, sync::Mutex};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use fabric::{
    AppendOutcome, ContentBlock, ItemId, ItemPayload, ItemRecord, Message, Role,
    SessionAppendStore, SessionId, SessionRecord, SESSION_SCHEMA_VERSION,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::service::turn_recovery::{RecoveryClassification, TurnRecoveryStore};

pub struct CanonicalSessionStore {
    connection: Mutex<Connection>,
}

const DATABASE_SCHEMA_VERSION: i64 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MigrationStep {
    Schema,
    Version,
}

pub fn default_session_db_path() -> std::path::PathBuf {
    fabric::paths::xdg_data_dir().join("sessions-v1.db")
}

/// Canonical session database for an explicitly owned user state root.
pub fn session_db_path(state_root: &Path) -> std::path::PathBuf {
    state_root.join("sessions-v1.db")
}

impl CanonicalSessionStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path)?;
        migrate(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
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
        Ok(())
    }
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
             );",
        )?;
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
    Ok(())
}

#[async_trait]
impl TurnRecoveryStore for CanonicalSessionStore {
    async fn list_session_ids(&self) -> Result<Vec<SessionId>> {
        let connection = self.connection.lock().unwrap();
        let mut statement =
            connection.prepare("SELECT session_id FROM sessions ORDER BY session_id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| row.map(SessionId).map_err(Into::into))
            .collect()
    }

    async fn mark_recovered_turn(
        &self,
        session_id: &SessionId,
        turn_id: fabric::TurnId,
        classification: RecoveryClassification,
    ) -> Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let json: String = tx
            .query_row(
                "SELECT record_json FROM sessions WHERE session_id=?1",
                params![session_id.0],
                |row| row.get(0),
            )
            .context("recovery session not found")?;
        let mut session: SessionRecord = serde_json::from_str(&json)?;
        let (classification_name, status) = match classification {
            RecoveryClassification::Interrupted => {
                ("interrupted", fabric::SessionStatus::Interrupted)
            }
            RecoveryClassification::Failed => ("failed", fabric::SessionStatus::Failed),
        };
        // Failed is the stronger aggregate state when a session has more than
        // one incomplete turn.
        if session.status != fabric::SessionStatus::Failed
            || status == fabric::SessionStatus::Failed
        {
            session.status = status;
        }
        tx.execute(
            "UPDATE sessions SET record_json=?2 WHERE session_id=?1",
            params![session_id.0, serde_json::to_string(&session)?],
        )?;
        tx.execute(
            "INSERT INTO recovered_turns(session_id,turn_id,classification) VALUES(?1,?2,?3)
             ON CONFLICT(session_id,turn_id) DO UPDATE SET classification=excluded.classification",
            params![session_id.0, turn_id.0.to_string(), classification_name],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[async_trait]
impl SessionAppendStore for CanonicalSessionStore {
    async fn create(&self, session: SessionRecord) -> Result<()> {
        Self::validate_session(&session)?;
        let json = serde_json::to_string(&session)?;
        let connection = self.connection.lock().unwrap();
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

    async fn append(
        &self,
        session: &SessionId,
        expected_sequence: u64,
        item: ItemRecord,
    ) -> Result<AppendOutcome> {
        Self::validate_item(session, expected_sequence, &item)?;
        let item_json = serde_json::to_string(&item)?;
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = tx
            .query_row(
                "SELECT item_json FROM session_items WHERE item_id=?1",
                params![item.id.0.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            if existing != item_json {
                bail!("item id retry conflicts with persisted content");
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
        let mut connection = self.connection.lock().unwrap();
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

    async fn load_session(&self, session: &SessionId) -> Result<Option<SessionRecord>> {
        let json = self
            .connection
            .lock()
            .unwrap()
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
        let connection = self.connection.lock().unwrap();
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
}

pub fn project_messages(items: &[ItemRecord]) -> Result<Vec<Message>> {
    let mut previous = 0;
    let mut messages = Vec::new();
    for item in items {
        if item.sequence <= previous {
            bail!(
                "items are duplicate or out of order at sequence {}",
                item.sequence
            );
        }
        previous = item.sequence;
    }
    // Canonical records remain immutable. Normalize only the model-facing
    // projection so an orphan result is never exposed during resume/replay.
    let normalized = crate::service::compaction_normalize::normalize_tool_pairs(
        items.iter().map(|item| item.payload.clone()).collect(),
    );
    for payload in &normalized.items {
        let message = match payload {
            ItemPayload::UserMessage { content } => Some(Message::user(content)),
            ItemPayload::AssistantMessage { content } => Some(Message::assistant(content)),
            ItemPayload::SystemNotice { content } => Some(Message::system(content)),
            ItemPayload::ToolCall {
                call_id,
                name,
                input,
            } => Some(Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolUse {
                    id: call_id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                }],
            }),
            ItemPayload::ToolResult {
                call_id,
                content,
                is_error,
                ..
            } => Some(Message::tool_result(call_id, content, *is_error)),
            ItemPayload::ContextProjection { .. } => None,
        };
        if let Some(message) = message {
            messages.push(message);
        }
    }
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

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
                    .connection
                    .lock()
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
            status: fabric::SessionStatus::Active,
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
                .connection
                .lock()
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
    async fn record_schema_validation_remains_independent_from_database_version() {
        let store = CanonicalSessionStore::open(":memory:").unwrap();
        let session_id = SessionId("unsupported-record".into());
        let error = store
            .create(SessionRecord {
                schema_version: SESSION_SCHEMA_VERSION + 1,
                id: session_id.clone(),
                parent: None,
                created_at_ms: 0,
                status: fabric::SessionStatus::Active,
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
                status: fabric::SessionStatus::Active,
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
                    turn_id: fabric::TurnId::new(),
                    sequence: 1,
                    created_at_ms: 0,
                    payload: ItemPayload::UserMessage {
                        content: "unsupported".into(),
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
        store: &CanonicalSessionStore,
        session_id: &str,
    ) -> fabric::TurnId {
        let session_id = SessionId(session_id.into());
        store
            .create(SessionRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id: session_id.clone(),
                parent: None,
                created_at_ms: 0,
                status: fabric::SessionStatus::Active,
            })
            .await
            .unwrap();
        let turn_id = fabric::TurnId::new();
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
                    },
                },
            )
            .await
            .unwrap();
        turn_id
    }

    #[tokio::test]
    async fn recovery_mutation_persists_turn_and_session_status() {
        let store = CanonicalSessionStore::open(":memory:").unwrap();
        let session_id = SessionId("recovery-test".into());
        store
            .create(SessionRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id: session_id.clone(),
                parent: None,
                created_at_ms: 0,
                status: fabric::SessionStatus::Active,
            })
            .await
            .unwrap();
        let turn_id = fabric::TurnId::new();
        store
            .mark_recovered_turn(&session_id, turn_id, RecoveryClassification::Interrupted)
            .await
            .unwrap();

        assert_eq!(
            store
                .load_session(&session_id)
                .await
                .unwrap()
                .unwrap()
                .status,
            fabric::SessionStatus::Interrupted
        );
        let persisted: String = store
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT classification FROM recovered_turns WHERE session_id=?1 AND turn_id=?2",
                params![session_id.0, turn_id.0.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(persisted, "interrupted");
    }

    #[tokio::test]
    async fn startup_recovery_enumerates_all_durable_sessions() {
        let store = CanonicalSessionStore::open(":memory:").unwrap();
        create_incomplete_turn(&store, "session-a").await;
        create_incomplete_turn(&store, "session-b").await;
        let mut hardening = crate::core::config::GrokHardeningConfig::default();
        hardening.compaction_v2 = true;

        let report = crate::service::turn_recovery::scan_incomplete_turns(&store, &hardening)
            .await
            .unwrap();

        assert_eq!(report.sessions_scanned, 2);
        assert_eq!(report.turns_scanned, 2);
        assert_eq!(report.incomplete_turns.len(), 2);
        for session in ["session-a", "session-b"] {
            assert_eq!(
                store
                    .load_session(&SessionId(session.into()))
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                fabric::SessionStatus::Interrupted
            );
        }
    }

    #[test]
    fn projection_hides_orphan_tool_result() {
        let item = ItemRecord {
            schema_version: SESSION_SCHEMA_VERSION,
            id: ItemId::new(),
            session_id: SessionId("s".into()),
            turn_id: fabric::TurnId::new(),
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
                status: fabric::SessionStatus::Active,
            })
            .await
            .unwrap();
        let turn_id = fabric::TurnId::new();
        let mut payloads = vec![ItemPayload::UserMessage {
            content: "started".into(),
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

            let reopened = CanonicalSessionStore::open(&path).unwrap();
            let mut hardening = crate::core::config::GrokHardeningConfig::default();
            hardening.compaction_v2 = true;
            let report =
                crate::service::turn_recovery::scan_incomplete_turns(&reopened, &hardening)
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
                fabric::SessionStatus::Active
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

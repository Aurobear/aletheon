//! SessionInfrastructure (RA-03 design) — canonical session store + writer.
//!
//! Constructed once, after `RuntimeJournalResources`, and injected into
//! `build_turn_services` / the session gateway.  It owns:
//!   - the canonical store (`CanonicalSessionStore` over `sessions-v1.db`),
//!   - the composed `SessionAppendStore`,
//!   - the `RuntimeSessionWriter` (Runtime mints canonical SessionId + appends
//!     SessionCreated).
//!
//! It does **not** own the spine/projections (those are
//! `RuntimeJournalResources`).  One writer, one store, no second composition
//! root.

use anyhow::Context;
use std::sync::Arc;

use crate::config::SessionWriterMode;
use adapters_sqlite::session::canonical_store::CanonicalSessionStore;

use super::journal_resources::RuntimeJournalResources;

/// Canonical session infrastructure: store + append store + Runtime writer.
pub struct SessionInfrastructure {
    append_store: Arc<dyn ::contracts::SessionAppendStore>,
    writer: Arc<runtime::RuntimeSessionWriter>,
}

/// Compose the shared journal, canonical session infrastructure, and initial
/// session as one RA-03 bootstrap transaction. Callers receive the exact
/// instances that later services must reuse; no second composition root is
/// allowed to reopen either database or spine.
pub async fn open_daemon_session(
    data_dir: &std::path::Path,
    max_event_spine_bytes: Option<u64>,
    clock: Arc<dyn ::contracts::Clock>,
    writer_mode: SessionWriterMode,
) -> anyhow::Result<(RuntimeJournalResources, SessionInfrastructure, String)> {
    let journal = RuntimeJournalResources::open(data_dir, max_event_spine_bytes)
        .context("open shared event journal resources")?;
    let infrastructure = SessionInfrastructure::open(data_dir, &journal, clock, writer_mode)
        .await
        .context("open canonical session infrastructure")?;
    let session_id = create_initial_session(data_dir, &infrastructure, writer_mode).await?;
    Ok((journal, infrastructure, session_id))
}

impl SessionInfrastructure {
    /// Open the canonical session store (`sessions-v1.db`) and compose the
    /// append store over the shared journal resources.  The Runtime writer
    /// uses this store so there is exactly one writer.
    pub async fn open(
        data_dir: &std::path::Path,
        journal: &RuntimeJournalResources,
        clock: Arc<dyn ::contracts::Clock>,
        writer_mode: SessionWriterMode,
    ) -> anyhow::Result<Self> {
        let canonical_store = Arc::new(
            CanonicalSessionStore::open(data_dir.join("sessions-v1.db"))
                .context("open durable Session read model")?,
        );
        let append_store = crate::composition::turn_coordinator::compose_session_store(
            canonical_store.clone(),
            journal.spine(),
            journal.projections(),
        );
        if writer_mode == SessionWriterMode::Runtime {
            migrate_legacy_sessions(data_dir, append_store.clone()).await?;
        }
        // Reconcile committed spine events before exposing the writer. A
        // daemon restart must repair the materialized read model before any
        // new SessionCreated command is admitted.
        let recovery =
            adapters_sqlite::session::event_sourced_store::reconcile_committed_session_events(
                journal.spine().as_ref(),
                journal.projections().as_ref(),
                canonical_store.as_ref(),
            )
            .await
            .context("reconcile committed Session events during daemon startup")?;
        tracing::info!(
            scanned = recovery.scanned,
            materialized = recovery.materialized,
            "Session event-spine recovery completed before Session writer admission"
        );
        let shadow = Arc::new(runtime::RuntimeJournalShadow::new(journal.spine()));
        let writer = Arc::new(runtime::RuntimeSessionWriter::new(
            runtime::SessionAuthority::new(shadow),
            append_store.clone(),
            clock,
        ));
        Ok(Self {
            append_store,
            writer,
        })
    }

    pub fn append_store(&self) -> Arc<dyn ::contracts::SessionAppendStore> {
        self.append_store.clone()
    }

    pub fn writer(&self) -> Arc<runtime::RuntimeSessionWriter> {
        self.writer.clone()
    }
}

/// Create the daemon's initial session through the canonical Runtime writer.
/// The writer-mode flag only selects compatibility reads; it must never reopen
/// or append to the historical `sessions.db`.
pub async fn create_initial_session(
    _data_dir: &std::path::Path,
    infrastructure: &SessionInfrastructure,
    _writer_mode: SessionWriterMode,
) -> anyhow::Result<String> {
    let receipt = infrastructure
        .writer()
        .create_session(None, None)
        .await
        .context("runtime session writer create failed")?;
    Ok(receipt
        .session
        .expect("runtime session receipt has a session")
        .0)
}

async fn migrate_legacy_sessions(
    data_dir: &std::path::Path,
    canonical: Arc<dyn ::contracts::SessionAppendStore>,
) -> anyhow::Result<()> {
    let path = data_dir.join("sessions.db");
    if !path.exists() {
        return Ok(());
    }
    let legacy = adapters_sqlite::session::store::SessionStore::open_read_only(&path)
        .context("open legacy sessions database read-only")?;
    let active = Arc::new(runtime::ActiveTurnRegistry::new());
    let migrator = runtime::session_service::SessionService::new(canonical, active);
    for record in legacy.list_records()? {
        let id = ::contracts::SessionId(record.session_id.clone());
        if migrator.try_resume(&id).await?.is_some() {
            continue;
        }
        let messages: Vec<::contracts::Message> = serde_json::from_str(&record.messages_json)
            .with_context(|| format!("decode legacy session {} messages", record.session_id))?;
        migrator
            .ensure_legacy_projection(&id, &messages, 0)
            .await
            .with_context(|| format!("migrate legacy session {}", record.session_id))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::SchemaId;
    use kernel::chronos::TestClock;
    use runtime::event_spine::EventSpine;

    #[tokio::test]
    async fn runtime_writer_uses_the_shared_durable_store_and_spine() {
        let temp = tempfile::tempdir().unwrap();
        let journal = RuntimeJournalResources::open(temp.path(), None).unwrap();
        let infrastructure = SessionInfrastructure::open(
            temp.path(),
            &journal,
            Arc::new(TestClock::default()),
            SessionWriterMode::Runtime,
        )
        .await
        .unwrap();

        let first = infrastructure
            .writer()
            .create_session(None, None)
            .await
            .unwrap()
            .session
            .unwrap();
        let second = infrastructure
            .writer()
            .create_session(None, None)
            .await
            .unwrap()
            .session
            .unwrap();
        assert_ne!(first, second);

        for id in [&first, &second] {
            let stored = infrastructure
                .append_store()
                .load_session(&::contracts::SessionId(id.0.clone()))
                .await
                .unwrap()
                .expect("Runtime-created session must be materialized");
            assert_eq!(stored.id.0, id.0);
        }

        let committed = journal
            .spine()
            .read_committed_page(0, i64::MAX as u64, 16)
            .unwrap();
        let created_count = committed
            .iter()
            .filter(|(_, event)| event.schema.0 == SchemaId::EVENT_SESSION_CREATED_V1)
            .count();
        assert_eq!(created_count, 2);

        let forked = infrastructure
            .writer()
            .fork_session(&first, 0)
            .await
            .unwrap()
            .session
            .unwrap();
        let fork_record = infrastructure
            .append_store()
            .load_session(&::contracts::SessionId(forked.0.clone()))
            .await
            .unwrap()
            .expect("Runtime fork must materialize child session");
        assert_eq!(
            fork_record
                .parent
                .as_ref()
                .map(|parent| parent.session_id.0.as_str()),
            Some(first.0.as_str())
        );

        // Reopening the composition root models a daemon restart: the durable
        // record remains readable and a new authority does not recycle an ID.
        drop(infrastructure);
        drop(journal);
        let reopened_journal = RuntimeJournalResources::open(temp.path(), None).unwrap();
        let reopened = SessionInfrastructure::open(
            temp.path(),
            &reopened_journal,
            Arc::new(TestClock::default()),
            SessionWriterMode::Runtime,
        )
        .await
        .unwrap();
        assert!(reopened
            .append_store()
            .load_session(&::contracts::SessionId(first.0.clone()))
            .await
            .unwrap()
            .is_some());
        let after_restart = reopened
            .writer()
            .create_session(None, None)
            .await
            .unwrap()
            .session
            .unwrap();
        assert_ne!(after_restart, first);
        assert_ne!(after_restart, second);
    }

    #[tokio::test]
    async fn runtime_cutover_migrates_legacy_records_without_writing_legacy_db() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = adapters_sqlite::session::store::SessionStore::open(
            temp.path().join("sessions.db").as_path(),
        )
        .unwrap();
        legacy
            .save(
                "legacy-session",
                &serde_json::to_string(&vec![::contracts::Message::user("migrate me")]).unwrap(),
                "{}",
            )
            .unwrap();
        drop(legacy);

        let journal = RuntimeJournalResources::open(temp.path(), None).unwrap();
        let infrastructure = SessionInfrastructure::open(
            temp.path(),
            &journal,
            Arc::new(TestClock::default()),
            SessionWriterMode::Runtime,
        )
        .await
        .unwrap();
        let migrated = infrastructure
            .append_store()
            .load_items(&::contracts::SessionId("legacy-session".into()), None)
            .await
            .unwrap();
        assert!(!migrated.is_empty());
        // Runtime mode only reads the legacy file; it never creates a second
        // legacy session row during the migration.
        let legacy_after = adapters_sqlite::session::store::SessionStore::open_read_only(
            &temp.path().join("sessions.db"),
        )
        .unwrap();
        assert_eq!(
            legacy_after.list_sessions().unwrap(),
            vec!["legacy-session"]
        );
    }
}

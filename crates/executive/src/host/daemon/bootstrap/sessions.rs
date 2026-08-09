//! Typed construction unit for daemon session state.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use fabric::{Clock, MonoTime};
use tokio::sync::Mutex;

use super::super::session_manager::SessionManager;

pub(super) struct SessionCompositionInput<'a> {
    pub(super) data_dir: &'a Path,
    pub(super) session_id: String,
    pub(super) context_window: usize,
    pub(super) compaction_threshold_percent: usize,
    pub(super) clock: Arc<dyn Clock>,
    /// RA-03 PR-C gate: true → Runtime SessionAuthority mints + appends;
    /// false (default) → legacy SessionStore path stays authoritative.
    pub(super) session_writer_runtime: bool,
}

pub(super) struct SessionComposition {
    pub(super) initial: Arc<Mutex<SessionManager>>,
    pub(super) registry: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
    pub(super) default_id: Arc<Mutex<String>>,
    pub(super) created_at: Arc<Mutex<HashMap<String, MonoTime>>>,
}

pub(super) async fn compose(
    input: SessionCompositionInput<'_>,
) -> anyhow::Result<SessionComposition> {
    anyhow::ensure!(!input.session_id.is_empty(), "session id must not be empty");
    anyhow::ensure!(input.context_window > 0, "context window must be non-zero");

    // RA-03 PR-C gate: when enabled, the Runtime SessionAuthority mints the
    // canonical SessionId + appends SessionCreated through the shared store,
    // and the legacy SessionStore path is skipped for the initial session.
    // Default legacy keeps the current writer authoritative until the
    // maintenance-window switch.
    let runtime_session_id = if input.session_writer_runtime {
        let canonical_store =
            crate::adapters::session::canonical_store::CanonicalSessionStore::open(
                input.data_dir.join("sessions.db"),
            )?;
        let event_spine: Arc<dyn fabric::EventSpine> = Arc::new(
            crate::adapters::events::SqliteEventSpine::open(input.data_dir.join("event-spine.db"))?,
        );
        let projections = Arc::new(
            crate::adapters::events::projection_set::DefaultEventProjectionSet::open(
                input.data_dir.join("projections.db"),
            )
            .map_err(|e| anyhow::anyhow!("projection set open failed: {e}"))?,
        );
        let store: Arc<dyn fabric::SessionAppendStore> =
            crate::composition::turn_coordinator::compose_session_store(
                Arc::new(canonical_store),
                event_spine.clone(),
                projections,
            );
        let shadow = Arc::new(runtime::RuntimeJournalShadow::new(event_spine));
        let writer =
            runtime::RuntimeSessionWriter::new(runtime::SessionAuthority::new(shadow), store);
        let receipt = writer
            .create_session(None, None)
            .await
            .map_err(|e| anyhow::anyhow!("runtime session writer failed: {e:?}"))?;
        Some(
            receipt
                .session
                .expect("runtime session receipt has a session"),
        )
    } else {
        None
    };

    let initial_session_id = runtime_session_id
        .as_ref()
        .map(|s| s.0.clone())
        .unwrap_or_else(|| input.session_id.clone());
    let initial = Arc::new(Mutex::new(
        SessionManager::new(
            input.data_dir,
            initial_session_id.clone(),
            input.context_window,
            input.compaction_threshold_percent,
            input.clock.clone(),
        )
        .await?,
    ));
    let registry = Arc::new(Mutex::new(HashMap::from([(
        initial_session_id.clone(),
        initial.clone(),
    )])));
    let default_id = Arc::new(Mutex::new(initial_session_id));
    let created_at = Arc::new(Mutex::new(HashMap::from([(
        input.session_id,
        input.clock.mono_now(),
    )])));

    Ok(SessionComposition {
        initial,
        registry,
        default_id,
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn composes_all_session_state_from_typed_inputs() {
        let root = tempfile::tempdir().unwrap();
        let composition = compose(SessionCompositionInput {
            data_dir: root.path(),
            session_id: "session-1".into(),
            context_window: 4096,
            compaction_threshold_percent: 80,
            clock: Arc::new(kernel::chronos::TestClock::new(100, 0)),
            session_writer_runtime: false,
        })
        .await
        .unwrap();

        assert!(composition.registry.lock().await.contains_key("session-1"));
        assert_eq!(&*composition.default_id.lock().await, "session-1");
        assert_eq!(
            composition.created_at.lock().await.get("session-1"),
            Some(&MonoTime(0))
        );
        assert!(Arc::ptr_eq(
            &composition.initial,
            composition.registry.lock().await.get("session-1").unwrap()
        ));
    }

    #[tokio::test]
    async fn reports_session_storage_construction_failure() {
        let root = tempfile::tempdir().unwrap();

        assert!(compose(SessionCompositionInput {
            data_dir: root.path(),
            session_id: String::new(),
            context_window: 0,
            compaction_threshold_percent: 80,
            clock: Arc::new(kernel::chronos::TestClock::new(100, 0)),
            session_writer_runtime: false,
        })
        .await
        .is_err());
    }
}

//! Typed construction unit for daemon session state.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use ::contracts::{Clock, MonoTime};
use tokio::sync::Mutex;

use super::super::context_working_set::ContextWorkingSet;

pub(super) struct SessionCompositionInput<'a> {
    pub(super) data_dir: &'a Path,
    pub(super) session_id: String,
    pub(super) context_window: usize,
    pub(super) compaction_threshold_percent: usize,
    pub(super) clock: Arc<dyn Clock>,
}

pub(super) struct SessionComposition {
    pub(super) initial: Arc<Mutex<ContextWorkingSet>>,
    pub(super) registry: Arc<Mutex<HashMap<String, Arc<Mutex<ContextWorkingSet>>>>>,
    pub(super) default_id: Arc<Mutex<String>>,
    pub(super) created_at: Arc<Mutex<HashMap<String, MonoTime>>>,
}

pub(super) async fn compose(
    input: SessionCompositionInput<'_>,
) -> anyhow::Result<SessionComposition> {
    anyhow::ensure!(!input.session_id.is_empty(), "session id must not be empty");
    anyhow::ensure!(input.context_window > 0, "context window must be non-zero");

    // RA-03: the canonical SessionId is minted by the unique
    // SessionInfrastructure BEFORE this compose runs (request.rs).  This
    // function is pure-memory: it never opens a DB or writes a session.  The
    // `session_writer` mode is used only for the initial id (legacy uuid vs
    // Runtime minted), both already resolved upstream.
    let initial_session_id = input.session_id.clone();
    let initial = Arc::new(Mutex::new(
        super::super::context_working_set::compose_context_working_set(
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
        })
        .await
        .is_err());
    }
}

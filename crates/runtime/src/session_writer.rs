//! RA-03 PR-C Runtime Session writer (Agent Kernel V2).
//!
//! The deployable Session writer: the Runtime mints the canonical `SessionId`
//! via `SessionAuthority`, appends the `SessionCreated` journal event through
//! the real `SessionAppendStore` (the same durable store the legacy path uses
//! — a single writer via a shared store), and returns the receipt.  The legacy
//! `SessionService` becomes a one-way facade; old entries return typed
//! retired/wrong-generation after the cutover.  This is the PR-C code; the
//! actual maintenance/drain + installed acceptance is the deployment slice.

use crate::command::{CommandReceipt, CreateSessionCommand, ForkSessionCommand, RuntimeCommand};
use crate::error::RuntimeError;
use crate::event::RuntimeEvent;
use crate::ids::{Generation, SessionId};
use crate::ports::RuntimeCommandPort;
use crate::session_authority::SessionAuthority;
use async_trait::async_trait;
use std::sync::Arc;

/// A typed SessionCreated journal event this writer emits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCreated {
    pub session: SessionId,
    pub generation: Generation,
}

/// Runtime Session writer over a real `SessionAppendStore`.
///
/// The store is the same durable surface the legacy writer used, so there is
/// one writer — the Runtime mints the ID and appends the event; nothing else
/// mints a canonical SessionId in this path.
pub struct RuntimeSessionWriter {
    authority: SessionAuthority,
    store: Arc<dyn ::contracts::SessionAppendStore>,
    clock: Arc<dyn ::contracts::Clock>,
}

impl RuntimeSessionWriter {
    pub fn new(
        authority: SessionAuthority,
        store: Arc<dyn ::contracts::SessionAppendStore>,
        clock: Arc<dyn ::contracts::Clock>,
    ) -> Self {
        Self {
            authority,
            store,
            clock,
        }
    }

    pub fn begin_maintenance(&self) -> crate::session_authority::SessionMaintenanceReceipt {
        self.authority.begin_maintenance()
    }

    pub async fn drain(
        &self,
        timeout: std::time::Duration,
    ) -> Result<crate::session_authority::SessionMaintenanceReceipt, RuntimeError> {
        self.authority.drain(timeout).await
    }

    pub fn resume_next_generation(&self) -> Result<Generation, RuntimeError> {
        self.authority.resume_next_generation()
    }

    pub fn current_generation(&self) -> Generation {
        self.authority.current_generation()
    }

    /// Mint a canonical SessionId + append `SessionCreated` + create the
    /// session record in the store.  Returns the Runtime-assigned receipt.
    pub async fn create_session(
        &self,
        correlation: Option<String>,
        principal_hint: Option<String>,
    ) -> Result<CommandReceipt, RuntimeError> {
        self.create_session_at_generation(None, correlation, principal_hint)
            .await
    }

    /// Generation-fenced create entry for callers holding a maintenance
    /// receipt. Old callers cannot write after the next generation resumes.
    pub async fn create_session_with_generation(
        &self,
        expected_generation: &Generation,
        correlation: Option<String>,
        principal_hint: Option<String>,
    ) -> Result<CommandReceipt, RuntimeError> {
        self.create_session_at_generation(Some(expected_generation), correlation, principal_hint)
            .await
    }

    async fn create_session_at_generation(
        &self,
        expected_generation: Option<&Generation>,
        correlation: Option<String>,
        principal_hint: Option<String>,
    ) -> Result<CommandReceipt, RuntimeError> {
        let _permit = self.authority.admit(expected_generation)?;
        // Neither value currently has a durable authenticated representation in
        // SessionRecord. Reject before mint/append rather than silently dropping
        // caller metadata or binding an unauthenticated principal hint.
        if correlation.is_some() || principal_hint.is_some() {
            return Err(RuntimeError::UnsupportedRequest);
        }

        // 1. Runtime assigns the canonical SessionId (no caller ID).
        let (session, generation) = self.authority.mint_session();

        // 2. Append SessionCreated through the real append store. The
        // EventSourcedSessionStore commits the event spine before materializing
        // the canonical read model; no process-local head participates in the
        // authority decision.
        //    (single writer).  Unknown shapes fail closed.  The store speaks
        //    ::contracts::SessionId (the shared durable type); the Runtime id is the
        //    canonical string value carried across.
        let fabric_session = ::contracts::SessionId(session.0.clone());
        let record = ::contracts::SessionRecord {
            schema_version: ::contracts::SESSION_SCHEMA_VERSION,
            id: fabric_session.clone(),
            parent: None,
            created_at_ms: self.clock.wall_now().0.max(0) as u64,
            status: ::contracts::SessionStatus::Active,
        };
        self.store
            .create(record)
            .await
            .map_err(|_| RuntimeError::Internal)?;

        Ok(CommandReceipt {
            session: Some(session),
            turn: None,
            agent_run: None,
            generation: Some(generation),
        })
    }

    /// Fork a session through the authoritative append store. The Runtime
    /// mints the child identity and binds the parent/sequence atomically in the
    /// same store used by SessionCreated.
    pub async fn fork_session(
        &self,
        parent: &SessionId,
        through_sequence: u64,
    ) -> Result<CommandReceipt, RuntimeError> {
        self.fork_session_at_generation(None, parent, through_sequence)
            .await
    }

    pub async fn fork_session_with_generation(
        &self,
        expected_generation: &Generation,
        parent: &SessionId,
        through_sequence: u64,
    ) -> Result<CommandReceipt, RuntimeError> {
        self.fork_session_at_generation(Some(expected_generation), parent, through_sequence)
            .await
    }

    async fn fork_session_at_generation(
        &self,
        expected_generation: Option<&Generation>,
        parent: &SessionId,
        through_sequence: u64,
    ) -> Result<CommandReceipt, RuntimeError> {
        let _permit = self.authority.admit(expected_generation)?;
        let (session, generation) = self.authority.mint_session();
        let child = ::contracts::SessionRecord {
            schema_version: ::contracts::SESSION_SCHEMA_VERSION,
            id: ::contracts::SessionId(session.0.clone()),
            parent: Some(::contracts::SessionFork {
                session_id: ::contracts::SessionId(parent.0.clone()),
                through_sequence,
            }),
            created_at_ms: self.clock.wall_now().0.max(0) as u64,
            status: ::contracts::SessionStatus::Active,
        };
        self.store
            .fork(
                &::contracts::SessionId(parent.0.clone()),
                through_sequence,
                child,
            )
            .await
            .map_err(|_| RuntimeError::Internal)?;
        Ok(CommandReceipt {
            session: Some(session),
            turn: None,
            agent_run: None,
            generation: Some(generation),
        })
    }

    /// The SessionCreated event this writer would emit (typed shape).
    pub fn created_event(&self, session: &SessionId) -> RuntimeEvent {
        RuntimeEvent::SessionCreated {
            session: session.clone(),
        }
    }
}

#[async_trait]
impl RuntimeCommandPort for RuntimeSessionWriter {
    async fn dispatch(&self, command: RuntimeCommand) -> Result<CommandReceipt, RuntimeError> {
        match command {
            RuntimeCommand::CreateSession(CreateSessionCommand {
                correlation,
                principal_hint,
            }) => self.create_session(correlation, principal_hint).await,
            RuntimeCommand::ForkSession(ForkSessionCommand {
                session,
                through_sequence,
            }) => self.fork_session(&session, through_sequence).await,
            // RA-03 writer only owns session creation; turns are RA-04.
            _ => Err(RuntimeError::UnknownSchema),
        }
    }
}

/// In-memory append store for writer tests.
#[derive(Clone, Default)]
pub struct InMemoryAppendStore {
    created: std::sync::Arc<std::sync::Mutex<Vec<::contracts::SessionRecord>>>,
}

impl InMemoryAppendStore {
    #[allow(dead_code)]
    fn list(&self) -> Vec<::contracts::SessionRecord> {
        self.created.lock().unwrap().clone()
    }
}

#[async_trait]
impl ::contracts::SessionAppendStore for InMemoryAppendStore {
    async fn create(&self, session: ::contracts::SessionRecord) -> anyhow::Result<()> {
        self.created.lock().unwrap().push(session);
        Ok(())
    }
    async fn append(
        &self,
        _session: &::contracts::SessionId,
        _expected_sequence: u64,
        _item: ::contracts::ItemRecord,
    ) -> anyhow::Result<::contracts::AppendOutcome> {
        Ok(::contracts::AppendOutcome::Appended)
    }
    async fn fork(
        &self,
        _parent: &::contracts::SessionId,
        _through_sequence: u64,
        _child: ::contracts::SessionRecord,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn bind_principal(
        &self,
        _session: &::contracts::SessionId,
        _principal: &::contracts::PrincipalId,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl ::contracts::SessionReadStore for InMemoryAppendStore {
    async fn load_session(
        &self,
        session: &::contracts::SessionId,
    ) -> anyhow::Result<Option<::contracts::SessionRecord>> {
        Ok(self
            .created
            .lock()
            .unwrap()
            .iter()
            .find(|r| &r.id == session)
            .cloned())
    }
    async fn load_items(
        &self,
        _session: &::contracts::SessionId,
        _after: Option<u64>,
    ) -> anyhow::Result<Vec<::contracts::ItemRecord>> {
        Ok(vec![])
    }
    async fn principal_for(
        &self,
        _session: &::contracts::SessionId,
    ) -> anyhow::Result<Option<::contracts::PrincipalId>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_spine::EventSpine;
    use crate::journal::RuntimeJournalShadow;

    struct EmptySpine;
    impl EventSpine for EmptySpine {
        fn append(
            &self,
            _e: crate::event_spine::UnsequencedEvent,
        ) -> anyhow::Result<crate::event_spine::SpineEvent> {
            anyhow::bail!("shadow must not append")
        }
        fn read_committed_page(
            &self,
            _a: u64,
            _t: u64,
            _l: usize,
        ) -> anyhow::Result<Vec<(u64, crate::event_spine::SpineEvent)>> {
            Ok(vec![])
        }
    }

    struct FixedClock;
    impl ::contracts::Clock for FixedClock {
        fn wall_now(&self) -> ::contracts::WallTime {
            ::contracts::WallTime(42)
        }

        fn mono_now(&self) -> ::contracts::MonoTime {
            ::contracts::MonoTime(7)
        }
    }

    #[tokio::test]
    async fn writer_mints_session_appends_and_returns_receipt() {
        let shadow = Arc::new(RuntimeJournalShadow::new(Arc::new(EmptySpine)));
        let mem_store = InMemoryAppendStore::default();
        let store: Arc<dyn ::contracts::SessionAppendStore> = Arc::new(mem_store.clone());
        let writer = RuntimeSessionWriter::new(
            SessionAuthority::new(shadow),
            store.clone(),
            Arc::new(FixedClock),
        );
        let receipt = writer
            .dispatch(RuntimeCommand::CreateSession(CreateSessionCommand {
                correlation: None,
                principal_hint: None,
            }))
            .await
            .unwrap();
        let session = receipt.session.unwrap();
        assert!(session.0.starts_with("session-"));
        // The store has one durable session record (single writer).
        let stored = mem_store
            .list()
            .into_iter()
            .find(|r| r.id.0 == session.0)
            .expect("session persisted");
        assert_eq!(stored.id.0, session.0);
        assert_eq!(stored.created_at_ms, 42);
        // The created event is typed.
        match writer.created_event(&session) {
            RuntimeEvent::SessionCreated { session: s } => assert_eq!(s, session),
            _ => panic!("expected SessionCreated"),
        }
    }

    #[tokio::test]
    async fn unsupported_correlation_or_principal_is_rejected_before_append() {
        let shadow = Arc::new(RuntimeJournalShadow::new(Arc::new(EmptySpine)));
        let mem_store = InMemoryAppendStore::default();
        let writer = RuntimeSessionWriter::new(
            SessionAuthority::new(shadow),
            Arc::new(mem_store.clone()),
            Arc::new(FixedClock),
        );

        assert_eq!(
            writer.create_session(Some("caller-ref".into()), None).await,
            Err(RuntimeError::UnsupportedRequest)
        );
        assert_eq!(
            writer.create_session(None, Some("alice".into())).await,
            Err(RuntimeError::UnsupportedRequest)
        );
        assert!(mem_store.list().is_empty());
    }

    #[tokio::test]
    async fn maintenance_drains_and_fences_old_generation() {
        let shadow = Arc::new(RuntimeJournalShadow::new(Arc::new(EmptySpine)));
        let writer = RuntimeSessionWriter::new(
            SessionAuthority::new(shadow),
            Arc::new(InMemoryAppendStore::default()),
            Arc::new(FixedClock),
        );
        let old = writer.current_generation();
        let receipt = writer.begin_maintenance();
        assert_eq!(receipt.generation, old);
        assert_eq!(
            writer
                .drain(std::time::Duration::from_millis(5))
                .await
                .unwrap()
                .active_writes,
            0
        );
        assert_eq!(
            writer.create_session(None, None).await,
            Err(RuntimeError::Retired)
        );
        let next = writer.resume_next_generation().unwrap();
        assert_eq!(next.0, old.0 + 1);
        assert_eq!(
            writer
                .create_session_with_generation(&old, None, None)
                .await,
            Err(RuntimeError::WrongGeneration)
        );
        assert!(writer
            .create_session_with_generation(&next, None, None)
            .await
            .is_ok());
    }
}

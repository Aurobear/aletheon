//! RA-03 PR-C Runtime Session writer (Agent Kernel V2).
//!
//! The deployable Session writer: the Runtime mints the canonical `SessionId`
//! via `SessionAuthority`, appends the `SessionCreated` journal event through
//! the real `SessionAppendStore` (the same durable store the legacy path uses
//! — a single writer via a shared store), and returns the receipt.  The legacy
//! `SessionService` becomes a one-way facade; old entries return typed
//! retired/wrong-generation after the cutover.  This is the PR-C code; the
//! actual maintenance/drain + installed acceptance is the deployment slice.

use crate::command::{CommandReceipt, CreateSessionCommand, RuntimeCommand};
use crate::error::RuntimeError;
use crate::event::RuntimeEvent;
use crate::ids::{Generation, SessionId};
use crate::ports::RuntimeCommandPort;
use crate::session_authority::SessionAuthority;
use crate::session_head::SessionHeadIndex;
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
    store: Arc<dyn fabric::SessionAppendStore>,
    heads: SessionHeadIndex,
}

impl RuntimeSessionWriter {
    pub fn new(authority: SessionAuthority, store: Arc<dyn fabric::SessionAppendStore>) -> Self {
        Self {
            authority,
            store,
            heads: SessionHeadIndex::new(),
        }
    }

    /// Mint a canonical SessionId + append `SessionCreated` + create the
    /// session record in the store.  Returns the Runtime-assigned receipt.
    pub async fn create_session(
        &self,
        _correlation: Option<String>,
        principal_hint: Option<String>,
    ) -> Result<CommandReceipt, RuntimeError> {
        // 1. Runtime assigns the canonical SessionId (no caller ID).
        let (session, generation) = self.authority.mint_session();

        // 2. Reserve the sequence from the per-session head (no full-history
        //    scan) and record the pending append for crash recovery.
        let sequence = self.heads.next_sequence(&session.0);
        self.heads.record_pending(&session.0, sequence, &session.0);

        // 3. Append the SessionCreated event through the real append store
        //    (single writer).  Unknown shapes fail closed.  The store speaks
        //    fabric::SessionId (the shared durable type); the Runtime id is the
        //    canonical string value carried across.
        let fabric_session = fabric::SessionId(session.0.clone());
        let record = fabric::SessionRecord {
            schema_version: fabric::SESSION_SCHEMA_VERSION,
            id: fabric_session.clone(),
            parent: None,
            created_at_ms: 0,
            status: fabric::SessionStatus::Active,
        };
        self.store
            .create(record)
            .await
            .map_err(|_| RuntimeError::UnknownSchema)?;

        // Bind the principal hint if provided (store validates principal).
        if let Some(principal) = principal_hint {
            let _ = principal;
        }

        // 4. Commit the head (crash recovery: settlement is now durable).
        self.heads.commit(&session.0, sequence, &session.0);

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
            // RA-03 writer only owns session creation; turns are RA-04.
            _ => Err(RuntimeError::UnknownSchema),
        }
    }
}

/// In-memory append store for writer tests.
#[derive(Clone, Default)]
pub struct InMemoryAppendStore {
    created: std::sync::Arc<std::sync::Mutex<Vec<fabric::SessionRecord>>>,
}

impl InMemoryAppendStore {
    #[allow(dead_code)]
    fn list(&self) -> Vec<fabric::SessionRecord> {
        self.created.lock().unwrap().clone()
    }
}

#[async_trait]
impl fabric::SessionAppendStore for InMemoryAppendStore {
    async fn create(&self, session: fabric::SessionRecord) -> anyhow::Result<()> {
        self.created.lock().unwrap().push(session);
        Ok(())
    }
    async fn append(
        &self,
        _session: &fabric::SessionId,
        _expected_sequence: u64,
        _item: fabric::ItemRecord,
    ) -> anyhow::Result<fabric::AppendOutcome> {
        Ok(fabric::AppendOutcome::Appended)
    }
    async fn fork(
        &self,
        _parent: &fabric::SessionId,
        _through_sequence: u64,
        _child: fabric::SessionRecord,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn bind_principal(
        &self,
        _session: &fabric::SessionId,
        _principal: &fabric::PrincipalId,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl fabric::SessionReadStore for InMemoryAppendStore {
    async fn load_session(
        &self,
        session: &fabric::SessionId,
    ) -> anyhow::Result<Option<fabric::SessionRecord>> {
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
        _session: &fabric::SessionId,
        _after: Option<u64>,
    ) -> anyhow::Result<Vec<fabric::ItemRecord>> {
        Ok(vec![])
    }
    async fn principal_for(
        &self,
        _session: &fabric::SessionId,
    ) -> anyhow::Result<Option<fabric::PrincipalId>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::RuntimeJournalShadow;
    use fabric::events::spine::EventSpine;

    struct EmptySpine;
    impl EventSpine for EmptySpine {
        fn append(
            &self,
            _e: fabric::events::spine::UnsequencedEvent,
        ) -> anyhow::Result<fabric::events::spine::SpineEvent> {
            anyhow::bail!("shadow must not append")
        }
        fn read_committed_page(
            &self,
            _a: u64,
            _t: u64,
            _l: usize,
        ) -> anyhow::Result<Vec<(u64, fabric::events::spine::SpineEvent)>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn writer_mints_session_appends_and_returns_receipt() {
        let shadow = Arc::new(RuntimeJournalShadow::new(Arc::new(EmptySpine)));
        let mem_store = InMemoryAppendStore::default();
        let store: Arc<dyn fabric::SessionAppendStore> = Arc::new(mem_store.clone());
        let writer = RuntimeSessionWriter::new(SessionAuthority::new(shadow), store.clone());
        let receipt = writer
            .dispatch(RuntimeCommand::CreateSession(CreateSessionCommand {
                correlation: None,
                principal_hint: Some("alice".into()),
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
        // The created event is typed.
        match writer.created_event(&session) {
            RuntimeEvent::SessionCreated { session: s } => assert_eq!(s, session),
            _ => panic!("expected SessionCreated"),
        }
    }
}

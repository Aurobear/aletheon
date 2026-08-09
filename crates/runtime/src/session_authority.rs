//! RA-03 Runtime SessionAuthority owner seam (Agent Kernel V2).
//!
//! The SessionAuthority is the **new** Session writer contract: it mints the
//! canonical `SessionId`, appends `SessionCreated`, and rebuilds a
//! `ContextWorkingSet` from the journal/projection.  Per runbook PR-A this
//! seam is defined additively and is **not wired** — the legacy
//! `SessionService`/`SessionStore` remain the single authoritative writer
//! until the RA-03 PR-C writer cutover (maintenance/drain, freeze, switch,
//! installed acceptance).  `SessionCreated` is the authority's first journal
//! event; forks and principal binding append under the same epoch.

use crate::error::RuntimeError;
use crate::event::{RuntimeEvent, TurnTerminal};
use crate::ids::{Generation, SessionId, TurnId};
use crate::journal::RuntimeJournalShadow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Epoch counter for session generation.  Only the Runtime composition may
/// construct the SessionAuthority; a client never mints a SessionId.
struct EpochSource {
    next: AtomicU64,
}

impl EpochSource {
    fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
        }
    }

    fn next_generation(&self) -> Generation {
        Generation(self.next.fetch_add(1, Ordering::Relaxed))
    }
}

/// Runtime SessionAuthority.  Mint + append are authority actions; this seam
/// defines the contract and a test-only in-memory implementation, not the
/// production writer (which arrives at RA-03 PR-C).
pub struct SessionAuthority {
    generation: EpochSource,
    shadow: Arc<RuntimeJournalShadow>,
}

impl SessionAuthority {
    pub fn new(shadow: Arc<RuntimeJournalShadow>) -> Self {
        Self {
            generation: EpochSource::new(),
            shadow,
        }
    }

    /// Mint a canonical SessionId + generation.  No caller-supplied ID.
    pub fn mint_session(&self) -> (SessionId, Generation) {
        (
            SessionId(format!("session-{}", self.generation.next_generation().0)),
            Generation(1),
        )
    }

    /// Append the `SessionCreated` journal event (authority action).  In this
    /// seam the shadow is read-only; the real append is wired at PR-C.  This
    /// method documents the event shape the authority will emit.
    pub fn created_event(&self, session: &SessionId, generation: Generation) -> RuntimeEvent {
        // The authority does not mint a Turn here; TurnStarted belongs to RA-04.
        let _ = (self.shadow.as_ref(), generation);
        RuntimeEvent::SessionCreated {
            session: session.clone(),
        }
    }

    /// Placeholder for the ContextWorkingSet rebuild: given a session + the
    /// journal projection, return the working set summary.  Real rebuild is
    /// RA-03 PR-C.
    pub fn rebuild_working_set(
        &self,
        session: &SessionId,
    ) -> Result<ContextWorkingSet, RuntimeError> {
        Ok(ContextWorkingSet {
            session: session.clone(),
            turn: None,
        })
    }
}

/// Rebuildable per-session working set (replaces the legacy SessionManager
/// authority).  Derived from journal/projection, never from a second writer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextWorkingSet {
    pub session: SessionId,
    pub turn: Option<TurnId>,
}

/// A later turn-start/settle event tied to the working set (RA-04 shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnProjection {
    pub turn: TurnId,
    pub terminal: TurnTerminal,
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn authority_mints_canonical_session_no_caller_id() {
        let shadow = Arc::new(RuntimeJournalShadow::new(Arc::new(EmptySpine)));
        let authority = SessionAuthority::new(shadow);
        let (session, generation) = authority.mint_session();
        assert!(session.0.starts_with("session-"));
        assert_eq!(generation.0, 1);
        // The created event is a typed RuntimeEvent with the Runtime-assigned id.
        match authority.created_event(&session, generation) {
            RuntimeEvent::SessionCreated { session: s } => assert_eq!(s, session),
            _ => panic!("expected SessionCreated"),
        }
    }

    #[test]
    fn working_set_rebuilds_from_session() {
        let shadow = Arc::new(RuntimeJournalShadow::new(Arc::new(EmptySpine)));
        let authority = SessionAuthority::new(shadow);
        let (session, _) = authority.mint_session();
        let ws = authority.rebuild_working_set(&session).unwrap();
        assert_eq!(ws.session, session);
        assert!(ws.turn.is_none());
    }
}

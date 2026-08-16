//! RA-03 Runtime SessionAuthority owner seam (Agent Kernel V2).
//!
//! The SessionAuthority owns canonical `SessionId` allocation and the typed
//! ContextWorkingSet contract. `RuntimeSessionWriter` wires the minted ID to
//! the production Session append store.

use crate::error::RuntimeError;
use crate::event::{RuntimeEvent, TurnTerminal};
use crate::ids::{Generation, SessionId, TurnId};
use crate::journal::RuntimeJournalShadow;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Runtime SessionAuthority. Minting is authoritative; durable append is
/// performed by `RuntimeSessionWriter` so allocation and persistence cannot be
/// invoked independently at the production command boundary.
pub struct SessionAuthority {
    shadow: Arc<RuntimeJournalShadow>,
    generation: AtomicU64,
    frozen: AtomicBool,
    active_writes: AtomicU64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMaintenanceReceipt {
    pub generation: Generation,
    pub active_writes: u64,
}

pub struct SessionWritePermit<'a> {
    authority: &'a SessionAuthority,
}

impl Drop for SessionWritePermit<'_> {
    fn drop(&mut self) {
        self.authority.active_writes.fetch_sub(1, Ordering::AcqRel);
    }
}

impl SessionAuthority {
    pub fn new(shadow: Arc<RuntimeJournalShadow>) -> Self {
        Self {
            shadow,
            generation: AtomicU64::new(1),
            frozen: AtomicBool::new(false),
            active_writes: AtomicU64::new(0),
        }
    }

    pub fn begin_maintenance(&self) -> SessionMaintenanceReceipt {
        self.frozen.store(true, Ordering::Release);
        SessionMaintenanceReceipt {
            generation: self.current_generation(),
            active_writes: self.active_writes.load(Ordering::Acquire),
        }
    }

    pub async fn drain(
        &self,
        timeout: Duration,
    ) -> Result<SessionMaintenanceReceipt, RuntimeError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let active = self.active_writes.load(Ordering::Acquire);
            if active == 0 {
                return Ok(SessionMaintenanceReceipt {
                    generation: self.current_generation(),
                    active_writes: 0,
                });
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(RuntimeError::Timeout);
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    /// Re-open admission only after maintenance has drained every write.
    ///
    /// Keeping this check in the authority (rather than relying on callers to
    /// sequence `drain` and resume correctly) prevents an old in-flight write
    /// from crossing the generation boundary.
    pub fn resume_next_generation(&self) -> Result<Generation, RuntimeError> {
        if !self.frozen.load(Ordering::Acquire) {
            return Err(RuntimeError::UnsupportedRequest);
        }
        if self.active_writes.load(Ordering::Acquire) != 0 {
            return Err(RuntimeError::MaintenanceNotDrained);
        }
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.frozen.store(false, Ordering::Release);
        Ok(Generation(generation))
    }

    pub fn current_generation(&self) -> Generation {
        Generation(self.generation.load(Ordering::Acquire))
    }

    pub fn admit(
        &self,
        expected: Option<&Generation>,
    ) -> Result<SessionWritePermit<'_>, RuntimeError> {
        if self.frozen.load(Ordering::Acquire) {
            return Err(RuntimeError::Retired);
        }
        let generation = self.generation.load(Ordering::Acquire);
        if expected.is_some_and(|expected| expected.0 != generation) {
            return Err(RuntimeError::WrongGeneration);
        }
        self.active_writes.fetch_add(1, Ordering::AcqRel);
        if self.frozen.load(Ordering::Acquire) {
            self.active_writes.fetch_sub(1, Ordering::AcqRel);
            return Err(RuntimeError::Retired);
        }
        Ok(SessionWritePermit { authority: self })
    }

    /// Mint a canonical SessionId + generation.  No caller-supplied ID.
    pub fn mint_session(&self) -> (SessionId, Generation) {
        (
            // Runtime owns the mint. A UUID keeps identities unique across
            // daemon restarts; the prior process-local counter recreated
            // `session-1` after every restart.
            SessionId(format!("session-{}", uuid::Uuid::new_v4())),
            self.current_generation(),
        )
    }

    /// Describe the `SessionCreated` journal event (authority action). The
    /// writer performs the durable append after minting the ID.
    pub fn created_event(&self, session: &SessionId, generation: Generation) -> RuntimeEvent {
        // The authority does not mint a Turn here; TurnStarted belongs to RA-04.
        let _ = (self.shadow.as_ref(), generation);
        RuntimeEvent::SessionCreated {
            session: session.clone(),
        }
    }
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
    use crate::event_spine::EventSpine;

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
    fn resume_is_rejected_until_active_writes_are_drained() {
        let shadow = Arc::new(RuntimeJournalShadow::new(Arc::new(EmptySpine)));
        let authority = SessionAuthority::new(shadow);
        let permit = authority.admit(None).expect("write admission");
        let receipt = authority.begin_maintenance();
        assert_eq!(receipt.active_writes, 1);
        assert_eq!(
            authority.resume_next_generation(),
            Err(RuntimeError::MaintenanceNotDrained)
        );

        drop(permit);
        let next = authority
            .resume_next_generation()
            .expect("drained maintenance resumes");
        assert_eq!(next.0, receipt.generation.0 + 1);
    }
}

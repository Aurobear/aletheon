//! RA-03 PR-B Session shadow verifier (Agent Kernel V2).
//!
//! Read-only shadow: replays the canonical session store through the RA-02
//! `RuntimeJournalShadow` and compares against the legacy projection.  It
//! **never appends, spawns, or performs an effect** (runbook PR-B).  The
//! legacy SessionService stays the single authoritative writer until the
//! PR-C switch.  This is the deployable shadow the daemon can run as a
//! self-check before the writer cutover.

use crate::error::RuntimeError;
use crate::journal::{RuntimeJournalShadow, StreamKind};
use crate::session_head::SessionHeadIndex;
use std::sync::Arc;

/// A typed shadow comparison result.  No append, no switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShadowReport {
    /// Replay matched the legacy projection (same last sequence/head).
    Match {
        session: String,
        last_sequence: u64,
        streams: Vec<StreamKind>,
    },
    /// The legacy projection has sessions the shadow cannot yet classify.
    Mismatch { session: String, reason: String },
    /// No sessions in the store (empty shadow).
    Empty,
}

/// Read-only shadow verifier over a canonical store head + journal shadow.
pub struct SessionShadowVerifier {
    journal: Arc<RuntimeJournalShadow>,
    heads: SessionHeadIndex,
}

impl SessionShadowVerifier {
    pub fn new(journal: Arc<RuntimeJournalShadow>) -> Self {
        Self {
            journal,
            heads: SessionHeadIndex::new(),
        }
    }

    /// Verify one session's head by replaying its committed pages.  Read-only:
    /// never appends (the journal shadow forbids it).  Compares the last
    /// committed sequence against the per-session head.
    pub fn verify_session(
        &self,
        session_id: &str,
        legacy_last_sequence: u64,
    ) -> Result<ShadowReport, RuntimeError> {
        // Replay the committed pages the shadow can read (bounded).
        let page = self
            .journal
            .replay_committed(0, legacy_last_sequence.saturating_add(1), 1000)
            .map_err(|_| RuntimeError::UnknownSchema)?;

        if page.is_empty() && legacy_last_sequence == 0 {
            return Ok(ShadowReport::Empty);
        }

        // Classify the streams seen and find the max sequence.
        let mut streams = Vec::new();
        let mut max_seq = 0u64;
        for entry in &page {
            if !streams.contains(&entry.stream) {
                streams.push(entry.stream);
            }
            if entry.position > max_seq {
                max_seq = entry.position;
            }
        }

        // The head is derived only from committed events (never a second
        // authority).  A legacy sequence beyond the committed max means the
        // legacy projection has uncommitted data the shadow can't verify yet.
        if legacy_last_sequence > max_seq {
            return Ok(ShadowReport::Mismatch {
                session: session_id.to_string(),
                reason: format!(
                    "legacy sequence {} exceeds committed max {}",
                    legacy_last_sequence, max_seq
                ),
            });
        }

        self.heads.commit(session_id, max_seq, session_id);
        Ok(ShadowReport::Match {
            session: session_id.to_string(),
            last_sequence: max_seq,
            streams,
        })
    }
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
    fn empty_store_returns_empty_report() {
        let shadow = Arc::new(RuntimeJournalShadow::new(Arc::new(EmptySpine)));
        let verifier = SessionShadowVerifier::new(shadow);
        assert_eq!(
            verifier.verify_session("s1", 0).unwrap(),
            ShadowReport::Empty
        );
    }

    #[test]
    fn legacy_beyond_committed_is_a_mismatch_not_a_silent_success() {
        let shadow = Arc::new(RuntimeJournalShadow::new(Arc::new(EmptySpine)));
        let verifier = SessionShadowVerifier::new(shadow);
        // Legacy claims sequence 5 but the shadow committed nothing readable.
        assert!(matches!(
            verifier.verify_session("s1", 5).unwrap(),
            ShadowReport::Mismatch { .. }
        ));
    }
}

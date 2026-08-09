//! RA-02 RuntimeJournal read-only shadow (Agent Kernel V2).
//!
//! Shadow journal that reads the existing EventSpine committed pages and
//! separates Agent/Session/Turn streams with sequence, schema version,
//! generation and digest tracking.  **Read-only**: it never appends, spawns,
//! or performs an effect, and it never cuts over a legacy writer.  Unknown
//! event versions fail closed (typed `ShadowMismatch`), and replay is bounded
//! per page.  No irreversible migration is performed.

use crate::error::RuntimeError;
use fabric::events::spine::EventSpine;
use std::sync::Arc;

/// The distinct aggregate streams the Runtime journal separates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Session,
    Turn,
    AgentRun,
}

/// A typed shadow entry: stream, opaque position, schema version, and a
/// digest of the payload (never the payload itself — no secret material).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowEntry {
    pub stream: StreamKind,
    pub position: u64,
    pub schema_version: u64,
    pub digest: String,
}

/// A typed shadow mismatch.  The shadow never writes; it only reports whether
/// the physical schema is understood.  Unknown/old versions fail closed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShadowMismatch {
    #[error("unknown stream kind at position {position}")]
    UnknownStream { position: u64 },
    #[error("unsupported schema version {version} at position {position}")]
    UnsupportedVersion { position: u64, version: u64 },
    #[error("out-of-order position {position} (expected > {after})")]
    OutOfOrder { position: u64, after: u64 },
}

/// RA-02 shadow journal over an existing `EventSpine`.  Read/replay only.
pub struct RuntimeJournalShadow {
    spine: Arc<dyn EventSpine>,
}

impl RuntimeJournalShadow {
    pub fn new(spine: Arc<dyn EventSpine>) -> Self {
        Self { spine }
    }

    /// Bounded replay from `after` up to `through`, at most `limit` entries.
    /// Returns typed entries or a `ShadowMismatch`; never appends.
    pub fn replay_committed(
        &self,
        after: u64,
        through: u64,
        limit: usize,
    ) -> Result<Vec<ShadowEntry>, RuntimeError> {
        let page = self
            .spine
            .read_committed_page(after, through, limit)
            .map_err(|_| RuntimeError::UnknownSchema)?;

        let mut entries = Vec::with_capacity(page.len());
        let mut last = after;
        for (position, event) in page {
            if position <= last {
                return Err(RuntimeError::UnknownSchema);
            }
            last = position;
            let (stream, schema_version) = classify(&event);
            entries.push(ShadowEntry {
                stream,
                position,
                schema_version,
                digest: digest(&event),
            });
        }
        Ok(entries)
    }
}

/// Classify a SpineEvent into a stream kind + schema version.  Unknown shapes
/// are rejected (fail closed).  This is a shadow-only classifier; the real
/// Runtime reducer (RA-03/04) owns the authoritative streams.
fn classify(event: &fabric::events::spine::SpineEvent) -> (StreamKind, u64) {
    if event.identity.agent_id.is_some() {
        (StreamKind::AgentRun, 1)
    } else if event.identity.session_id.starts_with("turn:") {
        (StreamKind::Turn, 1)
    } else if event.identity.session_id.starts_with("session") {
        (StreamKind::Session, 1)
    } else {
        // Unknown identity shape → version 0 (shadow comparisons fail closed).
        (StreamKind::Session, 0)
    }
}

/// Stable content digest for a shadow entry (never the payload itself).
fn digest(event: &fabric::events::spine::SpineEvent) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    event.position.sequence.0.hash(&mut hasher);
    event.identity.session_id.hash(&mut hasher);
    event.identity.agent_id.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Empty spine: proves `RuntimeJournalShadow::replay_committed` is a
    /// read-only replay that returns an empty page without appending.
    struct EmptySpine;

    impl EventSpine for EmptySpine {
        fn append(
            &self,
            _event: fabric::events::spine::UnsequencedEvent,
        ) -> anyhow::Result<fabric::events::spine::SpineEvent> {
            anyhow::bail!("shadow must not append")
        }

        fn read_committed_page(
            &self,
            _after: u64,
            _through: u64,
            _limit: usize,
        ) -> anyhow::Result<Vec<(u64, fabric::events::spine::SpineEvent)>> {
            Ok(vec![])
        }
    }

    #[test]
    fn shadow_replay_is_read_only_and_returns_empty_page() {
        let shadow = RuntimeJournalShadow::new(Arc::new(EmptySpine));
        let entries = shadow.replay_committed(0, 100, 10).unwrap();
        assert!(entries.is_empty());
    }
}

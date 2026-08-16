//! RuntimeJournalResources (RA-03 design) — shared journal spine + projections.
//!
//! Constructed ONCE before the first session_id consumer and injected into
//! every component that needs the durable event journal (agent services, turn
//! services, session writer).  It does **not** own the canonical session
//! store — that belongs to `SessionInfrastructure`.  This prevents a second
//! composition root: every consumer reuses the same `Arc` instance instead of
//! re-opening the DB files.
//!
//! The spine is opened with the same bounded policy (`max_event_spine_bytes`)
//! the daemon previously used at `build_agent_services`.

use anyhow::Context;
use std::sync::Arc;

/// Shared, durable event journal resources (spine + projections).
#[derive(Clone)]
pub struct RuntimeJournalResources {
    spine: Arc<adapters_sqlite::event_spine::SqliteEventSpine>,
    projections: Arc<adapters_sqlite::projection_set::DefaultEventProjectionSet>,
}

impl RuntimeJournalResources {
    /// Open the shared spine + projections rooted at `data_dir` with the given
    /// bounded spine capacity.  Call once before the first session consumer.
    pub fn open(
        data_dir: &std::path::Path,
        max_event_spine_bytes: Option<u64>,
    ) -> anyhow::Result<Self> {
        let spine = Arc::new(
            adapters_sqlite::event_spine::SqliteEventSpine::open_bounded(
                data_dir.join("events.db"),
                max_event_spine_bytes,
            )
            .context("open durable canonical event spine")?,
        );
        let projections = Arc::new(
            adapters_sqlite::projection_set::DefaultEventProjectionSet::open(
                data_dir.join("event-projections.db"),
            )
            .context("open durable event projections")?,
        );
        Ok(Self { spine, projections })
    }

    /// The single shared event spine.
    pub fn spine(&self) -> Arc<adapters_sqlite::event_spine::SqliteEventSpine> {
        self.spine.clone()
    }

    /// The single shared event projection set.
    pub fn projections(&self) -> Arc<adapters_sqlite::projection_set::DefaultEventProjectionSet> {
        self.projections.clone()
    }
}

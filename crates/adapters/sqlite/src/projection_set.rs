use mnemosyne::memory_job_projection::MemoryJobProjection;
use runtime::event_projection::agent_tree::AgentTreeProjection;
use runtime::event_projection::debug::DebugProjection;
use runtime::event_projection::metrics::MetricsProjection;
use std::{path::Path, sync::Arc};

use runtime::SpineEvent;

use crate::event_projection::SqliteProjectionStore;
use runtime::read_model::{
    EventProjection, EventProjectionSink, ProjectionAdvanceReport, ProjectionFailure, ProjectionLag,
};

use runtime::public_session_projection::SessionProjection;

pub struct DefaultEventProjectionSet {
    store: Arc<SqliteProjectionStore>,
}

impl DefaultEventProjectionSet {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, runtime::read_model::ProjectionError> {
        Ok(Self {
            store: Arc::new(SqliteProjectionStore::open(path)?),
        })
    }

    pub fn in_memory() -> Self {
        Self::open(":memory:").expect("in-memory event projections")
    }

    fn apply<P: EventProjection>(
        &self,
        projection: &P,
        event: &SpineEvent,
        report: &mut ProjectionAdvanceReport,
    ) {
        let descriptor = projection.descriptor();
        let input_sequence = event.position.sequence.0;
        match self.store.advance(projection, std::slice::from_ref(event)) {
            Ok((_, checkpoint)) => {
                report.lags.push(ProjectionLag {
                    projection: descriptor.name.into(),
                    input_sequence,
                    through_sequence: checkpoint.through_sequence,
                    pending_events: input_sequence.saturating_sub(checkpoint.through_sequence),
                });
                report.checkpoints.push(checkpoint);
            }
            Err(error) => {
                let through_sequence = self
                    .store
                    .checkpoint(descriptor.name, event.position.tree_id)
                    .ok()
                    .flatten()
                    .map_or(0, |checkpoint| checkpoint.through_sequence);
                report.lags.push(ProjectionLag {
                    projection: descriptor.name.into(),
                    input_sequence,
                    through_sequence,
                    pending_events: input_sequence.saturating_sub(through_sequence),
                });
                if let Ok(Some(poison)) = self
                    .store
                    .poison_for_tree(descriptor.name, event.position.tree_id)
                {
                    report.poisons.push(poison);
                }
                report.failures.push(ProjectionFailure {
                    projection: descriptor.name.into(),
                    error: error.to_string(),
                });
            }
        }
    }
}

impl EventProjectionSink for DefaultEventProjectionSet {
    fn project(&self, event: &SpineEvent) -> ProjectionAdvanceReport {
        let mut report = ProjectionAdvanceReport::default();
        self.apply(&SessionProjection, event, &mut report);
        self.apply(&DebugProjection, event, &mut report);
        self.apply(&MemoryJobProjection, event, &mut report);
        self.apply(&AgentTreeProjection, event, &mut report);
        self.apply(&MetricsProjection, event, &mut report);
        report
    }
}

pub fn default_event_projection_path() -> std::path::PathBuf {
    ::contracts::paths::xdg_data_dir().join("event-projections-v1.db")
}

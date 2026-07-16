use std::{path::Path, sync::Arc};

use fabric::SpineEvent;

use crate::service::event_projection::{
    EventProjection, EventProjectionSink, ProjectionAdvanceReport, ProjectionFailure,
    ProjectionLag, SqliteProjectionStore,
};

use super::{
    agent_tree_projection::AgentTreeProjection, debug_projection::DebugProjection,
    memory_job_projection::MemoryJobProjection, metrics_projection::MetricsProjection,
    session_projection::SessionProjection,
};

pub struct DefaultEventProjectionSet {
    store: Arc<SqliteProjectionStore>,
}

impl DefaultEventProjectionSet {
    pub fn open(
        path: impl AsRef<Path>,
    ) -> Result<Self, crate::service::event_projection::ProjectionError> {
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
    fabric::paths::xdg_data_dir().join("event-projections-v1.db")
}

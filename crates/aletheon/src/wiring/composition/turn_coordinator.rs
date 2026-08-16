//! Composition of the application turn coordinator with local persistence.

use std::path::Path;
use std::sync::Arc;

use ::contracts::SessionAppendStore;
use kernel::KernelRuntime;
use runtime::EventSpine;

use crate::config::GrokHardeningConfig;
use crate::wiring::application::evaluation::EvaluationService;
use crate::wiring::application::turn_coordinator::TurnCoordinator;
use adapters_sqlite::session::event_sourced_store::EventSourcedSessionStore;
use runtime::read_model::EventProjectionSink;
use runtime::session_projection::SessionProjectionStore;

pub fn compose_evaluation_service(
    kernel: Arc<KernelRuntime>,
    data_dir: &Path,
    settings: crate::config::EvaluationSettings,
    projection: Arc<application::evaluation_projection::EvaluationProjection>,
    capability_rollups: Arc<
        crate::wiring::application::capability_benchmark::CapabilityRollupProjectionSink,
    >,
) -> anyhow::Result<Arc<crate::wiring::application::evaluation::EvaluationService>> {
    let store: Arc<dyn metacog::evaluation::EvaluationReceiptStore> = Arc::new(
        adapters_sqlite::evaluation::SqliteEvaluationStore::open(data_dir.join("evaluations.db"))?,
    );
    Ok(Arc::new(
        EvaluationService::new(kernel, settings, store)?
            .with_projection(projection)
            .with_capability_rollups(capability_rollups),
    ))
}

pub fn compose_turn_coordinator(
    kernel: Arc<KernelRuntime>,
    read_store: Arc<dyn SessionProjectionStore>,
    event_spine: Arc<dyn EventSpine>,
    projections: Arc<dyn EventProjectionSink>,
    grok_hardening: GrokHardeningConfig,
) -> TurnCoordinator {
    let store = compose_session_store(read_store, event_spine, projections);
    TurnCoordinator::from_components(kernel, store, grok_hardening)
}

pub fn compose_session_store(
    read_store: Arc<dyn SessionProjectionStore>,
    event_spine: Arc<dyn EventSpine>,
    projections: Arc<dyn EventProjectionSink>,
) -> Arc<dyn SessionAppendStore> {
    Arc::new(EventSourcedSessionStore::new(
        read_store,
        event_spine,
        projections,
    ))
}

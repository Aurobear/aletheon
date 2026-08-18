//! Composition of the application turn coordinator with local persistence.

use std::path::Path;
use std::sync::Arc;

use ::contracts::SessionAppendStore;
use kernel::KernelRuntime;
use runtime::EventSpine;

use crate::config::GrokHardeningConfig;
use adapters_sqlite::session::event_sourced_store::EventSourcedSessionStore;
use application::evaluation::{
    DefaultCodingEvidenceCollector, DefaultTaskEvaluationContractIssuer, EvaluationReceiptStore,
    EvaluationService,
};
use application::turn::coordinator::{TurnCoordinator, TurnCoordinatorResources};
use runtime::read_model::EventProjectionSink;
use runtime::session_projection::SessionProjectionStore;

pub fn normalize_turn_coordinator_settings(
    grok: &GrokHardeningConfig,
    backpressure: runtime::backpressure::BackpressureConfig,
) -> application::turn::settings::TurnCoordinatorSettings {
    application::turn::settings::TurnCoordinatorSettings {
        prompt_queue: grok.prompt_queue,
        compaction_v2: grok.compaction_v2,
        backpressure,
    }
}

pub fn compose_evaluation_service(
    kernel: Arc<KernelRuntime>,
    data_dir: &Path,
    settings: crate::config::EvaluationSettings,
    projection: Arc<application::evaluation_projection::EvaluationProjection>,
    _capability_rollups: Arc<adapters_sqlite::SqliteCapabilityRollupProjectionSink>,
) -> anyhow::Result<Arc<EvaluationService>> {
    let store: Arc<dyn EvaluationReceiptStore> = Arc::new(
        adapters_sqlite::evaluation::SqliteEvaluationStore::open(data_dir.join("evaluations.db"))?,
    );
    let clock = kernel.clock();
    let max_evaluation_ms = settings.max_evaluation_ms;
    Ok(Arc::new(
        EvaluationService::new(
            Arc::new(DefaultTaskEvaluationContractIssuer::new(
                settings,
                clock.clone(),
            )?),
            Arc::new(DefaultCodingEvidenceCollector::new(
                clock.clone(),
                Arc::new(platform::evaluation_path::PlatformEvaluationPathResolver),
            )),
            Arc::new(crate::adapters::evaluation::CodingV2Scorer),
            Arc::new(crate::adapters::evaluation::MetacogCodingV2Engine),
            Arc::new(crate::adapters::evaluation::KernelEvaluationOperations::new(kernel)),
            store,
            clock,
            max_evaluation_ms,
        )?
        .with_projection(projection),
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
    TurnCoordinator::from_resources(TurnCoordinatorResources {
        clock: kernel.clock(),
        timer: Arc::new(crate::host::runtime::turn_operations::KernelTurnTimer::system()),
        operations: Arc::new(
            crate::host::runtime::turn_operations::KernelTurnOperations::new(kernel),
        ),
        identities: Arc::new(adapters_sqlite::session::turn_identity::CanonicalTurnIdentityAdapter),
        session: Arc::new(
            adapters_sqlite::session::turn_session_port::SessionAppendTurnPort::new(store),
        ),
        settings: normalize_turn_coordinator_settings(
            &grok_hardening,
            runtime::backpressure::BackpressureConfig::default(),
        ),
        runtime_turn_writer: Arc::new(runtime::RuntimeTurnWriter::new()),
    })
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

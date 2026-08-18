//! Integration-fixture builders for the retained Aletheon behavior tests.

use std::sync::Arc;

use ::contracts::SessionAppendStore;
use kernel::KernelRuntime;
use runtime::EventSpine;

use crate::config::GrokHardeningConfig;
use adapters_sqlite::session::event_sourced_store::EventSourcedSessionStore;
use adapters_sqlite::{event_spine::SqliteEventSpine, projection_set::DefaultEventProjectionSet};
use application::turn::coordinator::{TurnCoordinator, TurnCoordinatorResources};
use runtime::read_model::EventProjectionSink;
use runtime::session_projection::SessionProjectionStore;

pub fn compose_turn_coordinator(
    kernel: Arc<KernelRuntime>,
    read_store: Arc<dyn SessionProjectionStore>,
    event_spine: Arc<dyn EventSpine>,
    projections: Arc<dyn EventProjectionSink>,
    grok_hardening: GrokHardeningConfig,
) -> TurnCoordinator {
    let store = compose_session_store(read_store, event_spine, projections);
    compose_from_session_store(kernel, store, grok_hardening)
}

pub fn compose_from_session_store(
    kernel: Arc<KernelRuntime>,
    store: Arc<dyn SessionAppendStore>,
    grok_hardening: GrokHardeningConfig,
) -> TurnCoordinator {
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
        settings: crate::composition::turn_coordinator::normalize_turn_coordinator_settings(
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

pub fn compose_in_memory_session_store(
    read_store: Arc<dyn SessionProjectionStore>,
) -> Arc<dyn SessionAppendStore> {
    compose_session_store(
        read_store,
        Arc::new(SqliteEventSpine::open(":memory:").expect("in-memory event spine")),
        Arc::new(DefaultEventProjectionSet::in_memory()),
    )
}

pub fn compose_in_memory_turn_coordinator(
    kernel: Arc<KernelRuntime>,
    read_store: Arc<dyn SessionProjectionStore>,
) -> TurnCoordinator {
    let event_spine = Arc::new(SqliteEventSpine::open(":memory:").expect("in-memory event spine"));
    let projections = Arc::new(DefaultEventProjectionSet::in_memory());
    compose_turn_coordinator(
        kernel,
        read_store,
        event_spine,
        projections,
        GrokHardeningConfig::default(),
    )
}

pub fn compose_with_event_spine(
    kernel: Arc<KernelRuntime>,
    read_store: Arc<dyn SessionProjectionStore>,
    event_spine: Arc<dyn EventSpine>,
    grok_hardening: GrokHardeningConfig,
) -> TurnCoordinator {
    compose_turn_coordinator(
        kernel,
        read_store,
        event_spine,
        Arc::new(DefaultEventProjectionSet::in_memory()),
        grok_hardening,
    )
}

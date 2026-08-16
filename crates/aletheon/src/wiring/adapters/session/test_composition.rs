//! Integration-fixture builders for the retained Aletheon behavior tests.

use std::sync::Arc;

use ::contracts::SessionAppendStore;
use kernel::KernelRuntime;
use runtime::EventSpine;

use crate::config::GrokHardeningConfig;
use crate::wiring::adapters::session::event_sourced_store::EventSourcedSessionStore;
use crate::wiring::application::turn_coordinator::TurnCoordinator;
use adapters_sqlite::{event_spine::SqliteEventSpine, projection_set::DefaultEventProjectionSet};
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

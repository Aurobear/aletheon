//! Composition of the application turn coordinator with local persistence.

use std::sync::Arc;

use fabric::{EventSpine, SessionAppendStore};
use kernel::KernelRuntime;

use crate::adapters::events::{DefaultEventProjectionSet, SqliteEventSpine};
use crate::adapters::session::event_sourced_store::EventSourcedSessionStore;
use crate::application::event_projection::EventProjectionSink;
use crate::application::turn_coordinator::TurnCoordinator;
use crate::composition::config::GrokHardeningConfig;

pub fn compose_turn_coordinator(
    kernel: Arc<KernelRuntime>,
    read_store: Arc<dyn SessionAppendStore>,
    event_spine: Arc<dyn EventSpine>,
    projections: Arc<dyn EventProjectionSink>,
    grok_hardening: GrokHardeningConfig,
) -> TurnCoordinator {
    let store: Arc<dyn SessionAppendStore> = Arc::new(EventSourcedSessionStore::new(
        read_store.clone(),
        event_spine.clone(),
        projections,
    ));
    TurnCoordinator::from_components(kernel, read_store, store, event_spine, grok_hardening)
}

pub fn compose_in_memory_turn_coordinator(
    kernel: Arc<KernelRuntime>,
    read_store: Arc<dyn SessionAppendStore>,
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
    read_store: Arc<dyn SessionAppendStore>,
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

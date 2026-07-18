//! Crate-private construction support for daemon-turn tests.
//!
//! This deliberately substitutes only the cognitive pipeline runner. Kernel
//! process registration, operation coordination, canonical persistence, and
//! JSON-RPC formatting continue through `DaemonTurnOrchestrator::execute_turn`.

use super::orchestrator::{DaemonTurnOrchestrator, TestTurnRunner};
use crate::r#impl::events::SqliteEventSpine;
use crate::r#impl::session::canonical_store::CanonicalSessionStore;
use crate::service::session_service::SessionService;
use crate::service::turn_coordinator::{TurnCoordinator, TurnExecution};
use aletheon_kernel::chronos::TestClock;
use aletheon_kernel::KernelRuntime;
use fabric::{Clock, SessionAppendStore};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

pub(crate) struct DaemonTurnTestHarness {
    pub(crate) orchestrator: DaemonTurnOrchestrator,
    pub(crate) store: Arc<dyn SessionAppendStore>,
    pub(crate) coordinator: Arc<TurnCoordinator>,
}

pub(crate) struct DaemonTurnTestBuilder {
    runner: TestTurnRunner,
}

impl DaemonTurnTestBuilder {
    pub(crate) fn new(runner: TestTurnRunner) -> Self {
        Self { runner }
    }

    pub(crate) fn succeeding(output: impl Into<String>) -> Self {
        let output = output.into();
        Self::new(Arc::new(move |_request, _cancel| {
            let output = output.clone();
            Box::pin(async move {
                Ok(TurnExecution {
                    result: fabric::TurnResult {
                        output,
                        stop: fabric::TurnStop::Completed,
                        metrics: fabric::TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: vec![],
                    projection: None,
                    context_projection: None,
                })
            })
        }))
    }

    pub(crate) fn failing(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::new(Arc::new(move |_request, _cancel| {
            let message = message.clone();
            Box::pin(async move { anyhow::bail!(message) })
        }))
    }

    pub(crate) async fn build(self) -> DaemonTurnTestHarness {
        let clock = Arc::new(TestClock::default());
        let kernel = Arc::new(KernelRuntime::with_clock(clock as Arc<dyn Clock>));
        let store: Arc<dyn SessionAppendStore> =
            Arc::new(CanonicalSessionStore::open(":memory:").expect("test session store"));
        let spine = Arc::new(SqliteEventSpine::open(":memory:").expect("test event spine"));
        let coordinator = Arc::new(TurnCoordinator::with_event_spine(
            kernel.clone(),
            store.clone(),
            spine,
        ));
        let session_service = Arc::new(SessionService::new(
            store.clone(),
            Arc::new(Mutex::new(Default::default())),
        ));
        let orchestrator = DaemonTurnOrchestrator {
            kernel: kernel.clone(),
            notify_tx: Arc::new(Mutex::new(None::<mpsc::Sender<String>>)),
            main_agent_process_id: Arc::new(Mutex::new(None)),
            turn_token: Arc::new(Mutex::new(None)),
            pipeline: None,
            coordinator: coordinator.clone(),
            session_service,
            grok_hardening: Default::default(),
            test_runner: Some(self.runner),
        };
        DaemonTurnTestHarness {
            orchestrator,
            store,
            coordinator,
        }
    }
}

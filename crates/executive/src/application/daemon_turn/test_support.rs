//! Crate-private construction support for daemon-turn tests.
//!
//! This deliberately substitutes only the cognitive pipeline runner. Kernel
//! process registration, operation coordination, canonical persistence, and
//! JSON-RPC formatting continue through `DaemonTurnOrchestrator::execute_turn`.

use super::orchestrator::{DaemonTurnOrchestrator, TestTurnRunner};
use crate::adapters::events::SqliteEventSpine;
use crate::adapters::session::canonical_store::CanonicalSessionStore;
use crate::application::session_service::SessionService;
use crate::application::turn_coordinator::{TurnCoordinator, TurnExecution};
use crate::application::turn_runtime_ports::{ActiveAgentProfilePort, ResolvedTurnProfile};
use fabric::{Clock, SessionAppendStore};
use kernel::chronos::TestClock;
use kernel::KernelRuntime;
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
        let coordinator = Arc::new(
            crate::composition::turn_coordinator::compose_with_event_spine(
                kernel.clone(),
                store.clone(),
                spine,
                crate::composition::config::GrokHardeningConfig::default(),
            ),
        );
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
            turn_engine: None,
            coordinator: coordinator.clone(),
            session_service,
            grok_hardening: Default::default(),
            active_profile: Arc::new(StubActiveAgentProfile),
            test_runner: Some(self.runner),
        };
        DaemonTurnTestHarness {
            orchestrator,
            store,
            coordinator,
        }
    }
}

/// Minimal stub that returns a default profile for daemon-turn tests.
struct StubActiveAgentProfile;

#[async_trait::async_trait]
impl ActiveAgentProfilePort for StubActiveAgentProfile {
    async fn snapshot(&self) -> anyhow::Result<ResolvedTurnProfile> {
        Ok(ResolvedTurnProfile {
            profile_name: "stub".into(),
            allowed_tools: Default::default(),
            system_prompt: String::new(),
            model_policy: None,
            max_iterations: 0,
            max_input_tokens: 0,
            max_output_tokens: 0,
            max_tool_calls: 0,
            max_elapsed_ms: 0,
            approval_policy: fabric::AgentApprovalPolicy::AutoApprove,
            tool_timeout_ms: 30_000,
        })
    }
}

#![allow(dead_code)]

use executive::testing::agent_control::SqliteAgentRunRepository;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use executive::application::agent_control::{
    AgentControlService, AgentEventSink, AgentRuntimeInput, AgentRuntimeLauncher,
    AgentRuntimeRegistry, BoundedAgentAdmission,
};
use fabric::{
    AgentBudget, AgentContextFork, AgentControlError, AgentControlErrorKind, AgentControlPort,
    AgentId, AgentProfileId, AgentResult, AgentSpawnRequest, AttemptUsage, ProcessId, RuntimeId,
};
use kernel::chronos::TestClock;
use kernel::KernelRuntime;
use tokio::sync::Notify;

pub const TEST_RUNTIME: &str = "test-runtime";

pub struct TestLauncher {
    started: AtomicBool,
    calls: AtomicUsize,
    started_notify: Notify,
    release: Notify,
    fail: Option<String>,
}

impl TestLauncher {
    pub fn blocked() -> Arc<Self> {
        Arc::new(Self {
            started: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            started_notify: Notify::new(),
            release: Notify::new(),
            fail: None,
        })
    }

    pub fn failing(message: &str) -> Arc<Self> {
        Arc::new(Self {
            started: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            started_notify: Notify::new(),
            release: Notify::new(),
            fail: Some(message.into()),
        })
    }

    pub async fn wait_started(&self) {
        while !self.started.load(Ordering::SeqCst) {
            self.started_notify.notified().await;
        }
    }

    pub fn complete(&self) {
        self.release.notify_one();
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl AgentRuntimeLauncher for TestLauncher {
    async fn launch(
        &self,
        input: AgentRuntimeInput,
        _events: Arc<dyn AgentEventSink>,
    ) -> Result<AgentResult, AgentControlError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.store(true, Ordering::SeqCst);
        self.started_notify.notify_waiters();
        if let Some(message) = &self.fail {
            return Err(AgentControlError {
                kind: AgentControlErrorKind::Runtime,
                message: message.clone(),
            });
        }
        tokio::select! {
            _ = input.cancellation.cancelled() => Err(AgentControlError {
                kind: AgentControlErrorKind::Terminal,
                message: "cancelled by test".into(),
            }),
            _ = self.release.notified() => Ok(AgentResult {
                output: format!("completed: {}", input.request.task),
                usage: AttemptUsage::default(),
                evidence: vec![],
                artifacts: vec![],
            }),
        }
    }
}

pub struct Fixture {
    pub service: Arc<AgentControlService>,
    pub port: Arc<dyn AgentControlPort>,
    pub kernel: Arc<KernelRuntime>,
    pub repository: Arc<SqliteAgentRunRepository>,
    pub runtimes: Arc<AgentRuntimeRegistry>,
    pub admission: Arc<BoundedAgentAdmission>,
}

pub fn fixture(max_concurrent: usize, launcher: Arc<dyn AgentRuntimeLauncher>) -> Fixture {
    let clock = Arc::new(TestClock::new(1_700_000_000_000, 0));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let repository = Arc::new(SqliteAgentRunRepository::in_memory().unwrap());
    let runtimes = Arc::new(AgentRuntimeRegistry::default());
    runtimes
        .register(RuntimeId(TEST_RUNTIME.into()), launcher)
        .unwrap();
    let admission = Arc::new(BoundedAgentAdmission::new(max_concurrent).unwrap());
    let service = Arc::new(AgentControlService::new(
        kernel.clone(),
        clock,
        repository.clone(),
        admission.clone(),
        runtimes.clone(),
        Arc::new(executive::runtime::events::SqliteEventSpine::open(":memory:").unwrap()),
    ));
    Fixture {
        port: service.clone(),
        service,
        kernel,
        repository,
        runtimes,
        admission,
    }
}

pub fn spawn_request(root: AgentId, parent: Option<(AgentId, ProcessId)>) -> AgentSpawnRequest {
    AgentSpawnRequest {
        root_agent_id: root,
        parent_agent_id: parent.map(|value| value.0),
        parent_process_id: parent.map(|value| value.1),
        profile_id: AgentProfileId("worker".into()),
        runtime_id: RuntimeId(TEST_RUNTIME.into()),
        trusted_workspace: None,
        task: "perform controlled work".into(),
        context: AgentContextFork::SelectedProjection {
            items: vec!["labelled context".into()],
        },
        broadcast_refs: vec![],
        allowed_tools: vec!["file_read".into()],
        background_decls: vec![],
        budget: AgentBudget {
            max_input_tokens: 1_000,
            max_output_tokens: 1_000,
            max_tool_calls: 10,
            max_elapsed_ms: 60_000,
            max_cost_usd: Some(1.0),
            max_depth: 3,
        },
    }
}

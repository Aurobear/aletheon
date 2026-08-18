#![allow(dead_code)]

use adapters_sqlite::runtime_agent::SqliteAgentRunProjection;
use aletheon::host::runtime::test_registry::AgentExecutionRegistry;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use ::contracts::{
    AgentBudget, AgentContextFork, AgentControlError, AgentControlErrorKind, AgentControlPort,
    AgentId, AgentProfileId, AgentResult, AgentSpawnRequest, AttemptUsage, ProcessId, RuntimeId,
};
use aletheon::composition::agent_control::{
    AgentEventSink, AgentHostAdapter, AgentRuntimeInput, AgentRuntimeLauncher,
    BoundedAgentAdmission,
};
use async_trait::async_trait;
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
    inputs: StdMutex<Vec<AgentRuntimeInput>>,
}

impl TestLauncher {
    pub fn blocked() -> Arc<Self> {
        Arc::new(Self {
            started: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            started_notify: Notify::new(),
            release: Notify::new(),
            fail: None,
            inputs: StdMutex::new(Vec::new()),
        })
    }

    pub fn failing(message: &str) -> Arc<Self> {
        Arc::new(Self {
            started: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            started_notify: Notify::new(),
            release: Notify::new(),
            fail: Some(message.into()),
            inputs: StdMutex::new(Vec::new()),
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

    pub fn inputs(&self) -> Vec<AgentRuntimeInput> {
        self.inputs.lock().unwrap().clone()
    }
}

#[async_trait]
impl AgentRuntimeLauncher for TestLauncher {
    async fn launch(
        &self,
        input: AgentRuntimeInput,
        _events: Arc<dyn AgentEventSink>,
    ) -> Result<AgentResult, AgentControlError> {
        self.inputs.lock().unwrap().push(input.clone());
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
    pub service: Arc<AgentHostAdapter>,
    pub port: Arc<dyn AgentControlPort>,
    pub kernel: Arc<KernelRuntime>,
    pub repository: Arc<SqliteAgentRunProjection>,
    pub runtimes: Arc<AgentExecutionRegistry>,
    pub admission: Arc<BoundedAgentAdmission>,
}

pub fn fixture(max_concurrent: usize, launcher: Arc<dyn AgentRuntimeLauncher>) -> Fixture {
    fixture_with_task_admission(max_concurrent, launcher, None)
}

pub fn fixture_with_task_admission(
    max_concurrent: usize,
    launcher: Arc<dyn AgentRuntimeLauncher>,
    task_admission: Option<Arc<dyn runtime::agent_admission::CognitiveTaskAdmissionPort>>,
) -> Fixture {
    let clock = Arc::new(TestClock::new(1_700_000_000_000, 0));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let repository = Arc::new(SqliteAgentRunProjection::in_memory().unwrap());
    let runtimes = Arc::new(AgentExecutionRegistry::default());
    runtimes
        .register(RuntimeId(TEST_RUNTIME.into()), launcher)
        .unwrap();
    let admission = Arc::new(BoundedAgentAdmission::new(max_concurrent).unwrap());
    let mut service = AgentHostAdapter::new_fixture(
        kernel.clone(),
        clock,
        repository.clone(),
        admission.clone(),
        runtimes.clone(),
        Arc::new(adapters_sqlite::event_spine::SqliteEventSpine::open(":memory:").unwrap()),
    );
    if let Some(task_admission) = task_admission {
        service = service.with_cognitive_task_admission(task_admission);
    }
    let service = Arc::new(service);
    Fixture {
        port: Arc::new(
            aletheon::composition::agent_control::RuntimeAgentControlFacade::new(service.clone()),
        ),
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
        delegator_authority: None,
        cognitive_binding: None,
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

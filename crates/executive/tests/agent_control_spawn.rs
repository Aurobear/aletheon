mod agent_control_support;

use std::sync::Arc;
use std::time::Duration;

use agent_control_support::{fixture, spawn_request, TestLauncher, TEST_RUNTIME};
use executive::application::agent_control::AgentRuntimeLauncher;
use executive::application::agent_control::{
    AgentControlService, AgentRuntimeRegistry, BoundedAgentAdmission, SettlementReceiptStore,
};
use executive::testing::agent_control::SqliteAgentRunRepository;
use fabric::{
    AgentApprovalPolicy, AgentBudget, AgentContextFork, AgentControlError, AgentControlErrorKind,
    AgentControlPort, AgentId, AgentProfile, AgentProfileId, AgentRunStatus,
    AgentRuntimeCapability, AgentSpawnIntent, AgentWaitRequest, ParentRestriction, RiskTier,
    RuntimeId, SettlementReceipt,
};
use kernel::chronos::TestClock;
use kernel::KernelRuntime;
use std::collections::BTreeSet;
use std::collections::HashMap;

fn profile(
    id: &str,
    callable: &[&str],
    delegated: &[&str],
    risk_tier: RiskTier,
) -> AgentProfile {
    AgentProfile {
        id: AgentProfileId(id.into()),
        system_prompt: format!("{id} fixture"),
        model: "fixture-model".into(),
        allowed_tools: callable.iter().map(|tool| (*tool).into()).collect(),
        delegated_tools: delegated.iter().map(|tool| (*tool).into()).collect(),
        max_iterations: 4,
        max_input_tokens: 8_000,
        max_output_tokens: 1_000,
        max_tool_calls: 8,
        max_elapsed_ms: 60_000,
        profile_name: id.into(),
        risk_tier,
        approval_policy: AgentApprovalPolicy::PromptUser,
        tool_timeout_ms: 30_000,
        inheritable: true,
        parent_restriction: ParentRestriction::SameOrSafer,
    }
}

struct RejectingSettlementStore;

#[async_trait::async_trait]
impl SettlementReceiptStore for RejectingSettlementStore {
    async fn get(
        &self,
        _idempotency_key: &str,
    ) -> Result<Option<SettlementReceipt>, AgentControlError> {
        Ok(None)
    }

    async fn put_if_absent(
        &self,
        _receipt: SettlementReceipt,
    ) -> Result<SettlementReceipt, AgentControlError> {
        Err(AgentControlError {
            kind: AgentControlErrorKind::Persistence,
            message: "receipt store unavailable".into(),
        })
    }
}

#[tokio::test]
async fn runtime_resolution_and_admission_timeout_fail_before_process_creation() {
    let launcher = TestLauncher::blocked();
    let fixture = fixture(1, launcher.clone());
    let unknown_root = AgentId::new();
    let mut unknown = spawn_request(unknown_root, None);
    unknown.runtime_id = RuntimeId("missing".into());
    let error = fixture.port.spawn(unknown).await.unwrap_err();
    assert_eq!(error.kind, AgentControlErrorKind::NotFound);
    assert!(fixture
        .kernel
        .identity_for_agent(unknown_root)
        .await
        .is_none());

    let first_root = AgentId::new();
    fixture
        .port
        .spawn(spawn_request(first_root, None))
        .await
        .unwrap();
    launcher.wait_started().await;
    let rejected_root = AgentId::new();
    let mut rejected = spawn_request(rejected_root, None);
    rejected.budget.max_elapsed_ms = 20;
    let error = fixture.port.spawn(rejected).await.unwrap_err();
    assert_eq!(error.kind, AgentControlErrorKind::Timeout);
    assert!(fixture
        .kernel
        .identity_for_agent(rejected_root)
        .await
        .is_none());
    fixture.port.cancel(first_root, first_root).await.unwrap();
}

#[tokio::test]
async fn one_runtime_task_reaches_durable_terminal_state() {
    let launcher = TestLauncher::blocked();
    let fixture = fixture(2, launcher.clone());
    let root = AgentId::new();
    let handle = fixture.port.spawn(spawn_request(root, None)).await.unwrap();
    launcher.wait_started().await;
    assert_eq!(launcher.calls(), 1);
    launcher.complete();
    let snapshot = fixture
        .port
        .wait(AgentWaitRequest {
            caller_root_agent_id: root,
            agent_id: handle.agent_id,
            timeout_ms: 2_000,
        })
        .await
        .unwrap();
    assert_eq!(snapshot.status, AgentRunStatus::Succeeded);
    assert!(snapshot.result.unwrap().output.contains("controlled work"));
    tokio::time::timeout(Duration::from_secs(1), async {
        while fixture
            .service
            .live_runs()
            .get(handle.agent_id)
            .await
            .is_some()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        fixture
            .port
            .inspect(root, handle.agent_id)
            .await
            .unwrap()
            .status,
        AgentRunStatus::Succeeded
    );
}

#[tokio::test]
async fn a_agent_001_launcher_failure_is_terminal_and_releases_admission() {
    let launcher: Arc<dyn AgentRuntimeLauncher> = TestLauncher::failing("provider failed");
    let fixture = fixture(1, launcher);
    let root = AgentId::new();
    let handle = fixture.port.spawn(spawn_request(root, None)).await.unwrap();
    let snapshot = fixture
        .port
        .wait(AgentWaitRequest {
            caller_root_agent_id: root,
            agent_id: handle.agent_id,
            timeout_ms: 2_000,
        })
        .await
        .unwrap();
    assert_eq!(snapshot.status, AgentRunStatus::Failed);
    assert!(snapshot.last_error.unwrap().contains("provider failed"));
    assert_eq!(fixture.admission.available_permits(), 1);
    assert_eq!(
        fixture
            .kernel
            .inspect_process(handle.process_id)
            .await
            .unwrap()
            .state,
        fabric::ProcessState::Failed
    );
    assert!(fixture
        .runtimes
        .resolve(&RuntimeId(TEST_RUNTIME.into()))
        .is_ok());
}

async fn assert_child_success_requires_host_receipt() {
    let launcher = TestLauncher::blocked();
    let clock = Arc::new(TestClock::new(1_700_000_000_000, 0));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let repository = Arc::new(SqliteAgentRunRepository::in_memory().unwrap());
    let runtimes = Arc::new(AgentRuntimeRegistry::default());
    runtimes
        .register(RuntimeId(TEST_RUNTIME.into()), launcher.clone())
        .unwrap();
    let service = Arc::new(
        AgentControlService::new(
            kernel,
            clock,
            repository,
            Arc::new(BoundedAgentAdmission::new(1).unwrap()),
            runtimes,
            Arc::new(executive::runtime::events::SqliteEventSpine::open(":memory:").unwrap()),
        )
        .with_subagent_settlement("daemon:receipt-test", Arc::new(RejectingSettlementStore)),
    );
    let root = AgentId::new();
    let handle = service.spawn(spawn_request(root, None)).await.unwrap();
    launcher.wait_started().await;
    launcher.complete();

    let snapshot = service
        .wait(AgentWaitRequest {
            caller_root_agent_id: root,
            agent_id: handle.agent_id,
            timeout_ms: 2_000,
        })
        .await
        .unwrap();
    assert_eq!(snapshot.status, AgentRunStatus::Failed);
    assert!(snapshot.result.is_none());
    assert!(snapshot
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("receipt store unavailable")));
}

#[tokio::test]
async fn u_resume_003_child_self_report_cannot_advance_parent_without_receipt() {
    assert_child_success_requires_host_receipt().await;
}

#[tokio::test]
async fn a_agent_001_child_success_requires_host_terminal_receipt() {
    assert_child_success_requires_host_receipt().await;
}

#[tokio::test]
async fn generic_spawn_selects_only_a_compatible_manifested_runtime() {
    let explicit_only = TestLauncher::blocked();
    let fixture = fixture(2, explicit_only.clone());
    let selected = TestLauncher::blocked();
    fixture
        .runtimes
        .register_manifested(
            RuntimeId("selectable-analysis".into()),
            selected.clone(),
            runtime::RuntimeManifest {
                id: "selectable-analysis".into(),
                aliases: vec!["analysis".into()],
                display_name: "Selectable Analysis".into(),
                capabilities: BTreeSet::from([
                    runtime::RuntimeCapability::CodeRead,
                    runtime::RuntimeCapability::CodeSearch,
                ]),
                interaction_modes: BTreeSet::from([runtime::InteractionMode::Resident]),
                workspace_modes: BTreeSet::from([runtime::WorkspaceMode::WorkspaceLess]),
                task_encodings: BTreeSet::from([runtime::TaskEncoding::NaturalLanguage]),
                supported_profiles: None,
                tool_governance: runtime::ToolGovernance::Observed,
                priority: 0,
                max_context_tokens: Some(10_000),
                resource_requirements: Default::default(),
            },
        )
        .unwrap();

    let root = AgentId::new();
    let handle = fixture
        .port
        .spawn_intent(AgentSpawnIntent {
            root_agent_id: root,
            parent_agent_id: None,
            parent_process_id: None,
            profile_id: AgentProfileId("researcher".into()),
            runtime_override: None,
            required_capabilities: vec![
                AgentRuntimeCapability::CodeRead,
                AgentRuntimeCapability::CodeSearch,
            ],
            trusted_workspace: None,
            delegator_authority: None,
            task: "analyze the repository".into(),
            context: AgentContextFork::None,
            allowed_tools: vec!["file_read".into()],
            budget: AgentBudget {
                max_input_tokens: 1_000,
                max_output_tokens: 1_000,
                max_tool_calls: 10,
                max_elapsed_ms: 60_000,
                max_cost_usd: None,
                max_depth: 2,
            },
        })
        .await
        .unwrap();
    assert_eq!(handle.runtime_id.0, "selectable-analysis");
    selected.wait_started().await;
    assert_eq!(selected.calls(), 1);
    assert_eq!(explicit_only.calls(), 0);
    fixture.port.cancel(root, handle.agent_id).await.unwrap();
}

#[tokio::test]
async fn explicit_only_runtime_remains_available_without_capability_bypass() {
    let launcher = TestLauncher::blocked();
    let fixture = fixture(1, launcher.clone());
    let root = AgentId::new();
    let handle = fixture
        .port
        .spawn_intent(AgentSpawnIntent {
            root_agent_id: root,
            parent_agent_id: None,
            parent_process_id: None,
            profile_id: AgentProfileId("legacy-coder".into()),
            runtime_override: Some(TEST_RUNTIME.into()),
            required_capabilities: vec![],
            trusted_workspace: None,
            delegator_authority: None,
            task: r#"{"task":"legacy structured request"}"#.into(),
            context: AgentContextFork::None,
            allowed_tools: vec![],
            budget: AgentBudget {
                max_input_tokens: 1_000,
                max_output_tokens: 1_000,
                max_tool_calls: 10,
                max_elapsed_ms: 60_000,
                max_cost_usd: None,
                max_depth: 2,
            },
        })
        .await
        .unwrap();
    assert_eq!(handle.runtime_id.0, TEST_RUNTIME);
    launcher.wait_started().await;
    assert_eq!(launcher.calls(), 1);
    fixture.port.cancel(root, handle.agent_id).await.unwrap();
}

#[tokio::test]
async fn host_resolves_child_profile_tools_and_preserves_separate_delegation_authority() {
    let launcher = TestLauncher::blocked();
    let clock = Arc::new(TestClock::new(1_700_000_000_000, 0));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let repository = Arc::new(SqliteAgentRunRepository::in_memory().unwrap());
    let runtimes = Arc::new(AgentRuntimeRegistry::default());
    runtimes
        .register(RuntimeId(TEST_RUNTIME.into()), launcher.clone())
        .unwrap();
    let profiles = HashMap::from([
        (
            "orchestrator".into(),
            profile(
                "orchestrator",
                &["agent_spawn"],
                &["file_read", "exec_command"],
                RiskTier::Sandboxed,
            ),
        ),
        (
            "code".into(),
            profile(
                "code",
                &["file_read", "exec_command"],
                &["file_read"],
                RiskTier::Sandboxed,
            ),
        ),
    ]);
    let service = Arc::new(
        AgentControlService::new(
            kernel,
            clock,
            repository,
            Arc::new(BoundedAgentAdmission::new(3).unwrap()),
            runtimes,
            Arc::new(executive::runtime::events::SqliteEventSpine::open(":memory:").unwrap()),
        )
        .with_agent_profiles(profiles),
    );

    let root = AgentId::new();
    let mut root_request = spawn_request(root, None);
    root_request.profile_id = AgentProfileId("orchestrator".into());
    root_request.allowed_tools = vec!["agent_spawn".into()];
    let root_handle = service.spawn(root_request).await.unwrap();
    launcher.wait_started().await;

    let child = service
        .spawn_intent(AgentSpawnIntent {
            root_agent_id: root,
            parent_agent_id: Some(root_handle.agent_id),
            parent_process_id: Some(root_handle.process_id),
            profile_id: AgentProfileId("code".into()),
            runtime_override: Some(TEST_RUNTIME.into()),
            required_capabilities: vec![],
            trusted_workspace: None,
            delegator_authority: None,
            task: "implement the bounded change".into(),
            context: AgentContextFork::None,
            allowed_tools: vec![],
            budget: AgentBudget {
                max_input_tokens: 1_000,
                max_output_tokens: 500,
                max_tool_calls: 8,
                max_elapsed_ms: 60_000,
                max_cost_usd: None,
                max_depth: 3,
            },
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while launcher.calls() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let inputs = launcher.inputs();
    let child_input = inputs
        .iter()
        .find(|input| input.handle.agent_id == child.agent_id)
        .unwrap();
    assert_eq!(
        child_input.request.allowed_tools,
        vec!["file_read", "exec_command"]
    );
    assert_eq!(
        child_input.delegation_authority.allowed_tools,
        vec!["file_read"]
    );
    assert_eq!(
        inputs
            .iter()
            .find(|input| input.handle.agent_id == root_handle.agent_id)
            .unwrap()
            .request
            .allowed_tools,
        vec!["agent_spawn"]
    );

    service.cancel(root, child.agent_id).await.unwrap();
    service.cancel(root, root_handle.agent_id).await.unwrap();
}

mod agent_control_support;

use std::sync::Arc;
use std::time::Duration;

use agent_control_support::{fixture, spawn_request, TestLauncher, TEST_RUNTIME};
use executive::application::agent_control::AgentRuntimeLauncher;
use fabric::{
    AgentBudget, AgentContextFork, AgentControlErrorKind, AgentId, AgentProfileId, AgentRunStatus,
    AgentRuntimeCapability, AgentSpawnIntent, AgentWaitRequest, RuntimeId,
};
use std::collections::BTreeSet;

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
async fn launcher_failure_is_terminal_and_releases_admission() {
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
                workspace_modes: BTreeSet::from([runtime::WorkspaceMode::SharedReadOnly]),
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

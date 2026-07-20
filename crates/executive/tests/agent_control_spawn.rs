mod agent_control_support;

use std::sync::Arc;
use std::time::Duration;

use agent_control_support::{fixture, spawn_request, TestLauncher, TEST_RUNTIME};
use executive::service::agent_control::AgentRuntimeLauncher;
use fabric::{AgentControlErrorKind, AgentId, AgentRunStatus, AgentWaitRequest, RuntimeId};

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

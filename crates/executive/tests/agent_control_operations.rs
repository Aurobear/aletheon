mod agent_control_support;

use agent_control_support::{fixture, spawn_request, TestLauncher};
use fabric::{
    AgentControlErrorKind, AgentId, AgentListRequest, AgentRunStatus, AgentSendRequest,
    AgentWaitRequest,
};

#[tokio::test]
async fn operations_enforce_root_scope_and_wait_timeout() {
    let launcher = TestLauncher::blocked();
    let fixture = fixture(2, launcher.clone());
    let root = AgentId::new();
    let other_root = AgentId::new();
    let handle = fixture.port.spawn(spawn_request(root, None)).await.unwrap();
    launcher.wait_started().await;

    let timeout = fixture
        .port
        .wait(AgentWaitRequest {
            caller_root_agent_id: root,
            agent_id: handle.agent_id,
            timeout_ms: 5,
        })
        .await
        .unwrap_err();
    assert_eq!(timeout.kind, AgentControlErrorKind::Timeout);
    assert_eq!(
        fixture
            .port
            .inspect(other_root, handle.agent_id)
            .await
            .unwrap_err()
            .kind,
        AgentControlErrorKind::Forbidden
    );
    assert!(fixture
        .port
        .list(AgentListRequest {
            caller_root_agent_id: other_root,
            status: None,
            limit: 10,
        })
        .await
        .unwrap()
        .is_empty());

    fixture.port.cancel(root, handle.agent_id).await.unwrap();
}

#[tokio::test]
async fn send_is_sequenced_and_cancel_is_idempotent() {
    let launcher = TestLauncher::blocked();
    let fixture = fixture(2, launcher.clone());
    let root = AgentId::new();
    let handle = fixture.port.spawn(spawn_request(root, None)).await.unwrap();
    launcher.wait_started().await;

    let first = fixture
        .port
        .send(AgentSendRequest {
            caller_root_agent_id: root,
            agent_id: handle.agent_id,
            message: "first".into(),
            start_turn: false,
        })
        .await
        .unwrap();
    let second = fixture
        .port
        .send(AgentSendRequest {
            caller_root_agent_id: root,
            agent_id: handle.agent_id,
            message: "second".into(),
            start_turn: true,
        })
        .await
        .unwrap();
    assert_eq!((first.sequence, second.sequence), (1, 2));

    let cancelled = fixture.port.cancel(root, handle.agent_id).await.unwrap();
    assert_eq!(cancelled.status, AgentRunStatus::Cancelled);
    let repeated = fixture.port.cancel(root, handle.agent_id).await.unwrap();
    assert_eq!(repeated, cancelled);
    let terminal_send = fixture
        .port
        .send(AgentSendRequest {
            caller_root_agent_id: root,
            agent_id: handle.agent_id,
            message: "too late".into(),
            start_turn: false,
        })
        .await
        .unwrap_err();
    assert_eq!(terminal_send.kind, AgentControlErrorKind::Terminal);
}

#[tokio::test]
async fn child_parent_identity_is_validated_server_side() {
    let launcher = TestLauncher::blocked();
    let fixture = fixture(3, launcher.clone());
    let root = AgentId::new();
    let parent = fixture.port.spawn(spawn_request(root, None)).await.unwrap();
    launcher.wait_started().await;
    let child = fixture
        .port
        .spawn(spawn_request(
            root,
            Some((parent.agent_id, parent.process_id)),
        ))
        .await
        .unwrap();
    assert_eq!(child.root_agent_id, root);
    assert_eq!(child.parent_agent_id, Some(parent.agent_id));

    let forged_root = AgentId::new();
    let error = fixture
        .port
        .spawn(spawn_request(
            forged_root,
            Some((parent.agent_id, parent.process_id)),
        ))
        .await
        .unwrap_err();
    assert_eq!(error.kind, AgentControlErrorKind::Forbidden);
    fixture.port.cancel(root, child.agent_id).await.unwrap();
    fixture.port.cancel(root, parent.agent_id).await.unwrap();
}

mod agent_control_support;

use agent_control_support::{fixture, spawn_request, TestLauncher};
use fabric::{
    AgentControlErrorKind, AgentId, AgentListRequest, AgentProfileId, AgentRunStatus,
    AgentSendRequest, AgentWaitRequest, NamespaceId, ProcessSignal, SpawnSpec,
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
async fn trusted_live_root_process_can_spawn_its_first_durable_child() {
    let launcher = TestLauncher::blocked();
    let fixture = fixture(2, launcher.clone());
    let root = AgentId::new();
    let process = fixture
        .kernel
        .spawn_process(SpawnSpec {
            agent_id: root,
            profile: AgentProfileId("main".into()),
            namespace: NamespaceId("main-session".into()),
            ..SpawnSpec::default()
        })
        .await
        .unwrap();
    fixture
        .kernel
        .signal_process(process.id, ProcessSignal::Start)
        .await
        .unwrap();

    let child = fixture
        .port
        .spawn(spawn_request(root, Some((root, process.id))))
        .await
        .unwrap();
    assert_ne!(child.agent_id, root);
    assert_eq!(child.root_agent_id, root);
    assert_eq!(child.parent_agent_id, Some(root));
    fixture.port.cancel(root, child.agent_id).await.unwrap();
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
            sender_agent_id: None,
            agent_id: handle.agent_id,
            kind: fabric::AgentMessageKind::Input,
            delivery_id: None,
            correlation_id: None,
            deadline_mono_ms: None,
            message: "first".into(),
            start_turn: false,
        })
        .await
        .unwrap();
    let second = fixture
        .port
        .send(AgentSendRequest {
            caller_root_agent_id: root,
            sender_agent_id: None,
            agent_id: handle.agent_id,
            kind: fabric::AgentMessageKind::Input,
            delivery_id: None,
            correlation_id: None,
            deadline_mono_ms: None,
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
            sender_agent_id: None,
            agent_id: handle.agent_id,
            kind: fabric::AgentMessageKind::Input,
            delivery_id: None,
            correlation_id: None,
            deadline_mono_ms: None,
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

#[tokio::test]
async fn cancelling_parent_propagates_to_live_child_runtime() {
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

    assert_eq!(
        fixture
            .port
            .cancel(root, parent.agent_id)
            .await
            .unwrap()
            .status,
        AgentRunStatus::Cancelled
    );
    assert_eq!(
        fixture
            .port
            .wait(AgentWaitRequest {
                caller_root_agent_id: root,
                agent_id: child.agent_id,
                timeout_ms: 1_000,
            })
            .await
            .unwrap()
            .status,
        AgentRunStatus::Cancelled
    );
}

fn send_request(root: AgentId, sender: AgentId, target: AgentId, text: &str) -> AgentSendRequest {
    AgentSendRequest {
        caller_root_agent_id: root,
        sender_agent_id: Some(sender),
        agent_id: target,
        kind: fabric::AgentMessageKind::Input,
        delivery_id: None,
        correlation_id: None,
        deadline_mono_ms: None,
        message: text.into(),
        start_turn: false,
    }
}

#[tokio::test]
async fn topology_allows_direct_family_and_only_explicit_sibling_routes() {
    let launcher = TestLauncher::blocked();
    let fixture = fixture(3, launcher.clone());
    let root = AgentId::new();
    let parent = fixture.port.spawn(spawn_request(root, None)).await.unwrap();
    launcher.wait_started().await;
    let left = fixture
        .port
        .spawn(spawn_request(
            root,
            Some((parent.agent_id, parent.process_id)),
        ))
        .await
        .unwrap();
    let right = fixture
        .port
        .spawn(spawn_request(
            root,
            Some((parent.agent_id, parent.process_id)),
        ))
        .await
        .unwrap();

    fixture
        .port
        .send(send_request(
            root,
            parent.agent_id,
            left.agent_id,
            "parent-child",
        ))
        .await
        .unwrap();
    fixture
        .port
        .send(send_request(
            root,
            left.agent_id,
            parent.agent_id,
            "child-parent",
        ))
        .await
        .unwrap();
    assert_eq!(
        fixture
            .port
            .send(send_request(
                root,
                left.agent_id,
                right.agent_id,
                "forbidden"
            ))
            .await
            .unwrap_err()
            .kind,
        AgentControlErrorKind::Forbidden
    );
    fixture
        .service
        .permit_sibling_route(root, parent.agent_id, left.agent_id, right.agent_id)
        .await
        .unwrap();
    fixture
        .port
        .send(send_request(
            root,
            left.agent_id,
            right.agent_id,
            "permitted",
        ))
        .await
        .unwrap();

    fixture.port.cancel(root, left.agent_id).await.unwrap();
    fixture.port.cancel(root, right.agent_id).await.unwrap();
    fixture.port.cancel(root, parent.agent_id).await.unwrap();
}

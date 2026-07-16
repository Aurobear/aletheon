use executive::service::agent_control::{
    AgentRunRecord, AgentRunRepository, SqliteAgentRunRepository,
};
use fabric::{
    AgentBudget, AgentContextFork, AgentHandle, AgentId, AgentMessageDeliveryState,
    AgentMessageKind, AgentMessagePayload, AgentProfileId, AgentRunStatus, AgentSnapshot,
    AgentSpawnRequest, OperationId, ProcessId, RuntimeId, AGENT_MESSAGE_SCHEMA_V1,
};

fn run(root: AgentId) -> AgentRunRecord {
    let request = AgentSpawnRequest {
        root_agent_id: root,
        parent_agent_id: None,
        parent_process_id: None,
        profile_id: AgentProfileId("worker".into()),
        runtime_id: RuntimeId("test".into()),
        task: "mailbox test".into(),
        context: AgentContextFork::None,
        broadcast_refs: vec![],
        allowed_tools: vec![],
        budget: AgentBudget {
            max_input_tokens: 100,
            max_output_tokens: 100,
            max_tool_calls: 1,
            max_elapsed_ms: 1_000,
            max_cost_usd: None,
            max_depth: 1,
        },
    };
    let process = ProcessId::new();
    AgentRunRecord {
        snapshot: AgentSnapshot {
            handle: AgentHandle {
                agent_id: root,
                root_agent_id: root,
                parent_agent_id: None,
                process_id: process,
                operation_id: OperationId::new(),
                runtime_id: request.runtime_id.clone(),
                profile_id: request.profile_id.clone(),
            },
            status: AgentRunStatus::Queued,
            result: None,
            created_at_ms: 1,
            started_at_ms: None,
            ended_at_ms: None,
            last_error: None,
        },
        request_hash: SqliteAgentRunRepository::request_hash(&request).unwrap(),
        workspace_id: executive::service::agent_control::agent_workspace_id(root),
        root_process_id: process,
        broadcast_refs: vec![],
        request,
        version: 0,
        retain_until_ms: 10_000,
    }
}

fn payload(content: &str) -> AgentMessagePayload {
    AgentMessagePayload {
        schema_version: AGENT_MESSAGE_SCHEMA_V1,
        kind: AgentMessageKind::Input,
        content: content.into(),
        start_turn: true,
        correlation_id: None,
    }
}

#[tokio::test]
async fn repository_sequences_survive_reopen_and_duplicate_delivery_is_idempotent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("agent.sqlite");
    let root = AgentId::new();
    let delivery = uuid::Uuid::new_v4();
    {
        let repository = SqliteAgentRunRepository::open(&path).unwrap();
        repository.create(&run(root)).await.unwrap();
        let first = repository
            .append_message(root, root, delivery, &payload("first"), 10)
            .await
            .unwrap();
        assert_eq!(first.sequence, 1);
        assert_eq!(first.delivery, AgentMessageDeliveryState::Pending);
        let settled = repository
            .mark_message_delivery(root, delivery, AgentMessageDeliveryState::Delivered)
            .await
            .unwrap();
        assert_eq!(settled.delivery, AgentMessageDeliveryState::Delivered);
    }
    let repository = SqliteAgentRunRepository::open(&path).unwrap();
    let replay = repository
        .append_message(root, root, delivery, &payload("ignored retry body"), 11)
        .await
        .unwrap();
    assert_eq!(replay.sequence, 1);
    assert_eq!(replay.payload.content, "first");
    assert_eq!(replay.delivery, AgentMessageDeliveryState::Delivered);
    let second = repository
        .append_message(root, root, uuid::Uuid::new_v4(), &payload("second"), 12)
        .await
        .unwrap();
    assert_eq!(second.sequence, 2);
    assert!(second.payload_ref.starts_with("sha256:"));
}

#[tokio::test]
async fn repository_rejects_invalid_schema_oversize_and_conflicting_settlement() {
    let repository = SqliteAgentRunRepository::in_memory().unwrap();
    let root = AgentId::new();
    repository.create(&run(root)).await.unwrap();
    let delivery = uuid::Uuid::new_v4();
    let mut invalid = payload("message");
    invalid.schema_version = 99;
    assert!(repository
        .append_message(root, root, delivery, &invalid, 10)
        .await
        .is_err());
    let oversized = payload(&"x".repeat(fabric::agent_control::MAX_AGENT_MESSAGE_BYTES + 1));
    assert!(repository
        .append_message(root, root, delivery, &oversized, 10)
        .await
        .is_err());
    repository
        .append_message(root, root, delivery, &payload("valid"), 10)
        .await
        .unwrap();
    repository
        .mark_message_delivery(root, delivery, AgentMessageDeliveryState::Rejected)
        .await
        .unwrap();
    assert!(repository
        .mark_message_delivery(root, delivery, AgentMessageDeliveryState::Delivered)
        .await
        .is_err());
}

#[test]
fn every_message_kind_has_a_versioned_bounded_payload() {
    for kind in [
        AgentMessageKind::Input,
        AgentMessageKind::Progress,
        AgentMessageKind::Result,
        AgentMessageKind::Signal,
        AgentMessageKind::Request,
        AgentMessageKind::Response,
    ] {
        AgentMessagePayload {
            schema_version: AGENT_MESSAGE_SCHEMA_V1,
            kind,
            content: "bounded".into(),
            start_turn: false,
            correlation_id: None,
        }
        .validate()
        .unwrap();
    }
}

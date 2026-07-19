use std::sync::Arc;

use async_trait::async_trait;
use fabric::{
    AgentBroadcastRef, AgentBudget, AgentContextFork, AgentControlError, AgentControlMessage,
    AgentControlPort, AgentHandle, AgentId, AgentListRequest, AgentProfileId, AgentRunStatus,
    AgentSendRequest, AgentSnapshot, AgentSpawnRequest, AgentWaitRequest, AgoraSpaceId,
    BroadcastEpoch, ContentId, OperationId, ProcessId, RuntimeId,
};
use uuid::Uuid;

fn budget() -> AgentBudget {
    AgentBudget {
        max_input_tokens: 1_000,
        max_output_tokens: 500,
        max_tool_calls: 10,
        max_elapsed_ms: 60_000,
        max_cost_usd: Some(1.0),
        max_depth: 2,
    }
}

fn spawn_request() -> AgentSpawnRequest {
    AgentSpawnRequest {
        root_agent_id: AgentId::new(),
        parent_agent_id: None,
        parent_process_id: None,
        profile_id: AgentProfileId("reviewer".into()),
        runtime_id: RuntimeId("native-cognit".into()),
        trusted_workspace: None,
        task: "review the implementation".into(),
        context: AgentContextFork::SelectedProjection {
            items: vec!["goal: preserve behavior".into()],
        },
        broadcast_refs: vec![],
        allowed_tools: vec!["file_read".into()],
        background_decls: vec![],
        budget: budget(),
    }
}

fn handle(root: AgentId) -> AgentHandle {
    AgentHandle {
        agent_id: AgentId::new(),
        root_agent_id: root,
        parent_agent_id: None,
        process_id: ProcessId(Uuid::new_v4()),
        operation_id: OperationId(Uuid::new_v4()),
        runtime_id: RuntimeId("native-cognit".into()),
        profile_id: AgentProfileId("reviewer".into()),
    }
}

fn snapshot(handle: AgentHandle) -> AgentSnapshot {
    AgentSnapshot {
        handle,
        status: AgentRunStatus::Running,
        result: None,
        created_at_ms: 1,
        started_at_ms: Some(2),
        ended_at_ms: None,
        last_error: None,
    }
}

#[test]
fn request_validation_enforces_all_bounds() {
    spawn_request().validate().unwrap();
    let mut invalid = spawn_request();
    invalid.task = "x".repeat(fabric::agent_control::MAX_AGENT_TASK_BYTES + 1);
    assert!(invalid.validate().is_err());

    let mut invalid = spawn_request();
    invalid.context = AgentContextFork::LastTurns { count: 0 };
    assert!(invalid.validate().is_err());

    let mut invalid = spawn_request();
    invalid.budget.max_tool_calls = 0;
    assert!(invalid.validate().is_err());

    let receipt = AgentBroadcastRef {
        space: AgoraSpaceId("root:workspace".into()),
        epoch: BroadcastEpoch(1),
        content_id: ContentId::new(),
    };
    let mut invalid = spawn_request();
    invalid.broadcast_refs = vec![receipt.clone(), receipt];
    assert!(invalid.validate().is_err());

    let root = AgentId::new();
    assert!(AgentWaitRequest {
        caller_root_agent_id: root,
        agent_id: AgentId::new(),
        timeout_ms: 0,
    }
    .validate()
    .is_err());
    assert!(AgentSendRequest {
        caller_root_agent_id: root,
        sender_agent_id: None,
        agent_id: AgentId::new(),
        kind: fabric::AgentMessageKind::Input,
        delivery_id: None,
        correlation_id: None,
        deadline_mono_ms: None,
        message: String::new(),
        start_turn: false,
    }
    .validate()
    .is_err());
    assert!(AgentListRequest {
        caller_root_agent_id: root,
        status: None,
        limit: fabric::agent_control::MAX_LIST_ITEMS + 1,
    }
    .validate()
    .is_err());
}

#[test]
fn serialization_uses_stable_status_and_context_tags() {
    let context = serde_json::to_value(AgentContextFork::LastTurns { count: 3 }).unwrap();
    assert_eq!(context["mode"], "last_turns");
    assert_eq!(
        serde_json::to_value(AgentRunStatus::Interrupted).unwrap(),
        "interrupted"
    );
    let request = spawn_request();
    let restored: AgentSpawnRequest =
        serde_json::from_str(&serde_json::to_string(&request).unwrap()).unwrap();
    assert_eq!(restored, request);

    let mut legacy = serde_json::to_value(spawn_request()).unwrap();
    legacy.as_object_mut().unwrap().remove("broadcast_refs");
    let restored: AgentSpawnRequest = serde_json::from_value(legacy).unwrap();
    assert!(restored.broadcast_refs.is_empty());
}

struct MockPort;

#[async_trait]
impl AgentControlPort for MockPort {
    async fn spawn(&self, request: AgentSpawnRequest) -> Result<AgentHandle, AgentControlError> {
        request.validate()?;
        Ok(handle(request.root_agent_id))
    }

    async fn wait(&self, request: AgentWaitRequest) -> Result<AgentSnapshot, AgentControlError> {
        request.validate()?;
        Ok(snapshot(handle(request.caller_root_agent_id)))
    }

    async fn send(
        &self,
        request: AgentSendRequest,
    ) -> Result<AgentControlMessage, AgentControlError> {
        request.validate()?;
        Ok(AgentControlMessage {
            delivery_id: request.delivery_id.unwrap_or_else(uuid::Uuid::new_v4),
            sequence: 1,
            from: request
                .sender_agent_id
                .unwrap_or(request.caller_root_agent_id),
            to: request.agent_id,
            kind: request.kind,
            delivery: fabric::AgentMessageDeliveryState::Delivered,
            content: request.message,
        })
    }

    async fn cancel(&self, root: AgentId, _: AgentId) -> Result<AgentSnapshot, AgentControlError> {
        Ok(snapshot(handle(root)))
    }

    async fn inspect(&self, root: AgentId, _: AgentId) -> Result<AgentSnapshot, AgentControlError> {
        Ok(snapshot(handle(root)))
    }

    async fn list(
        &self,
        request: AgentListRequest,
    ) -> Result<Vec<AgentSnapshot>, AgentControlError> {
        request.validate()?;
        Ok(vec![snapshot(handle(request.caller_root_agent_id))])
    }
}

#[tokio::test]
async fn port_is_object_safe_and_uses_typed_results() {
    let port: Arc<dyn AgentControlPort> = Arc::new(MockPort);
    let request = spawn_request();
    let root = request.root_agent_id;
    let child = port.spawn(request).await.unwrap();
    assert_eq!(child.root_agent_id, root);
    let rows = port
        .list(AgentListRequest {
            caller_root_agent_id: root,
            status: Some(AgentRunStatus::Running),
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
}

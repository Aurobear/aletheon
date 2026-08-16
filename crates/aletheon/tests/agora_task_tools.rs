use agora::AgoraService;
use std::sync::Arc;

use ::contracts::cognitive_workflow::{
    AgoraProjectionRequest, CognitiveRole, CognitiveTaskNodeId, CognitiveTaskStatus,
};
use ::contracts::{AgoraSpaceId, ProcessId};
use corpus::tools::agora_task_tools::AgoraTaskTools;
use corpus::{Tool, ToolContext};
use serde_json::{json, Value};

fn context(session: &str) -> ToolContext {
    ToolContext {
        agent: None,
        approval_authority: None,
        working_dir: std::env::current_dir().unwrap(),
        session_id: session.into(),
        clock: Arc::new(kernel::chronos::TestClock::default()),
        turn_event_sender: None,
    }
}

fn tools_and_service() -> (Vec<Arc<dyn Tool>>, Arc<dyn AgoraService>) {
    let service: Arc<dyn AgoraService> = Arc::new(agora::AgoraRegistry::new(Arc::new(
        kernel::chronos::TestClock::default(),
    )));
    let tools = AgoraTaskTools::new(service.clone(), ProcessId::new()).tools();
    (tools, service)
}

#[tokio::test]
async fn model_tools_and_host_gate_observe_same_task_version() {
    let (tools, service) = tools_and_service();
    let create = tools
        .iter()
        .find(|tool| tool.name() == "task_create")
        .unwrap();
    let created = create
        .execute(
            json!({
                "id": "implementation",
                "subject": "Implement",
                "description": "versioned cognitive workflow",
                "role": "executor",
                "acceptance_criteria": ["tests pass"]
            }),
            &context("root-turn-a"),
        )
        .await;
    assert!(!created.is_error, "{}", created.content);
    let created: Value = serde_json::from_str(&created.content).unwrap();
    assert_eq!(created["workspace_version"], 1);

    let host_projection = service
        .project_task(AgoraProjectionRequest {
            space: AgoraSpaceId("root-turn-a".into()),
            task_node_id: CognitiveTaskNodeId("implementation".into()),
            role: CognitiveRole::Root,
            max_artifacts: 0,
            include_kinds: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(host_projection.workspace_version, 1);
    assert_eq!(host_projection.task.status, CognitiveTaskStatus::Pending);

    let update = tools
        .iter()
        .find(|tool| tool.name() == "task_update")
        .unwrap();
    let updated = update
        .execute(
            json!({
                "id": "implementation",
                "status": "in_progress",
                "expected_version": host_projection.workspace_version
            }),
            &context("root-turn-a"),
        )
        .await;
    assert!(!updated.is_error, "{}", updated.content);
    let list = service
        .list_tasks(AgoraSpaceId("root-turn-a".into()))
        .await
        .unwrap();
    assert_eq!(list.workspace_version, 2);
    assert_eq!(list.tasks[0].status, CognitiveTaskStatus::Running);
}

#[tokio::test]
async fn task_update_rejects_stale_model_version() {
    let (tools, _service) = tools_and_service();
    tools
        .iter()
        .find(|tool| tool.name() == "task_create")
        .unwrap()
        .execute(
            json!({"id": "task", "subject": "S", "description": "D"}),
            &context("root-turn-b"),
        )
        .await;
    let stale = tools
        .iter()
        .find(|tool| tool.name() == "task_update")
        .unwrap()
        .execute(
            json!({"id": "task", "status": "completed", "expected_version": 0}),
            &context("root-turn-b"),
        )
        .await;
    assert!(stale.is_error);
    assert!(stale.content.contains("version conflict"));
}

#[tokio::test]
async fn request_user_input_blocks_exact_task_with_checkpoint() {
    let (tools, service) = tools_and_service();
    tools
        .iter()
        .find(|tool| tool.name() == "task_create")
        .unwrap()
        .execute(
            json!({"id": "ambiguous", "subject": "Choose", "description": "material behavior"}),
            &context("root-turn-c"),
        )
        .await;
    let result = tools
        .iter()
        .find(|tool| tool.name() == "request_user_input")
        .unwrap()
        .execute(
            json!({
                "task_id": "ambiguous",
                "question": "Which behavior is authoritative?",
                "choices": ["spec", "code"],
                "expected_version": 1,
                "outstanding_obligations": ["resolve conflict"]
            }),
            &context("root-turn-c"),
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    let projection = service
        .project_task(AgoraProjectionRequest {
            space: AgoraSpaceId("root-turn-c".into()),
            task_node_id: CognitiveTaskNodeId("ambiguous".into()),
            role: CognitiveRole::Root,
            max_artifacts: 8,
            include_kinds: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(projection.task.status, CognitiveTaskStatus::Blocked);
    let clarification = projection.clarification.unwrap();
    assert_eq!(clarification.checkpoint.workspace_version, 1);
    assert_eq!(
        clarification.checkpoint.outstanding_obligations,
        vec!["resolve conflict"]
    );
}

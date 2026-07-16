use std::sync::Arc;

use executive::service::agent_control::{
    AgentRunRecord, AgentRunRepository, SqliteAgentRunRepository,
};
use fabric::{
    AgentBudget, AgentContextFork, AgentHandle, AgentId, AgentProfileId, AgentResult,
    AgentRunStatus, AgentSnapshot, AgentSpawnRequest, AttemptUsage, OperationId, ProcessId,
    RuntimeId,
};

fn request(
    root: AgentId,
    parent: Option<AgentId>,
    parent_process: Option<ProcessId>,
) -> AgentSpawnRequest {
    AgentSpawnRequest {
        root_agent_id: root,
        parent_agent_id: parent,
        parent_process_id: parent_process,
        profile_id: AgentProfileId("worker".into()),
        runtime_id: RuntimeId("test".into()),
        task: "inspect the repository".into(),
        context: AgentContextFork::None,
        allowed_tools: vec!["file_read".into()],
        budget: AgentBudget {
            max_input_tokens: 100,
            max_output_tokens: 100,
            max_tool_calls: 2,
            max_elapsed_ms: 5_000,
            max_cost_usd: Some(1.0),
            max_depth: 2,
        },
    }
}

fn record(root: AgentId, parent: Option<AgentId>, created_at_ms: i64) -> AgentRunRecord {
    let agent = if parent.is_some() {
        AgentId::new()
    } else {
        root
    };
    let request = request(root, parent, None);
    AgentRunRecord {
        snapshot: AgentSnapshot {
            handle: AgentHandle {
                agent_id: agent,
                root_agent_id: root,
                parent_agent_id: parent,
                process_id: ProcessId::new(),
                operation_id: OperationId::new(),
                runtime_id: request.runtime_id.clone(),
                profile_id: request.profile_id.clone(),
            },
            status: AgentRunStatus::Queued,
            result: None,
            created_at_ms,
            started_at_ms: None,
            ended_at_ms: None,
            last_error: None,
        },
        request_hash: SqliteAgentRunRepository::request_hash(&request).unwrap(),
        request,
        version: 0,
        retain_until_ms: created_at_ms + 60_000,
    }
}

#[tokio::test]
async fn repository_reopens_and_preserves_parent_edge() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("agents.sqlite");
    let root = AgentId::new();
    let parent = AgentId::new();
    let run = record(root, Some(parent), 10);
    let agent = run.agent_id();

    SqliteAgentRunRepository::open(&path)
        .unwrap()
        .create(&run)
        .await
        .unwrap();
    let reopened = SqliteAgentRunRepository::open(&path).unwrap();
    let stored = reopened.get(agent).await.unwrap().unwrap();
    assert_eq!(stored, run);
    assert_eq!(stored.snapshot.handle.parent_agent_id, Some(parent));
}

#[tokio::test]
async fn duplicate_identity_and_request_hash_mismatch_are_conflicts() {
    let repository = SqliteAgentRunRepository::in_memory().unwrap();
    let root = AgentId::new();
    let run = record(root, None, 10);
    repository.create(&run).await.unwrap();
    let duplicate = repository.create(&run).await.unwrap_err();
    assert_eq!(duplicate.kind, fabric::AgentControlErrorKind::Conflict);

    let mut mismatched = record(AgentId::new(), None, 11);
    mismatched.request.task = "changed after hashing".into();
    let mismatch = repository.create(&mismatched).await.unwrap_err();
    assert_eq!(mismatch.kind, fabric::AgentControlErrorKind::Conflict);
}

#[tokio::test]
async fn transitions_are_compare_and_swap_and_terminal_results_survive() {
    let repository = SqliteAgentRunRepository::in_memory().unwrap();
    let root = AgentId::new();
    let run = record(root, None, 10);
    let agent = run.agent_id();
    repository.create(&run).await.unwrap();
    let running = repository
        .transition(
            agent,
            AgentRunStatus::Queued,
            AgentRunStatus::Running,
            None,
            None,
            20,
        )
        .await
        .unwrap();
    assert_eq!(running.version, 1);
    assert_eq!(running.snapshot.started_at_ms, Some(20));
    let conflict = repository
        .transition(
            agent,
            AgentRunStatus::Queued,
            AgentRunStatus::Running,
            None,
            None,
            21,
        )
        .await
        .unwrap_err();
    assert_eq!(conflict.kind, fabric::AgentControlErrorKind::Conflict);

    let result = AgentResult {
        output: "done".into(),
        usage: AttemptUsage::default(),
        evidence: vec![],
        artifacts: vec![],
    };
    repository
        .transition(
            agent,
            AgentRunStatus::Running,
            AgentRunStatus::Succeeded,
            Some(result.clone()),
            None,
            30,
        )
        .await
        .unwrap();
    assert_eq!(
        repository
            .get(agent)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .result,
        Some(result)
    );
}

#[tokio::test]
async fn root_list_is_bounded_filtered_and_deterministic() {
    let repository: Arc<dyn AgentRunRepository> =
        Arc::new(SqliteAgentRunRepository::in_memory().unwrap());
    let root = AgentId::new();
    let first = record(root, Some(root), 10);
    let second = record(root, Some(root), 20);
    repository.create(&first).await.unwrap();
    repository.create(&second).await.unwrap();

    let rows = repository
        .list_root(root, Some(AgentRunStatus::Queued), 1)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].agent_id(), second.agent_id());
    assert!(repository.list_root(root, None, 0).await.is_err());
}

#[tokio::test]
async fn message_reference_is_persisted_before_sequence_is_returned() {
    let repository = SqliteAgentRunRepository::in_memory().unwrap();
    let root = AgentId::new();
    let run = record(root, None, 10);
    repository.create(&run).await.unwrap();

    let first = repository
        .append_message(root, root, "one", 11)
        .await
        .unwrap();
    let second = repository
        .append_message(root, root, "two", 12)
        .await
        .unwrap();
    assert_eq!((first.sequence, second.sequence), (1, 2));
    assert_ne!(first.content_hash, second.content_hash);
}

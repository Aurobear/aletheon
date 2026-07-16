use std::sync::Arc;

use executive::service::agent_control::{
    AgentRunRecord, AgentRunRepository, SqliteAgentRunRepository,
};
use fabric::{
    AgentBroadcastRef, AgentBudget, AgentContextFork, AgentHandle, AgentId, AgentProfileId,
    AgentResult, AgentRunStatus, AgentSnapshot, AgentSpawnRequest, AgoraSpaceId, AttemptUsage,
    BroadcastEpoch, ContentId, OperationId, ProcessId, RuntimeId,
};
use rusqlite::{params, Connection};

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
        broadcast_refs: vec![],
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
    let process_id = ProcessId::new();
    AgentRunRecord {
        snapshot: AgentSnapshot {
            handle: AgentHandle {
                agent_id: agent,
                root_agent_id: root,
                parent_agent_id: parent,
                process_id,
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
        workspace_id: executive::service::agent_control::agent_workspace_id(agent),
        root_process_id: process_id,
        broadcast_refs: request.broadcast_refs.clone(),
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
    let mut run = record(root, Some(parent), 10);
    run.request.broadcast_refs = vec![AgentBroadcastRef {
        space: AgoraSpaceId("parent:workspace".into()),
        epoch: BroadcastEpoch(3),
        content_id: ContentId::new(),
    }];
    run.broadcast_refs = run.request.broadcast_refs.clone();
    run.request_hash = SqliteAgentRunRepository::request_hash(&run.request).unwrap();
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
async fn repository_migrates_pre_workspace_rows_without_losing_runs() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy-agents.sqlite");
    let root = AgentId::new();
    let run = record(root, None, 10);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE agent_runs (
                agent_id TEXT PRIMARY KEY,
                root_agent_id TEXT NOT NULL,
                parent_agent_id TEXT,
                process_id TEXT NOT NULL UNIQUE,
                operation_id TEXT NOT NULL UNIQUE,
                runtime_id TEXT NOT NULL,
                profile_id TEXT NOT NULL,
                status TEXT NOT NULL,
                request_json TEXT NOT NULL,
                request_hash TEXT NOT NULL,
                result_json TEXT,
                created_at_ms INTEGER NOT NULL,
                started_at_ms INTEGER,
                ended_at_ms INTEGER,
                last_error TEXT,
                version INTEGER NOT NULL DEFAULT 0,
                retain_until_ms INTEGER NOT NULL
            );",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO agent_runs VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, 'queued', ?7, ?8, NULL, ?9, NULL, NULL, NULL, 0, ?10)",
            params![
                run.agent_id().0.to_string(),
                root.0.to_string(),
                run.snapshot.handle.process_id.0.to_string(),
                run.snapshot.handle.operation_id.0.to_string(),
                run.snapshot.handle.runtime_id.0,
                run.snapshot.handle.profile_id.0,
                serde_json::to_string(&run.request).unwrap(),
                run.request_hash,
                run.snapshot.created_at_ms,
                run.retain_until_ms,
            ],
        )
        .unwrap();
    drop(connection);

    let repository = SqliteAgentRunRepository::open(&path).unwrap();
    let migrated = repository.get(root).await.unwrap().unwrap();
    assert_eq!(
        migrated.workspace_id,
        executive::service::agent_control::agent_workspace_id(root)
    );
    assert_eq!(
        migrated.root_process_id,
        migrated.snapshot.handle.process_id
    );
    assert!(migrated.broadcast_refs.is_empty());
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

    let payload = |content: &str| fabric::AgentMessagePayload {
        schema_version: fabric::AGENT_MESSAGE_SCHEMA_V1,
        kind: fabric::AgentMessageKind::Input,
        content: content.into(),
        start_turn: false,
        correlation_id: None,
        deadline_mono_ms: None,
    };

    let first = repository
        .append_message(root, root, uuid::Uuid::new_v4(), &payload("one"), 11)
        .await
        .unwrap();
    let second = repository
        .append_message(root, root, uuid::Uuid::new_v4(), &payload("two"), 12)
        .await
        .unwrap();
    assert_eq!((first.sequence, second.sequence), (1, 2));
    assert_ne!(first.payload_ref, second.payload_ref);
}

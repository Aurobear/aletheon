use std::sync::Arc;

use executive::r#impl::runtime::worktree_recovery::AgentWorktreeReclaimer;
use executive::service::agent_control::{
    AgentCleanupCoordinator, AgentResourceLease, AgentResourceLeaseKind, AgentRunRecord,
    AgentRunRepository, SqliteAgentRunRepository,
};
use fabric::{
    AgentBudget, AgentContextFork, AgentHandle, AgentId, AgentProfileId, AgentRunStatus,
    AgentSnapshot, AgentSpawnRequest, AgoraSpaceId, OperationId, ProcessId, RuntimeId,
    RuntimeResumability,
};
use parking_lot::Mutex;

#[derive(Default)]
struct RecordingWorktrees {
    calls: Mutex<Vec<String>>,
}

impl AgentWorktreeReclaimer for RecordingWorktrees {
    fn reclaim(&self, lease: &AgentResourceLease) -> anyhow::Result<()> {
        self.calls.lock().push(lease.lease_key.clone());
        if lease.lease_key.contains("dirty") || lease.lease_key.contains("partial") {
            anyhow::bail!("unsafe worktree retained")
        }
        Ok(())
    }
}

fn run(status: AgentRunStatus, retain_until_ms: i64) -> AgentRunRecord {
    let agent = AgentId::new();
    let process = ProcessId::new();
    let request = AgentSpawnRequest {
        root_agent_id: agent,
        parent_agent_id: None,
        parent_process_id: None,
        profile_id: AgentProfileId("cleanup".into()),
        runtime_id: RuntimeId("native-cognit".into()),
        task: "cleanup fixture".into(),
        context: AgentContextFork::None,
        broadcast_refs: vec![],
        allowed_tools: vec![],
        budget: AgentBudget {
            max_input_tokens: 1,
            max_output_tokens: 1,
            max_tool_calls: 1,
            max_elapsed_ms: 1,
            max_cost_usd: None,
            max_depth: 1,
        },
    };
    AgentRunRecord {
        snapshot: AgentSnapshot {
            handle: AgentHandle {
                agent_id: agent,
                root_agent_id: agent,
                parent_agent_id: None,
                process_id: process,
                operation_id: OperationId::new(),
                runtime_id: request.runtime_id.clone(),
                profile_id: request.profile_id.clone(),
            },
            status,
            result: None,
            created_at_ms: 1,
            started_at_ms: (status != AgentRunStatus::Queued).then_some(2),
            ended_at_ms: status.is_terminal().then_some(3),
            last_error: None,
        },
        request_hash: SqliteAgentRunRepository::request_hash(&request).unwrap(),
        request,
        workspace_id: AgoraSpaceId(format!("agent:{}", agent.0)),
        root_process_id: process,
        broadcast_refs: vec![],
        version: 0,
        retain_until_ms,
        resumability: RuntimeResumability::Never,
        recovery: None,
    }
}

async fn persist(repository: &SqliteAgentRunRepository, desired: &AgentRunRecord) {
    let mut queued = desired.clone();
    queued.snapshot.status = AgentRunStatus::Queued;
    queued.snapshot.started_at_ms = None;
    queued.snapshot.ended_at_ms = None;
    repository.create(&queued).await.unwrap();
    if desired.status() != AgentRunStatus::Queued {
        repository
            .transition(
                desired.agent_id(),
                AgentRunStatus::Queued,
                desired.status(),
                None,
                None,
                3,
            )
            .await
            .unwrap();
    }
}

fn lease(run: &AgentRunRecord, key: &str, kind: AgentResourceLeaseKind) -> AgentResourceLease {
    AgentResourceLease {
        lease_key: key.into(),
        agent_id: run.agent_id(),
        kind,
        owner: "daemon:old".into(),
        expires_at_ms: 10,
        worktree_root: (kind == AgentResourceLeaseKind::Worktree).then(|| "/safe/root".into()),
        worktree_path: (kind == AgentResourceLeaseKind::Worktree)
            .then(|| format!("/safe/root/{key}")),
        expected_head: (kind == AgentResourceLeaseKind::Worktree).then(|| "abc123".into()),
    }
}

#[tokio::test]
async fn only_expired_verified_terminal_resources_are_reclaimed_idempotently() {
    let repository = Arc::new(SqliteAgentRunRepository::in_memory().unwrap());
    let terminal = run(AgentRunStatus::Failed, 1_000);
    let running = run(AgentRunStatus::Running, 1_000);
    let dirty = run(AgentRunStatus::Interrupted, 1_000);
    persist(&repository, &terminal).await;
    persist(&repository, &running).await;
    persist(&repository, &dirty).await;
    repository
        .put_resource_lease(&lease(
            &terminal,
            "admission:terminal",
            AgentResourceLeaseKind::Admission,
        ))
        .await
        .unwrap();
    repository
        .put_resource_lease(&lease(
            &terminal,
            "worktree:verified",
            AgentResourceLeaseKind::Worktree,
        ))
        .await
        .unwrap();
    repository
        .put_resource_lease(&lease(
            &running,
            "execution:live",
            AgentResourceLeaseKind::Execution,
        ))
        .await
        .unwrap();
    repository
        .put_resource_lease(&lease(
            &dirty,
            "worktree:dirty",
            AgentResourceLeaseKind::Worktree,
        ))
        .await
        .unwrap();
    let worktrees = Arc::new(RecordingWorktrees::default());
    let cleanup = AgentCleanupCoordinator::new(repository.clone(), worktrees.clone());

    let first = cleanup.reclaim_expired(20).await.unwrap();
    assert_eq!(first.reclaimed, 2);
    assert_eq!(first.retained_unsafe, 1);
    assert_eq!(first.failures, 1);
    assert_eq!(worktrees.calls.lock().len(), 2);
    let repeated = cleanup.reclaim_expired(20).await.unwrap();
    assert_eq!(repeated.reclaimed, 0);
    assert!(repeated.retained_unsafe >= 1);
}

#[tokio::test]
async fn forged_lease_owner_conflict_partial_failure_and_retention_gate_fail_closed() {
    let repository = Arc::new(SqliteAgentRunRepository::in_memory().unwrap());
    let retained = run(AgentRunStatus::Failed, 100);
    let compactable = run(AgentRunStatus::Interrupted, 5);
    persist(&repository, &retained).await;
    persist(&repository, &compactable).await;
    let partial = lease(
        &retained,
        "worktree:partial",
        AgentResourceLeaseKind::Worktree,
    );
    repository.put_resource_lease(&partial).await.unwrap();
    let mut forged = partial.clone();
    forged.owner = "attacker".into();
    assert!(repository.put_resource_lease(&forged).await.is_err());
    assert!(!repository
        .delete_resource_lease(&partial.lease_key, "attacker")
        .await
        .unwrap());

    let cleanup =
        AgentCleanupCoordinator::new(repository.clone(), Arc::new(RecordingWorktrees::default()));
    let report = cleanup.reclaim_expired(20).await.unwrap();
    assert_eq!(report.failures, 1);
    assert_eq!(report.compacted_rows, 1);
    assert!(repository.get(retained.agent_id()).await.unwrap().is_some());
    assert!(repository
        .get(compactable.agent_id())
        .await
        .unwrap()
        .is_none());
}

use std::sync::Arc;

use aletheon_kernel::chronos::TestClock;
use aletheon_kernel::KernelRuntime;
use executive::service::agent_control::{
    AgentControlService, AgentRecoveryCoordinator, AgentRecoveryObservation, AgentRunRecord,
    AgentRunRepository, AgentRuntimeRegistry, BoundedAgentAdmission, SqliteAgentRunRepository,
};
use fabric::{
    AgentBudget, AgentContextFork, AgentHandle, AgentId, AgentProfileId, AgentRecoveryDecision,
    AgentRecoveryReceipt, AgentRunStatus, AgentSnapshot, AgentSpawnRequest, AgoraSpaceId,
    OperationId, ProcessId, RuntimeId, RuntimeResumability,
};
use tempfile::tempdir;

fn record(status: AgentRunStatus, resumability: RuntimeResumability) -> AgentRunRecord {
    let agent = AgentId::new();
    let process = ProcessId::new();
    let request = AgentSpawnRequest {
        root_agent_id: agent,
        parent_agent_id: None,
        parent_process_id: None,
        profile_id: AgentProfileId("recovery-worker".into()),
        runtime_id: RuntimeId("native-cognit".into()),
        trusted_workspace: None,
        task: "recover without replay".into(),
        context: AgentContextFork::None,
        broadcast_refs: vec![],
        allowed_tools: vec![],
        background_decls: vec![],
        budget: AgentBudget {
            max_input_tokens: 100,
            max_output_tokens: 100,
            max_tool_calls: 1,
            max_elapsed_ms: 1_000,
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
            created_at_ms: 10,
            started_at_ms: (status != AgentRunStatus::Queued).then_some(11),
            ended_at_ms: None,
            last_error: None,
        },
        request_hash: SqliteAgentRunRepository::request_hash(&request).unwrap(),
        request,
        workspace_id: AgoraSpaceId(format!("agent:{}", agent.0)),
        root_process_id: process,
        broadcast_refs: vec![],
        version: if status == AgentRunStatus::Queued {
            0
        } else {
            1
        },
        retain_until_ms: 100_000,
        resumability,
        recovery: None,
    }
}

async fn persist(repository: &SqliteAgentRunRepository, run: &AgentRunRecord) {
    let mut queued = run.clone();
    queued.snapshot.status = AgentRunStatus::Queued;
    queued.snapshot.started_at_ms = None;
    queued.version = 0;
    repository.create(&queued).await.unwrap();
    if run.status() != AgentRunStatus::Queued {
        repository
            .transition(
                run.agent_id(),
                AgentRunStatus::Queued,
                run.status(),
                None,
                None,
                11,
            )
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn decision_interrupts_ambiguous_native_work_and_finalizes_kernel_terminal_work() {
    let repository = Arc::new(SqliteAgentRunRepository::in_memory().unwrap());
    let queued = record(AgentRunStatus::Queued, RuntimeResumability::Never);
    let provider = record(AgentRunStatus::Running, RuntimeResumability::Never);
    let terminal = record(AgentRunStatus::Running, RuntimeResumability::Never);
    persist(&repository, &queued).await;
    persist(&repository, &provider).await;
    persist(&repository, &terminal).await;
    let coordinator = AgentRecoveryCoordinator::new(repository.clone(), "daemon:2", 20).unwrap();

    assert_eq!(
        coordinator
            .recover_one(
                &queued,
                AgentRecoveryObservation {
                    process_live: false,
                    operation_terminal: None,
                    checkpoint_available: false,
                },
            )
            .await
            .unwrap(),
        AgentRecoveryDecision::Interrupt
    );
    coordinator
        .recover_one(
            &provider,
            AgentRecoveryObservation {
                process_live: true,
                operation_terminal: None,
                checkpoint_available: false,
            },
        )
        .await
        .unwrap();
    coordinator
        .recover_one(
            &terminal,
            AgentRecoveryObservation {
                process_live: false,
                operation_terminal: Some(AgentRunStatus::Failed),
                checkpoint_available: false,
            },
        )
        .await
        .unwrap();

    assert_eq!(
        repository
            .get(queued.agent_id())
            .await
            .unwrap()
            .unwrap()
            .status(),
        AgentRunStatus::Interrupted
    );
    assert_eq!(
        repository
            .get(provider.agent_id())
            .await
            .unwrap()
            .unwrap()
            .status(),
        AgentRunStatus::Interrupted
    );
    assert_eq!(
        repository
            .get(terminal.agent_id())
            .await
            .unwrap()
            .unwrap()
            .status(),
        AgentRunStatus::Failed
    );
}

#[tokio::test]
async fn checkpoint_resume_preserves_identity_and_pre_action_crash_replays_only_the_decision() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("agent-runs.db");
    let repository = Arc::new(SqliteAgentRunRepository::open(&path).unwrap());
    let resumable = record(
        AgentRunStatus::Running,
        RuntimeResumability::Checkpointed {
            reference: "checkpoint:sha256:abc".into(),
        },
    );
    persist(&repository, &resumable).await;
    let coordinator = AgentRecoveryCoordinator::new(repository.clone(), "daemon:3", 30).unwrap();
    let decision = coordinator
        .recover_one(
            &resumable,
            AgentRecoveryObservation {
                process_live: true,
                operation_terminal: None,
                checkpoint_available: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(decision, AgentRecoveryDecision::Resume);
    drop(coordinator);
    drop(repository);
    let reopened = Arc::new(SqliteAgentRunRepository::open(&path).unwrap());
    let stored = reopened.get(resumable.agent_id()).await.unwrap().unwrap();
    assert_eq!(
        stored.snapshot.handle.agent_id,
        resumable.snapshot.handle.agent_id
    );
    assert_eq!(stored.resumability, resumable.resumability);
    assert_eq!(
        stored.recovery.as_ref().unwrap().decision,
        AgentRecoveryDecision::Resume
    );

    let interrupted = record(AgentRunStatus::Running, RuntimeResumability::Never);
    persist(&reopened, &interrupted).await;
    reopened
        .record_recovery(
            interrupted.agent_id(),
            &AgentRecoveryReceipt {
                decision: AgentRecoveryDecision::Interrupt,
                daemon_generation: "daemon:crashed-before-action".into(),
                recovered_at_ms: 40,
                idempotency_key: "recovery:stable".into(),
            },
        )
        .await
        .unwrap();
    let stored = reopened.get(interrupted.agent_id()).await.unwrap().unwrap();
    AgentRecoveryCoordinator::new(reopened.clone(), "daemon:4", 50)
        .unwrap()
        .recover_one(
            &stored,
            AgentRecoveryObservation {
                process_live: false,
                operation_terminal: None,
                checkpoint_available: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        reopened
            .get(interrupted.agent_id())
            .await
            .unwrap()
            .unwrap()
            .status(),
        AgentRunStatus::Interrupted
    );
}

#[tokio::test]
async fn startup_reconciles_open_rows_before_admission_and_never_replays_native_work() {
    let clock = Arc::new(TestClock::new(1_000, 0));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let repository = Arc::new(SqliteAgentRunRepository::in_memory().unwrap());
    let run = record(AgentRunStatus::Queued, RuntimeResumability::Never);
    persist(&repository, &run).await;
    let service = AgentControlService::new(
        kernel,
        clock,
        repository.clone(),
        Arc::new(BoundedAgentAdmission::new(1).unwrap()),
        Arc::new(AgentRuntimeRegistry::default()),
    );

    let report = service
        .reconcile_startup("daemon:startup-test")
        .await
        .unwrap();
    assert!(report.ready());
    assert_eq!(report.open_rows, 1);
    assert_eq!(report.interrupted, 1);
    assert_eq!(
        repository
            .get(run.agent_id())
            .await
            .unwrap()
            .unwrap()
            .status(),
        AgentRunStatus::Interrupted
    );
}

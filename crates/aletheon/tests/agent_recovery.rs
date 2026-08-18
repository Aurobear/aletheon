use adapters_sqlite::runtime_agent::SqliteAgentRunProjection;
use aletheon::host::runtime::test_registry::AgentExecutionRegistry;
use std::sync::Arc;

use ::contracts::{
    AgentBudget, AgentContextFork, AgentHandle, AgentId, AgentProfileId, AgentRecoveryDecision,
    AgentRecoveryReceipt, AgentRunStatus, AgentSnapshot, AgentSpawnRequest, AgoraSpaceId,
    OperationId, ProcessId, RuntimeId, RuntimeResumability,
};
use aletheon::composition::agent_control::{
    AgentHostAdapter, AgentRecoveryCoordinator, AgentRecoveryObservation, AgentRunProjection,
    AgentRunRecord, BoundedAgentAdmission, RuntimeAgentRunProjection, RuntimeProcessReclaimOutcome,
    RuntimeProcessSupervisor,
};
use kernel::chronos::TestClock;
use kernel::KernelRuntime;
use tempfile::tempdir;

#[derive(Debug)]
struct ReclaimingSupervisor;

#[async_trait::async_trait]
impl RuntimeProcessSupervisor for ReclaimingSupervisor {
    async fn reclaim(
        &self,
        _identity: ::contracts::RuntimeProcessId,
    ) -> Result<RuntimeProcessReclaimOutcome, ::contracts::AgentControlError> {
        Ok(RuntimeProcessReclaimOutcome::Reclaimed)
    }
}

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
        delegator_authority: None,
        cognitive_binding: None,
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
        request_hash: SqliteAgentRunProjection::request_hash(&request).unwrap(),
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

async fn persist(repository: &SqliteAgentRunProjection, run: &AgentRunRecord) {
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
    let repository = Arc::new(SqliteAgentRunProjection::in_memory().unwrap());
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
    let repository = Arc::new(SqliteAgentRunProjection::open(&path).unwrap());
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
    let reopened = Arc::new(SqliteAgentRunProjection::open(&path).unwrap());
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
async fn runtime_projection_reconciles_replayed_orphan_before_sql_terminal_projection() {
    let clock = Arc::new(TestClock::new(1_500, 0));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let sql_projection = Arc::new(SqliteAgentRunProjection::in_memory().unwrap());
    let run = record(AgentRunStatus::Queued, RuntimeResumability::Never);
    persist(&sql_projection, &run).await;

    let supervisor = Arc::new(runtime::RuntimeAgentSupervisor::new(
        runtime::DelegateBackendRegistry::new(),
    ));
    supervisor
        .replay_from_stream([runtime::AgentStreamEvent::new(
            1,
            runtime::RuntimeEvent::AgentRunStarted {
                session: runtime::SessionId(run.root_agent_id().0.to_string()),
                agent_run: runtime::AgentRunId(run.agent_id().0.to_string()),
                generation: Some(runtime::Generation(4)),
                backend: Some(run.snapshot.handle.runtime_id.0.clone()),
            },
        )])
        .await
        .unwrap();

    let repository: Arc<dyn AgentRunProjection> = Arc::new(RuntimeAgentRunProjection::new(
        sql_projection.clone(),
        supervisor.clone(),
    ));
    let service = AgentHostAdapter::new_runtime_only(
        kernel,
        clock,
        repository,
        Arc::new(BoundedAgentAdmission::new(1).unwrap()),
        Arc::new(adapters_sqlite::event_spine::SqliteEventSpine::open(":memory:").unwrap()),
        supervisor.clone(),
    );

    let report = service
        .reconcile_startup("daemon:runtime-projection")
        .await
        .unwrap();
    assert!(report.ready());
    assert_eq!(report.interrupted, 1);
    assert_eq!(
        sql_projection
            .get(run.agent_id())
            .await
            .unwrap()
            .unwrap()
            .status(),
        AgentRunStatus::Interrupted
    );
    assert_eq!(
        supervisor
            .wait(&runtime::AgentRunId(run.agent_id().0.to_string()))
            .await
            .unwrap(),
        runtime::TurnTerminal::Interrupted
    );
}

#[tokio::test]
async fn runtime_reconcile_fences_started_run_when_host_projection_was_never_committed() {
    let clock = Arc::new(TestClock::new(1_600, 0));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let sql_projection = Arc::new(SqliteAgentRunProjection::in_memory().unwrap());
    let run = record(AgentRunStatus::Queued, RuntimeResumability::Never);
    let supervisor = Arc::new(runtime::RuntimeAgentSupervisor::new(
        runtime::DelegateBackendRegistry::new(),
    ));
    let agent_run = runtime::AgentRunId(run.agent_id().0.to_string());
    supervisor
        .replay_from_stream([runtime::AgentStreamEvent::new(
            1,
            runtime::RuntimeEvent::AgentRunStarted {
                session: runtime::SessionId(run.root_agent_id().0.to_string()),
                agent_run: agent_run.clone(),
                generation: Some(runtime::Generation(8)),
                backend: Some(run.snapshot.handle.runtime_id.0.clone()),
            },
        )])
        .await
        .unwrap();

    let repository: Arc<dyn AgentRunProjection> = Arc::new(RuntimeAgentRunProjection::new(
        sql_projection,
        supervisor.clone(),
    ));
    let service = AgentHostAdapter::new_runtime_only(
        kernel,
        clock,
        repository,
        Arc::new(BoundedAgentAdmission::new(1).unwrap()),
        Arc::new(adapters_sqlite::event_spine::SqliteEventSpine::open(":memory:").unwrap()),
        supervisor.clone(),
    );

    let report = service
        .reconcile_startup("daemon:missing-projection")
        .await
        .unwrap();
    assert!(report.ready());
    assert_eq!(report.interrupted, 1);
    assert_eq!(
        supervisor.wait(&agent_run).await.unwrap(),
        runtime::TurnTerminal::Interrupted
    );
    assert!(supervisor.open_lifecycle_runs().is_empty());
}

#[tokio::test]
async fn startup_reconciles_open_rows_before_admission_and_never_replays_native_work() {
    let clock = Arc::new(TestClock::new(1_000, 0));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let repository = Arc::new(SqliteAgentRunProjection::in_memory().unwrap());
    let run = record(AgentRunStatus::Queued, RuntimeResumability::Never);
    persist(&repository, &run).await;
    let service = AgentHostAdapter::new_fixture(
        kernel,
        clock,
        repository.clone(),
        Arc::new(BoundedAgentAdmission::new(1).unwrap()),
        Arc::new(AgentExecutionRegistry::default()),
        Arc::new(adapters_sqlite::event_spine::SqliteEventSpine::open(":memory:").unwrap()),
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

#[tokio::test]
async fn startup_recovery_visits_every_page_and_records_the_current_generation() {
    let clock = Arc::new(TestClock::new(2_000, 0));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let repository = Arc::new(SqliteAgentRunProjection::in_memory().unwrap());
    let mut agents = Vec::new();
    for _ in 0..1_001 {
        let run = record(AgentRunStatus::Queued, RuntimeResumability::Never);
        agents.push(run.agent_id());
        persist(&repository, &run).await;
    }
    let service = AgentHostAdapter::new_fixture(
        kernel,
        clock,
        repository.clone(),
        Arc::new(BoundedAgentAdmission::new(1).unwrap()),
        Arc::new(AgentExecutionRegistry::default()),
        Arc::new(adapters_sqlite::event_spine::SqliteEventSpine::open(":memory:").unwrap()),
    );

    let report = service.reconcile_startup("daemon:paged").await.unwrap();
    assert!(report.ready());
    assert_eq!(report.open_rows, 1_001);
    assert_eq!(report.interrupted, 1_001);
    for agent in agents {
        let stored = repository.get(agent).await.unwrap().unwrap();
        assert_eq!(stored.status(), AgentRunStatus::Interrupted);
        assert_eq!(stored.recovery.unwrap().daemon_generation, "daemon:paged");
    }
}

#[tokio::test]
async fn a_new_daemon_generation_makes_a_fresh_recovery_decision() {
    let repository = Arc::new(SqliteAgentRunProjection::in_memory().unwrap());
    let run = record(
        AgentRunStatus::Running,
        RuntimeResumability::Checkpointed {
            reference: "checkpoint:first".into(),
        },
    );
    persist(&repository, &run).await;
    let first = AgentRecoveryCoordinator::new(repository.clone(), "daemon:first", 30).unwrap();
    assert_eq!(
        first
            .recover_one(
                &run,
                AgentRecoveryObservation {
                    process_live: true,
                    operation_terminal: None,
                    checkpoint_available: true,
                },
            )
            .await
            .unwrap(),
        AgentRecoveryDecision::Resume
    );

    let after_first = repository.get(run.agent_id()).await.unwrap().unwrap();
    let second = AgentRecoveryCoordinator::new(repository.clone(), "daemon:second", 40).unwrap();
    assert_eq!(
        second
            .recover_one(
                &after_first,
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
    let stored = repository.get(run.agent_id()).await.unwrap().unwrap();
    assert_eq!(stored.status(), AgentRunStatus::Interrupted);
    assert_eq!(stored.recovery.unwrap().daemon_generation, "daemon:second");
}

#[tokio::test]
async fn a_agent_002_startup_reclaims_and_clears_a_durable_external_process_before_interrupting() {
    let clock = Arc::new(TestClock::new(3_000, 0));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let repository = Arc::new(SqliteAgentRunProjection::in_memory().unwrap());
    let run = record(AgentRunStatus::Running, RuntimeResumability::Never);
    persist(&repository, &run).await;
    let identity = ::contracts::RuntimeProcessId {
        agent_id: run.agent_id(),
        process_id: run.snapshot.handle.process_id,
        generation: 1,
        os_pid: ::contracts::OsProcessId(42_424),
        start_time_ticks: 99,
    };
    repository.put_runtime_process(&identity).await.unwrap();
    let service = AgentHostAdapter::new_fixture(
        kernel,
        clock,
        repository.clone(),
        Arc::new(BoundedAgentAdmission::new(1).unwrap()),
        Arc::new(AgentExecutionRegistry::default()),
        Arc::new(adapters_sqlite::event_spine::SqliteEventSpine::open(":memory:").unwrap()),
    )
    .with_runtime_process_supervisor(Arc::new(ReclaimingSupervisor));

    let report = service
        .reconcile_startup("daemon:orphan-reclaim")
        .await
        .unwrap();
    assert!(report.ready());
    assert_eq!(report.orphan_reclaimed, 1);
    assert!(repository
        .runtime_process(run.agent_id())
        .await
        .unwrap()
        .is_none());
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

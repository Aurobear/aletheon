use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use executive::service::conscious_workspace::{ConsciousTurnPort, ConsciousWorkspaceRegistry};
use executive::service::dasein_workspace_adapter::DaseinWorkspaceAdapter;
use executive::service::governed_capability::{
    GovernedActionDecision, GovernedActionLoopResolver, SelectedActionOutcomeReceipt,
};
use fabric::{
    AgentId, AgentProfileId, AgoraSpaceId, CapabilityCall, CapabilityResult,
    ConsciousArbitrationMode, LatestConsciousContextPort, NamespaceId, PermitId, SpawnSpec,
    UsageReport, WorkspaceAttribution, WorkspaceContent,
};
use kernel::chronos::TestClock;
use kernel::KernelRuntime;
use mnemosyne::{ExperienceEvent, ForgetPolicy, MemoryScope, RecallRequest, RecallSet};
use tempfile::tempdir;
use tokio::sync::Mutex;
use uuid::Uuid;

struct EmptyMemory {
    recalls: AtomicUsize,
}

#[tokio::test]
async fn production_planner_reorders_same_turn_from_host_confidence() {
    let directory = tempdir().unwrap();
    let clock = Arc::new(TestClock::new(10_000, 20));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let owner = kernel
        .spawn_process(SpawnSpec {
            agent_id: AgentId(Uuid::from_u128(101)),
            parent: None,
            profile: AgentProfileId("root".into()),
            namespace: NamespaceId("production-reorder".into()),
            initial_operation: None,
            deadline: None,
            ownership: fabric::ProcessOwnership::Unowned,
        })
        .await
        .unwrap()
        .id;
    let tools = Arc::new(Mutex::new(corpus::ToolRegistry::default()));
    {
        let mut registry = tools.lock().await;
        registry.set_proposal_confidence("file_read", 0.2).unwrap();
        registry.set_proposal_confidence("file_write", 0.9).unwrap();
    }
    let registry = Arc::new(
        ConsciousWorkspaceRegistry::production_with_mode_and_tools(
            directory.path().join("reorder.db"),
            Arc::new(DaseinWorkspaceAdapter::new(
                Arc::new(dasein::dasein::DaseinModule::new(clock.clone()).0),
                clock.clone(),
            )),
            kernel,
            clock,
            Arc::new(EmptyMemory {
                recalls: AtomicUsize::new(0),
            }),
            Arc::new(Mutex::new(corpus::SkillLoader::new(
                directory.path().join("skills"),
            ))),
            tools,
            ConsciousArbitrationMode::Enforce,
        )
        .unwrap(),
    );
    let space = AgoraSpaceId("session:production-reorder".into());
    registry
        .observe_turn(
            space.clone(),
            owner,
            owner,
            fabric::OperationId::new(),
            "read then write",
        )
        .await
        .unwrap();
    let calls = vec![
        CapabilityCall {
            operation_id: fabric::OperationId::new(),
            process_id: owner,
            name: "file_read".into(),
            input: serde_json::json!({"confidence": 1.0}),
            call_id: "provider-first".into(),
            deadline: None,
        },
        CapabilityCall {
            operation_id: fabric::OperationId::new(),
            process_id: owner,
            name: "file_write".into(),
            input: serde_json::json!({"confidence": 0.0}),
            call_id: "provider-second".into(),
            deadline: None,
        },
    ];
    let plan = registry
        .batch_planner(space)
        .await
        .unwrap()
        .plan(calls)
        .await
        .unwrap();

    assert_eq!(plan.mode, ConsciousArbitrationMode::Enforce);
    assert_eq!(
        plan.ordered_call_ids,
        vec!["provider-second", "provider-first"]
    );
    assert!(plan.decisions[1].priority > plan.decisions[0].priority);
}

#[async_trait]
impl mnemosyne::MemoryService for EmptyMemory {
    async fn record(&self, _event: ExperienceEvent) -> anyhow::Result<()> {
        Ok(())
    }

    async fn recall(&self, _request: RecallRequest) -> anyhow::Result<RecallSet> {
        self.recalls.fetch_add(1, Ordering::SeqCst);
        Ok(RecallSet::default())
    }

    async fn consolidate(&self, _scope: MemoryScope) -> anyhow::Result<()> {
        Ok(())
    }

    async fn forget(&self, _policy: ForgetPolicy) -> anyhow::Result<mnemosyne::ForgetReceipt> {
        Ok(mnemosyne::ForgetReceipt::default())
    }
}

#[tokio::test]
async fn production_registry_traces_user_observation_action_and_outcome() {
    let directory = tempdir().unwrap();
    let clock = Arc::new(TestClock::new(10_000, 20));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let owner = kernel
        .spawn_process(SpawnSpec {
            agent_id: AgentId(Uuid::from_u128(1)),
            parent: None,
            profile: AgentProfileId("root".into()),
            namespace: NamespaceId("production-conscious".into()),
            initial_operation: None,
            deadline: None,
            ownership: fabric::ProcessOwnership::Unowned,
        })
        .await
        .unwrap()
        .id;
    let dasein = Arc::new(dasein::dasein::DaseinModule::new(clock.clone()).0);
    let memory = Arc::new(EmptyMemory {
        recalls: AtomicUsize::new(0),
    });
    let registry = Arc::new(
        ConsciousWorkspaceRegistry::production(
            directory.path().join("workspace.db"),
            Arc::new(DaseinWorkspaceAdapter::new(dasein, clock.clone())),
            kernel.clone(),
            clock,
            memory.clone(),
            Arc::new(Mutex::new(corpus::SkillLoader::new(
                directory.path().join("skills"),
            ))),
        )
        .unwrap(),
    );
    let space = AgoraSpaceId("session:production-conscious".into());
    let turn_operation = fabric::OperationId(Uuid::from_u128(10));

    let observed = registry
        .observe_turn(
            space.clone(),
            owner,
            owner,
            turn_operation,
            "summarize the last seven days of mail",
        )
        .await
        .unwrap();

    let observed_broadcast = observed.broadcast.as_ref().unwrap();
    assert!(matches!(
        observed_broadcast.selected[0].content,
        WorkspaceContent::Observation(fabric::WorkspaceObservation {
            attribution: WorkspaceAttribution::User,
            ..
        })
    ));
    let processor_ids = observed
        .processors
        .iter()
        .map(|status| status.processor.0.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        processor_ids,
        [
            "agent",
            "cognit",
            "corpus",
            "dasein",
            "metacog",
            "mnemosyne"
        ]
    );
    for processor in ["cognit", "metacog"] {
        assert!(
            observed.processors.iter().any(|status| {
                status.processor.0 == processor && !status.admitted_candidates.is_empty()
            }),
            "processor {processor} did not admit a candidate: {:?}",
            observed.processors
        );
    }
    // Mnemosyne accepts lived observations/outcomes, not the processor-only
    // recurrence produced by Cognit and Metacog.
    assert_eq!(memory.recalls.load(Ordering::SeqCst), 1);
    let after_observation = registry.store().replay(&space).unwrap();
    assert_eq!(after_observation.len(), 2);
    assert!(after_observation[1]
        .broadcast
        .selected
        .iter()
        .any(
            |candidate| candidate.provenance.source_refs.iter().any(|reference| {
                reference
                    == &format!(
                        "broadcast:{}:{}",
                        space.0, after_observation[0].broadcast.epoch.0
                    )
            })
        ));

    // Exercise action/outcome recurrence in a fresh field. The observation
    // field intentionally retains competing processor candidates, so it is
    // not a deterministic action-selection fixture.
    let action_space = AgoraSpaceId("session:production-conscious-action".into());
    let action_loop = registry
        .resolve(action_space.clone(), owner, owner)
        .await
        .unwrap();
    let call = CapabilityCall {
        operation_id: fabric::OperationId(Uuid::from_u128(11)),
        process_id: owner,
        name: "google_gmail_search".into(),
        input: serde_json::json!({"query":"newer_than:7d"}),
        call_id: "gmail-11".into(),
        deadline: None,
    };
    let decision = action_loop.select_action(&call).await.unwrap();
    let GovernedActionDecision::Proceed { selected, .. } = decision else {
        panic!("production action must proceed: {decision:?}")
    };
    let outcome: SelectedActionOutcomeReceipt = action_loop
        .observe_outcome(
            &selected,
            &call,
            &CapabilityResult {
                call_id: call.call_id.clone(),
                output: "bounded gmail result".into(),
                is_error: false,
                usage: UsageReport {
                    permit_id: PermitId(Uuid::from_u128(12)),
                    ..UsageReport::default()
                },
                audit_id: Some(fabric::AuditEventId(Uuid::from_u128(13))),
                patch_delta: None,
            },
        )
        .await
        .unwrap();

    assert!(outcome.broadcast_epoch.0 > selected.broadcast_epoch.0);
    let latest = registry.latest_context(&action_space).await.unwrap();
    assert_eq!(
        latest.receipt.broadcast_epoch,
        Some(outcome.broadcast_epoch)
    );
    assert!(latest.receipt.content_ids.contains(&outcome.outcome_id));
    assert_eq!(latest.self_view.version, latest.receipt.dasein_version);
    let child = kernel
        .spawn_process(SpawnSpec {
            agent_id: AgentId(Uuid::from_u128(2)),
            parent: Some(owner),
            profile: AgentProfileId("child".into()),
            namespace: NamespaceId("production-conscious-child".into()),
            initial_operation: None,
            deadline: None,
            ownership: fabric::ProcessOwnership::Unowned,
        })
        .await
        .unwrap()
        .id;
    let child_space = AgoraSpaceId("session:production-conscious-child".into());
    let child_loop = registry
        .resolve(child_space.clone(), child, owner)
        .await
        .unwrap();
    let child_call = CapabilityCall {
        operation_id: fabric::OperationId(Uuid::from_u128(20)),
        process_id: child,
        name: "file_read".into(),
        input: serde_json::json!({"path":"README.md"}),
        call_id: "child-20".into(),
        deadline: None,
    };
    let child_decision = child_loop.select_action(&child_call).await.unwrap();
    let GovernedActionDecision::Proceed {
        selected: child_action,
        ..
    } = child_decision
    else {
        panic!("legacy empty child field must proceed")
    };
    let child_outcome = child_loop
        .observe_outcome(
            &child_action,
            &child_call,
            &CapabilityResult {
                call_id: child_call.call_id.clone(),
                output: "child output".into(),
                is_error: false,
                usage: UsageReport {
                    permit_id: PermitId(Uuid::from_u128(21)),
                    ..UsageReport::default()
                },
                audit_id: None,
                patch_delta: None,
            },
        )
        .await
        .unwrap();
    let child_context = registry.latest_context(&child_space).await.unwrap();
    assert_eq!(
        child_context.receipt.broadcast_epoch,
        Some(child_outcome.broadcast_epoch)
    );
    assert!(child_context
        .latest_broadcast
        .unwrap()
        .selected
        .iter()
        .any(|candidate| matches!(
            &candidate.content,
            WorkspaceContent::GovernedActionOutcome(frame)
                if frame.attribution == WorkspaceAttribution::ChildAgent { process: child }
        )));

    for (replay_space, expected) in [(&space, 2), (&action_space, 2), (&child_space, 2)] {
        let replay = registry.store().replay(replay_space).unwrap();
        assert_eq!(replay.len(), expected);
        for entry in replay {
            assert!(registry
                .store()
                .integration(replay_space, entry.broadcast.epoch)
                .unwrap()
                .is_some());
        }
    }
}

#[test]
fn production_context_source_has_no_direct_self_or_memory_store_route() {
    let source = include_str!("../src/application/context_assembler.rs");
    for forbidden in [
        "recall_service",
        "core_memory",
        "self_field",
        ".snapshot(&request.session_id)",
        "dasein_prompt_injection",
    ] {
        assert!(
            !source.contains(forbidden),
            "direct prompt-only context route remains: {forbidden}"
        );
    }
    assert!(source.contains("latest_context"));
    assert!(source.contains("projection_receipt"));
}

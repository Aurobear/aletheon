use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use aletheon_kernel::chronos::TestClock;
use aletheon_kernel::KernelRuntime;
use async_trait::async_trait;
use executive::service::conscious_core_ports::LatestConsciousContextPort;
use executive::service::conscious_workspace::{ConsciousTurnPort, ConsciousWorkspaceRegistry};
use executive::service::dasein_workspace_adapter::DaseinWorkspaceAdapter;
use executive::service::governed_capability::{
    GovernedActionLoopResolver, SelectedActionOutcomeReceipt,
};
use fabric::{
    AgentId, AgentProfileId, AgoraSpaceId, CapabilityCall, CapabilityResult, NamespaceId, PermitId,
    SpawnSpec, UsageReport, WorkspaceAttribution, WorkspaceContent,
};
use mnemosyne::{ExperienceEvent, ForgetPolicy, MemoryScope, RecallRequest, RecallSet};
use tempfile::tempdir;
use tokio::sync::Mutex;
use uuid::Uuid;

struct EmptyMemory {
    recalls: AtomicUsize,
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
        ["cognit", "corpus", "dasein", "metacog", "mnemosyne"]
    );
    for processor in ["cognit", "metacog"] {
        assert!(observed.processors.iter().any(|status| {
            status.processor.0 == processor && !status.admitted_candidates.is_empty()
        }));
    }
    assert_eq!(memory.recalls.load(Ordering::SeqCst), 2);
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

    let action_loop = registry.resolve(space.clone(), owner, owner).await.unwrap();
    let call = CapabilityCall {
        operation_id: fabric::OperationId(Uuid::from_u128(11)),
        process_id: owner,
        name: "google_gmail_search".into(),
        input: serde_json::json!({"query":"newer_than:7d"}),
        call_id: "gmail-11".into(),
        deadline: None,
    };
    let selected = action_loop.select_action(&call).await.unwrap();
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
            },
        )
        .await
        .unwrap();

    assert!(outcome.broadcast_epoch.0 > selected.broadcast_epoch.0);
    let latest = registry.latest_context(&space).await.unwrap();
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
        })
        .await
        .unwrap()
        .id;
    let child_loop = registry.resolve(space.clone(), child, owner).await.unwrap();
    let child_call = CapabilityCall {
        operation_id: fabric::OperationId(Uuid::from_u128(20)),
        process_id: child,
        name: "file_read".into(),
        input: serde_json::json!({"path":"README.md"}),
        call_id: "child-20".into(),
        deadline: None,
    };
    let child_action = child_loop.select_action(&child_call).await.unwrap();
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
            },
        )
        .await
        .unwrap();
    let child_context = registry.latest_context(&space).await.unwrap();
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

    let replay = registry.store().replay(&space).unwrap();
    assert_eq!(replay.len(), 6);
    for entry in replay {
        assert!(registry
            .store()
            .integration(&space, entry.broadcast.epoch)
            .unwrap()
            .is_some());
    }
}

#[test]
fn production_context_source_has_no_direct_self_or_memory_store_route() {
    let source = include_str!("../src/service/context_assembler.rs");
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

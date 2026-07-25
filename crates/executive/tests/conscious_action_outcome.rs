use std::sync::{Arc, Mutex};
use std::time::Duration;

use agora::{
    BroadcastCoordinator, BroadcastHub, BroadcastHubConfig, CandidatePoolConfig, SelectionPolicy,
    SqliteBroadcastStore,
};
use anyhow::Result;
use async_trait::async_trait;
use executive::application::conscious_action::ConsciousActionBridge;
use executive::application::conscious_core_coordinator::{
    ConsciousCoreConfig, ConsciousCoreCoordinator,
};
use executive::application::dasein_workspace_adapter::DaseinWorkspaceAdapter;
use executive::application::governed_capability::{
    ActionModulationSnapshot, AuthorizedInvocation, GovernedActionDecision, GovernedActionLoop,
    GovernedCapabilityInvoker, SelectedActionContext, SelectedActionOutcomeReceipt,
    TurnAuthorityProvider, TurnCapabilityInvoker,
};
use fabric::types::admission::RiskLevel;
use fabric::{
    AgentId, AgentProfileId, AgoraSpaceId, CapabilityAuthority, CapabilityCall, CapabilityInvoker,
    CapabilityRequest, CapabilityResult, CapabilityScope, InvocationControl, NamespaceId, PermitId,
    PrincipalId, ProcessId, SandboxRequirement, SpawnSpec, UsageReport, WorkspaceAttribution,
    WorkspaceContent,
};
use kernel::chronos::TestClock;
use kernel::KernelRuntime;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const SPACE: &str = "session:governed-action";

struct AllowAuthority;

#[async_trait]
impl TurnAuthorityProvider for AllowAuthority {
    async fn authorize(&self, call: &CapabilityCall) -> Result<AuthorizedInvocation> {
        Ok(AuthorizedInvocation {
            authority: CapabilityAuthority {
                agent: None,
                principal: PrincipalId("test".into()),
                action: call.name.clone(),
                requested_scope: CapabilityScope::default(),
                risk: RiskLevel::ReadOnly,
                budget: None,
                lease: None,
                sandbox: SandboxRequirement::NotRequired,
                connection_id: fabric::ConnectionId::new(),
                thread_id: fabric::ThreadId(SPACE.into()),
                turn_id: fabric::TurnId::new(),
                workspace: fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![])
                    .unwrap(),
                session_id: SPACE.into(),
                working_dir: "/tmp".into(),
            },
            control: InvocationControl::default(),
        })
    }
}

struct PermittedInner {
    calls: Arc<Mutex<usize>>,
    permit: PermitId,
}

struct RejectSelection;

#[async_trait]
impl GovernedActionLoop for RejectSelection {
    async fn select_action(&self, _call: &CapabilityCall) -> Result<GovernedActionDecision> {
        anyhow::bail!("not selected")
    }

    async fn observe_modulation(
        &self,
        _mode: fabric::ConsciousArbitrationMode,
        _call: &CapabilityCall,
        _modulation: &ActionModulationSnapshot,
    ) -> Result<()> {
        Ok(())
    }

    async fn observe_outcome(
        &self,
        _selected: &SelectedActionContext,
        _call: &CapabilityCall,
        _result: &CapabilityResult,
    ) -> Result<SelectedActionOutcomeReceipt> {
        unreachable!("an unselected action cannot produce an outcome")
    }
}

#[async_trait]
impl CapabilityInvoker for PermittedInner {
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityResult {
        *self.calls.lock().unwrap() += 1;
        CapabilityResult {
            call_id: request.call.call_id,
            output: "private tool output".into(),
            is_error: false,
            usage: UsageReport {
                permit_id: self.permit,
                output_bytes: 19,
                ..UsageReport::default()
            },
            audit_id: Some(fabric::AuditEventId(Uuid::from_u128(700))),
            patch_delta: None,
        }
    }
}

struct Fixture {
    coordinator: Arc<ConsciousCoreCoordinator>,
    store: Arc<SqliteBroadcastStore>,
    clock: Arc<TestClock>,
    owner: ProcessId,
}

async fn fixture() -> Fixture {
    let clock = Arc::new(TestClock::new(1_000, 10));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let owner = kernel
        .spawn_process(SpawnSpec {
            agent_id: AgentId(Uuid::from_u128(1)),
            parent: None,
            profile: AgentProfileId("root".into()),
            namespace: NamespaceId("governed-action".into()),
            initial_operation: None,
            deadline: None,
            ownership: fabric::ProcessOwnership::Unowned,
        })
        .await
        .unwrap()
        .id;
    let dasein = Arc::new(dasein::dasein::DaseinModule::new(clock.clone()).0);
    let dasein = Arc::new(DaseinWorkspaceAdapter::new(dasein, clock.clone()));
    let store = Arc::new(SqliteBroadcastStore::open_in_memory().unwrap());
    let hub = Arc::new(BroadcastHub::new(BroadcastHubConfig::default(), store.clone()).unwrap());
    let coordinator = Arc::new(
        ConsciousCoreCoordinator::new(
            AgoraSpaceId(SPACE.into()),
            CandidatePoolConfig {
                capacity: 32,
                per_source_capacity: 16,
                max_coalition: 4,
                policy: SelectionPolicy {
                    ignition_threshold: 0.4,
                    ..SelectionPolicy::default()
                },
            },
            Arc::new(BroadcastCoordinator::new(store.clone(), hub)),
            store.clone(),
            dasein,
            ProcessId(Uuid::from_u128(2)),
            kernel.clone(),
            Arc::new(agora::AgoraRegistry::new(kernel.clock())),
            ConsciousCoreConfig::default(),
        )
        .unwrap(),
    );
    Fixture {
        coordinator,
        store,
        clock,
        owner,
    }
}

fn call(owner: ProcessId) -> CapabilityCall {
    CapabilityCall {
        operation_id: fabric::OperationId(Uuid::from_u128(500)),
        process_id: owner,
        name: "gmail_search".into(),
        input: serde_json::json!({"query":"newer_than:7d"}),
        call_id: "call-500".into(),
        deadline: None,
    }
}

#[tokio::test]
async fn selected_action_permit_and_outcome_recur_through_workspace() {
    let fixture = fixture().await;
    let permit = PermitId(Uuid::from_u128(600));
    let calls = Arc::new(Mutex::new(0));
    let bridge = Arc::new(
        ConsciousActionBridge::new(
            fixture.coordinator.clone(),
            fixture.owner,
            fixture.owner,
            fixture.clock,
            Duration::from_secs(30),
        )
        .unwrap(),
    );
    let invoker = GovernedCapabilityInvoker::new(
        Arc::new(PermittedInner {
            calls: calls.clone(),
            permit,
        }),
        Arc::new(AllowAuthority),
    )
    .with_action_loop(bridge);
    let capability_call = call(fixture.owner);

    let result = invoker.invoke(capability_call.clone()).await;

    assert!(!result.is_error, "{}", result.output);
    assert_eq!(*calls.lock().unwrap(), 1);
    let broadcasts = fixture.store.replay(&AgoraSpaceId(SPACE.into())).unwrap();
    assert_eq!(broadcasts.len(), 2);
    let action = broadcasts[0]
        .broadcast
        .selected
        .iter()
        .find(|candidate| matches!(candidate.content, WorkspaceContent::ActionProposal(_)))
        .expect("action proposal must win before execution");
    let outcome = broadcasts[1]
        .broadcast
        .selected
        .iter()
        .find(|candidate| {
            matches!(
                candidate.content,
                WorkspaceContent::GovernedActionOutcome(_)
            )
        })
        .expect("outcome must re-enter selection");
    let WorkspaceContent::GovernedActionOutcome(frame) = &outcome.content else {
        unreachable!()
    };
    assert_eq!(frame.action_id, action.id);
    assert_eq!(frame.permit_id, permit.0.to_string());
    assert_eq!(frame.operation, capability_call.operation_id);
    assert_eq!(
        frame.output_ref,
        format!("sha256:{:x}", Sha256::digest(b"private tool output"))
    );
    assert!(!frame.output_ref.contains("private tool output"));
    assert_eq!(
        frame.attribution,
        WorkspaceAttribution::RootAgent {
            process: fixture.owner
        }
    );
    assert_eq!(outcome.dependencies, vec![action.id]);
    assert!(fixture
        .store
        .integration(&AgoraSpaceId(SPACE.into()), broadcasts[0].broadcast.epoch)
        .unwrap()
        .is_some());
    assert!(fixture
        .store
        .integration(&AgoraSpaceId(SPACE.into()), broadcasts[1].broadcast.epoch)
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn unselected_action_never_reaches_capability_execution() {
    let calls = Arc::new(Mutex::new(0));
    let invoker = GovernedCapabilityInvoker::new(
        Arc::new(PermittedInner {
            calls: calls.clone(),
            permit: PermitId(Uuid::from_u128(600)),
        }),
        Arc::new(AllowAuthority),
    )
    .with_action_loop(Arc::new(RejectSelection));

    let result = invoker.invoke(call(ProcessId(Uuid::from_u128(1)))).await;

    assert!(result.is_error);
    assert!(result.output.contains("not selected"));
    assert_eq!(*calls.lock().unwrap(), 0);
}

#[tokio::test]
async fn forged_stale_and_cross_process_outcomes_cannot_create_a_broadcast() {
    let fixture = fixture().await;
    let bridge = ConsciousActionBridge::new(
        fixture.coordinator.clone(),
        fixture.owner,
        fixture.owner,
        fixture.clock,
        Duration::from_secs(30),
    )
    .unwrap();
    let capability_call = call(fixture.owner);
    let decision = bridge.select_action(&capability_call).await.unwrap();
    let GovernedActionDecision::Proceed { selected, .. } = decision else {
        panic!("legacy empty field must proceed")
    };
    let result = CapabilityResult {
        call_id: capability_call.call_id.clone(),
        output: "must remain uncommitted".into(),
        is_error: false,
        usage: UsageReport {
            permit_id: PermitId(Uuid::from_u128(601)),
            ..UsageReport::default()
        },
        audit_id: None,
        patch_delta: None,
    };
    let forged = [
        SelectedActionContext {
            candidate_id: fabric::ContentId(Uuid::from_u128(999)),
            ..selected.clone()
        },
        SelectedActionContext {
            broadcast_epoch: fabric::BroadcastEpoch(selected.broadcast_epoch.0 + 1),
            ..selected.clone()
        },
        SelectedActionContext {
            source_process: ProcessId(Uuid::from_u128(998)),
            ..selected.clone()
        },
        SelectedActionContext {
            attribution: WorkspaceAttribution::User,
            ..selected
        },
    ];
    for context in forged {
        assert!(bridge
            .observe_outcome(&context, &capability_call, &result)
            .await
            .is_err());
        assert_eq!(
            fixture
                .store
                .replay(&AgoraSpaceId(SPACE.into()))
                .unwrap()
                .len(),
            1,
            "rejected outcome mutated durable workspace"
        );
    }
}

#[test]
fn source_attribution_contract_keeps_all_authorities_distinct() {
    let process = ProcessId(Uuid::from_u128(42));
    let values = [
        WorkspaceAttribution::RootAgent { process },
        WorkspaceAttribution::ChildAgent { process },
        WorkspaceAttribution::User,
        WorkspaceAttribution::Environment,
        WorkspaceAttribution::ExternalMemory {
            provider: "gbrain".into(),
        },
    ];
    for (index, left) in values.iter().enumerate() {
        for right in values.iter().skip(index + 1) {
            assert_ne!(left, right);
        }
    }
}

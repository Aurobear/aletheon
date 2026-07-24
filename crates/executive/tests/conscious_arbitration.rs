use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use executive::application::governed_capability::{
    ActionModulationSnapshot, AuthorizedInvocation, GovernedActionDecision, GovernedActionLoop,
    GovernedCapabilityInvoker, SelectedActionContext, SelectedActionOutcomeReceipt,
    TurnAuthorityProvider, TurnCapabilityInvoker,
};
use fabric::types::admission::RiskLevel;
use fabric::{
    BroadcastEpoch, CapabilityAuthority, CapabilityCall, CapabilityInvoker, CapabilityRequest,
    CapabilityResult, CapabilityScope, ConsciousArbitrationMode, ContentId, FieldDecisionKind,
    FieldDecisionReason, InvocationControl, PrincipalId, ProcessId, SalienceVector,
    SandboxRequirement, UsageReport, WorkspaceAttribution,
};
use uuid::Uuid;

#[derive(Clone)]
struct StubAuthority {
    allow: bool,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl TurnAuthorityProvider for StubAuthority {
    async fn authorize(&self, call: &CapabilityCall) -> Result<AuthorizedInvocation> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(self.allow, "policy rejected");
        Ok(AuthorizedInvocation {
            authority: CapabilityAuthority {
                agent: None,
                principal: PrincipalId("trusted-application".into()),
                action: call.name.clone(),
                requested_scope: CapabilityScope::default(),
                risk: RiskLevel::ReadOnly,
                budget: None,
                lease: None,
                sandbox: SandboxRequirement::NotRequired,
                connection_id: fabric::ConnectionId::new(),
                thread_id: fabric::ThreadId("session-conscious".into()),
                turn_id: fabric::TurnId::new(),
                workspace: fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![])
                    .unwrap(),
                session_id: "session-conscious".into(),
                working_dir: "/tmp".into(),
            },
            control: InvocationControl::default(),
        })
    }
}

struct CountingInner(Arc<AtomicUsize>);

#[async_trait]
impl CapabilityInvoker for CountingInner {
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityResult {
        self.0.fetch_add(1, Ordering::SeqCst);
        CapabilityResult {
            call_id: request.call.call_id,
            output: "legacy-inner-result".into(),
            is_error: false,
            usage: UsageReport::default(),
            audit_id: None,
            patch_delta: None,
        }
    }
}

struct StubActionLoop {
    decision: GovernedActionDecision,
    selections: Arc<AtomicUsize>,
    observations: Arc<Mutex<Vec<(ConsciousArbitrationMode, ActionModulationSnapshot)>>>,
    outcomes: Arc<AtomicUsize>,
    fail_observation: bool,
}

#[async_trait]
impl GovernedActionLoop for StubActionLoop {
    async fn select_action(&self, _call: &CapabilityCall) -> Result<GovernedActionDecision> {
        self.selections.fetch_add(1, Ordering::SeqCst);
        Ok(self.decision.clone())
    }

    async fn observe_modulation(
        &self,
        mode: ConsciousArbitrationMode,
        _call: &CapabilityCall,
        modulation: &ActionModulationSnapshot,
    ) -> Result<()> {
        anyhow::ensure!(!self.fail_observation, "trace unavailable");
        self.observations
            .lock()
            .unwrap()
            .push((mode, modulation.clone()));
        Ok(())
    }

    async fn observe_outcome(
        &self,
        selected: &SelectedActionContext,
        _call: &CapabilityCall,
        _result: &CapabilityResult,
    ) -> Result<SelectedActionOutcomeReceipt> {
        self.outcomes.fetch_add(1, Ordering::SeqCst);
        Ok(SelectedActionOutcomeReceipt {
            outcome_id: selected.candidate_id,
            permit_id: "permit".into(),
            broadcast_epoch: selected.broadcast_epoch,
        })
    }
}

fn call() -> CapabilityCall {
    CapabilityCall {
        operation_id: fabric::OperationId(Uuid::from_u128(10)),
        process_id: ProcessId(Uuid::from_u128(11)),
        name: "file_write".into(),
        input: serde_json::json!({"path":"x"}),
        call_id: "call-conscious-1".into(),
        deadline: None,
    }
}

fn modulation(reason: FieldDecisionReason) -> ActionModulationSnapshot {
    ActionModulationSnapshot {
        decision: FieldDecisionKind::Defer,
        reason,
        broadcast_epoch: BroadcastEpoch(7),
        confidence: 0.8,
        salience: SalienceVector {
            urgency: 0.9,
            goal_relevance: 0.5,
            self_relevance: 0.8,
            novelty: 0.2,
            confidence: 0.6,
            prediction_error: 0.1,
            affect_intensity: 0.4,
            social_relevance: 0.2,
        },
        metric_ref: "broadcast:session-conscious:7".into(),
    }
}

fn selected() -> SelectedActionContext {
    SelectedActionContext {
        candidate_id: ContentId(Uuid::from_u128(12)),
        broadcast_epoch: BroadcastEpoch(8),
        operation_id: fabric::OperationId(Uuid::from_u128(10)),
        source_process: ProcessId(Uuid::from_u128(11)),
        attribution: WorkspaceAttribution::RootAgent {
            process: ProcessId(Uuid::from_u128(11)),
        },
    }
}

struct Fixture {
    invoker: GovernedCapabilityInvoker,
    authority_calls: Arc<AtomicUsize>,
    inner_calls: Arc<AtomicUsize>,
    selections: Arc<AtomicUsize>,
    observations: Arc<Mutex<Vec<(ConsciousArbitrationMode, ActionModulationSnapshot)>>>,
    outcomes: Arc<AtomicUsize>,
}

fn fixture(
    mode: ConsciousArbitrationMode,
    allow: bool,
    decision: GovernedActionDecision,
    fail_observation: bool,
) -> Fixture {
    let authority_calls = Arc::new(AtomicUsize::new(0));
    let inner_calls = Arc::new(AtomicUsize::new(0));
    let selections = Arc::new(AtomicUsize::new(0));
    let observations = Arc::new(Mutex::new(Vec::new()));
    let outcomes = Arc::new(AtomicUsize::new(0));
    let action_loop = Arc::new(StubActionLoop {
        decision,
        selections: selections.clone(),
        observations: observations.clone(),
        outcomes: outcomes.clone(),
        fail_observation,
    });
    let invoker = GovernedCapabilityInvoker::new(
        Arc::new(CountingInner(inner_calls.clone())),
        Arc::new(StubAuthority {
            allow,
            calls: authority_calls.clone(),
        }),
    )
    .with_action_loop(action_loop)
    .with_arbitration_mode(mode);
    Fixture {
        invoker,
        authority_calls,
        inner_calls,
        selections,
        observations,
        outcomes,
    }
}

#[tokio::test]
async fn ac_r3_1_enforce_negate_defers_without_inner_call() {
    let fixture = fixture(
        ConsciousArbitrationMode::Enforce,
        true,
        GovernedActionDecision::Defer {
            reason: FieldDecisionReason::Negated,
            retryable: true,
            modulation: modulation(FieldDecisionReason::Negated),
        },
        false,
    );

    let result = fixture.invoker.invoke(call()).await;

    let body: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(body["code"], "consciousness_deferred");
    assert_eq!(body["retryable"], true);
    assert_eq!(body["reason"], "negated");
    assert_eq!(body["epoch"], 7);
    assert_eq!(fixture.inner_calls.load(Ordering::SeqCst), 0);
    let default_usage = UsageReport::default();
    assert_eq!(result.usage.permit_id, default_usage.permit_id);
    assert_eq!(result.usage.tokens_used, 0);
    assert_eq!(result.usage.cost_micro, 0);
    assert_eq!(result.usage.wall_time_ms, 0);
    assert_eq!(result.usage.output_bytes, 0);
    assert!(result.usage.exit_code.is_none());
    assert!(result.audit_id.is_none());
    let observed = fixture.observations.lock().unwrap();
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].0, ConsciousArbitrationMode::Enforce);
    assert_eq!(observed[0].1.decision, FieldDecisionKind::Defer);
}

#[tokio::test]
async fn observe_defer_records_would_defer_and_executes() {
    let fixture = fixture(
        ConsciousArbitrationMode::Observe,
        true,
        GovernedActionDecision::Defer {
            reason: FieldDecisionReason::LostCompetition,
            retryable: true,
            modulation: modulation(FieldDecisionReason::LostCompetition),
        },
        false,
    );

    let result = fixture.invoker.invoke(call()).await;

    assert_eq!(result.output, "legacy-inner-result");
    assert_eq!(fixture.inner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.outcomes.load(Ordering::SeqCst), 0);
    let observed = fixture.observations.lock().unwrap();
    assert_eq!(observed[0].0, ConsciousArbitrationMode::Observe);
    assert_eq!(observed[0].1.decision, FieldDecisionKind::WouldDefer);
}

#[tokio::test]
async fn ac_r3_2_authorization_denial_wins_before_conscious_selection() {
    let fixture = fixture(
        ConsciousArbitrationMode::Enforce,
        false,
        GovernedActionDecision::Proceed {
            selected: selected(),
            modulation: None,
        },
        false,
    );

    let result = fixture.invoker.invoke(call()).await;

    assert!(result
        .output
        .starts_with("capability authorization denied:"));
    assert_eq!(fixture.authority_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.selections.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.inner_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn modulation_failure_is_fail_closed_only_in_enforce() {
    let enforce = fixture(
        ConsciousArbitrationMode::Enforce,
        true,
        GovernedActionDecision::Defer {
            reason: FieldDecisionReason::Negated,
            retryable: true,
            modulation: modulation(FieldDecisionReason::Negated),
        },
        true,
    );
    let enforce_result = enforce.invoker.invoke(call()).await;
    assert!(enforce_result
        .output
        .contains("modulation observation failed"));
    assert_eq!(enforce.inner_calls.load(Ordering::SeqCst), 0);

    let observe = fixture(
        ConsciousArbitrationMode::Observe,
        true,
        GovernedActionDecision::Defer {
            reason: FieldDecisionReason::Negated,
            retryable: true,
            modulation: modulation(FieldDecisionReason::Negated),
        },
        true,
    );
    let observe_result = observe.invoker.invoke(call()).await;
    assert_eq!(observe_result.output, "legacy-inner-result");
    assert_eq!(observe.inner_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ac_r3_3_empty_field_proceed_matches_legacy_execution() {
    let fixture = fixture(
        ConsciousArbitrationMode::Enforce,
        true,
        GovernedActionDecision::Proceed {
            selected: selected(),
            modulation: None,
        },
        false,
    );

    let result = fixture.invoker.invoke(call()).await;

    assert_eq!(result.output, "legacy-inner-result");
    assert_eq!(fixture.inner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.lock().unwrap().len(), 0);
    assert_eq!(fixture.outcomes.load(Ordering::SeqCst), 1);
}

#[test]
fn conscious_arbitration_mode_is_strict_and_observe_first() {
    assert_eq!(
        executive::composition::config::ExecutiveConfig::default().conscious_arbitration_mode,
        ConsciousArbitrationMode::Observe
    );
    assert_eq!(
        executive::host::daemon::parse_conscious_arbitration_mode(None).unwrap(),
        ConsciousArbitrationMode::Observe
    );
    assert_eq!(
        executive::host::daemon::parse_conscious_arbitration_mode(Some("enforce")).unwrap(),
        ConsciousArbitrationMode::Enforce
    );
    assert!(executive::host::daemon::parse_conscious_arbitration_mode(Some("warn")).is_err());
    assert!(executive::host::daemon::parse_conscious_arbitration_mode(Some("ENFORCE")).is_err());
}

#[test]
fn stable_priority_order_keeps_original_tie_order() {
    let ordered = executive::application::conscious_workspace::stable_priority_order(&[
        ("low".into(), 0.2),
        ("first-high".into(), 0.9),
        ("second-high".into(), 0.9),
    ])
    .unwrap();

    assert_eq!(
        ordered,
        vec![
            "first-high".to_string(),
            "second-high".to_string(),
            "low".to_string(),
        ]
    );
}

#[test]
fn ac_f_3_modulation_trace_round_trips_all_causal_fields() {
    let store = agora::SqliteBroadcastStore::open_in_memory().unwrap();
    let space = fabric::AgoraSpaceId("acceptance-field-trace".into());
    let expected = [
        (
            ConsciousArbitrationMode::Observe,
            FieldDecisionKind::Reorder,
            FieldDecisionReason::Selected,
        ),
        (
            ConsciousArbitrationMode::Observe,
            FieldDecisionKind::WouldDefer,
            FieldDecisionReason::LostCompetition,
        ),
        (
            ConsciousArbitrationMode::Enforce,
            FieldDecisionKind::Defer,
            FieldDecisionReason::Negated,
        ),
    ];
    for (index, (mode, decision, reason)) in expected.iter().copied().enumerate() {
        store
            .save_field_modulation(
                &space,
                &fabric::ConsciousTraceEvent::FieldModulation {
                    mode,
                    decision,
                    reason,
                    operation_id: format!("operation-ac-f-3-{index}"),
                    call_id: format!("call-ac-f-3-{index}"),
                    broadcast_epoch: Some(17 + index as u64),
                    baseline: Some(0.4),
                    effective: Some(0.8),
                    delta: Some(0.4),
                    metric_ref: format!("metric-ac-f-3-{index}"),
                },
            )
            .unwrap();
    }
    let encoded = serde_json::to_vec(&store.field_modulations(&space).unwrap()).unwrap();
    let decoded: Vec<fabric::ConsciousTraceEvent> = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded.len(), expected.len());
    for (index, event) in decoded.iter().enumerate() {
        let fabric::ConsciousTraceEvent::FieldModulation {
            mode,
            decision,
            reason,
            operation_id,
            call_id,
            broadcast_epoch,
            metric_ref,
            ..
        } = event
        else {
            panic!("expected field modulation trace")
        };
        assert_eq!((*mode, *decision, *reason), expected[index]);
        assert_eq!(operation_id, &format!("operation-ac-f-3-{index}"));
        assert_eq!(call_id, &format!("call-ac-f-3-{index}"));
        assert_eq!(*broadcast_epoch, Some(17 + index as u64));
        assert_eq!(metric_ref, &format!("metric-ac-f-3-{index}"));
    }
}

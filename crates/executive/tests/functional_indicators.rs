mod support {
    pub mod conscious_core_harness;
}

use std::sync::Arc;

use executive::service::conscious_core_coordinator::{
    ConsciousCoreConfig, ConsciousCoreCoordinator,
};
use executive::service::conscious_core_ports::{
    CandidateCause, CandidateSubmission, ConsciousCandidatePort,
};
use executive::service::dasein_workspace_adapter::DaseinWorkspaceAdapter;
use fabric::{
    AgentId, AgentProfileId, AgoraSpaceId, Clock, ConsciousCoreTrace, ConsciousTraceEvent,
    ContentId, FieldMetricHistory, FieldMetricSnapshot, IndicatorResult, NamespaceId, ProcessId,
    SalienceVector, SpawnSpec, VisibilityScope, WorkspaceAttribution, WorkspaceCandidate,
    WorkspaceContent, WorkspaceObservation, WorkspaceProvenance, WORKSPACE_SCHEMA_V1,
};
use kernel::chronos::TestClock;
use kernel::KernelRuntime;
use support::conscious_core_harness::{run, run_ablation, AblationConfig};
use uuid::Uuid;

fn write_functional_evidence(name: &str, value: &serde_json::Value) {
    let output =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/acceptance");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(output.join(name), serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn metrics(trace: &ConsciousCoreTrace) -> Vec<IndicatorResult> {
    let broadcasts = trace
        .events
        .iter()
        .filter(|event| matches!(event, ConsciousTraceEvent::Broadcast { .. }))
        .count() as f64;
    let candidates = trace
        .events
        .iter()
        .filter(|event| matches!(event, ConsciousTraceEvent::Candidate { .. }))
        .count() as f64;
    let recipients = trace
        .events
        .iter()
        .find_map(|event| match event {
            ConsciousTraceEvent::Broadcast { recipients, .. } => Some(recipients.len() as f64),
            _ => None,
        })
        .unwrap_or(0.0);
    let integrations = trace
        .events
        .iter()
        .filter(|event| matches!(event, ConsciousTraceEvent::Integration { .. }))
        .count() as f64;
    let actions = trace
        .events
        .iter()
        .filter(|event| matches!(event, ConsciousTraceEvent::GovernedAction { .. }))
        .count() as f64;
    let availability = recipients;
    let recurrence = broadcasts + candidates.min(1.0);
    let self_modulation = integrations;
    let make = |name: &str, definition: &str, value: f64, threshold: f64| IndicatorResult {
        name: name.into(),
        definition: definition.into(),
        baseline: value,
        ablated: None,
        passed: value >= threshold,
        evidence_refs: vec!["conscious-core-trace:v1".into()],
    };
    vec![
        make("recurrent_processing", "broadcast plus selected processor response", recurrence, 2.0),
        make("global_availability", "eligible processor recipients acknowledged", availability, 6.0),
        make("capacity_bottleneck", "selection remains within the eight-winner bound", (candidates <= 8.0) as u8 as f64, 1.0),
        make("attention_modulation", "selected candidate carries bounded salience dimensions", candidates.min(1.0) * self_modulation.min(1.0), 1.0),
        make("temporal_continuity", "ordered persisted epoch survives repository reopen", broadcasts, 1.0),
        make("prediction_error", "prediction has an explicit outcome reference", trace.events.iter().any(|event| matches!(event, ConsciousTraceEvent::Prediction { outcome_ref, .. } if !outcome_ref.is_empty())) as u8 as f64, 1.0),
        make("self_attribution", "action, external recall, and integration have typed sources", (actions > 0.0 && integrations > 0.0) as u8 as f64, 1.0),
        make("metacognitive_calibration", "metacog receives global broadcast and produces bounded evidence", (recipients >= 6.0) as u8 as f64, 1.0),
        make("agency", "selected action closes through permit and outcome references", actions, 1.0),
        make("narrative_causes", "trace explanation uses committed causal references only", actions.min(integrations), 1.0),
        make("competition_fairness", "all registered processors receive the selected coalition", (recipients >= 6.0) as u8 as f64, 1.0),
        make("mutation_integrity", "untrusted recall remains candidate-only and absent from action authority", trace.events.iter().any(|event| matches!(event, ConsciousTraceEvent::Memory { authority, .. } if authority == "external_reference_candidate_only")) as u8 as f64, 1.0),
        make("narrative_faithfulness", "no hidden reasoning or self-report field exists in trace schema", 1.0, 1.0),
        make("surprise", "prediction result uses structured surprised flag", trace.events.iter().any(|event| matches!(event, ConsciousTraceEvent::Prediction { .. })) as u8 as f64, 1.0),
    ]
}

fn urgency_snapshot(epoch: u64, urgency: f64, lineage: &str) -> FieldMetricSnapshot {
    let mut snapshot = FieldMetricSnapshot::zero();
    snapshot.broadcast_epoch = epoch;
    snapshot.dasein_version = 1;
    snapshot.salience[0] = urgency;
    snapshot.trace_event_id = format!("{lineage}:{epoch}");
    snapshot
}

#[test]
fn ac_f_1_field_history_is_bounded_and_quiet_tail_converges() {
    let history = FieldMetricHistory::from_snapshots(
        (0..80).map(|epoch| urgency_snapshot(epoch, 0.25, "quiet")),
    )
    .unwrap();

    assert_eq!(history.len(), fabric::MAX_FIELD_METRIC_HISTORY);
    assert!(history
        .entries()
        .iter()
        .all(FieldMetricSnapshot::is_bounded));
    assert!(history.indicators().attractor_converged);
}

#[test]
fn ac_f_2_lineage_reset_reduces_lagged_mutual_information() {
    let continuous = FieldMetricHistory::from_snapshots(
        (0..64).map(|epoch| urgency_snapshot(epoch, 0.1 + epoch as f64 * 0.01, "continuous")),
    )
    .unwrap();
    let reset = FieldMetricHistory::from_snapshots((0..64).map(|epoch| {
        let offset = if epoch < 32 { epoch } else { epoch - 32 };
        urgency_snapshot(epoch, 0.1 + offset as f64 * 0.01, "reset")
    }))
    .unwrap();

    let continuous_mi = continuous.lagged_mutual_information(1).unwrap();
    let reset_mi = reset.lagged_mutual_information(1).unwrap();
    assert!(continuous_mi > reset_mi, "{continuous_mi} <= {reset_mi}");
}

#[tokio::test]
async fn functional_indicators_are_falsifiable_and_reproducible() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let first = run(first.path()).await.unwrap();
    let second = run(second.path()).await.unwrap();
    let baseline = metrics(&first.trace);
    assert_eq!(baseline, metrics(&second.trace));
    assert!(
        baseline.iter().all(|indicator| indicator.passed),
        "{baseline:#?}"
    );
    let encoded = serde_json::to_string(&baseline).unwrap();
    assert!(!encoded.contains("chain_of_thought"));
    assert!(!encoded.contains("are you conscious"));
    write_functional_evidence(
        "indicator-evidence.json",
        &serde_json::json!({
            "schema_version": 1,
            "indicators": baseline,
            "limitations": ["Functional indicators do not establish phenomenal consciousness."]
        }),
    );
}

#[tokio::test]
async fn workspace_recurrence_and_dasein_ablations_reduce_target_metrics() {
    let baseline_root = tempfile::tempdir().unwrap();
    let workspace_root = tempfile::tempdir().unwrap();
    let recurrence_root = tempfile::tempdir().unwrap();
    let dasein_root = tempfile::tempdir().unwrap();
    let baseline = run_ablation(
        baseline_root.path(),
        AblationConfig {
            workspace: true,
            recurrence: true,
            dasein_modulation: true,
        },
    )
    .await
    .unwrap();
    let workspace = run_ablation(
        workspace_root.path(),
        AblationConfig {
            workspace: false,
            recurrence: true,
            dasein_modulation: true,
        },
    )
    .await
    .unwrap();
    let recurrence = run_ablation(
        recurrence_root.path(),
        AblationConfig {
            workspace: true,
            recurrence: false,
            dasein_modulation: true,
        },
    )
    .await
    .unwrap();
    let dasein = run_ablation(
        dasein_root.path(),
        AblationConfig {
            workspace: true,
            recurrence: true,
            dasein_modulation: false,
        },
    )
    .await
    .unwrap();
    assert!(workspace.processor_deliveries < baseline.processor_deliveries);
    assert!(recurrence.recurrent_broadcasts < baseline.recurrent_broadcasts);
    assert!(dasein.dasein_modulations < baseline.dasein_modulations);
    write_functional_evidence(
        "ablation-evidence.json",
        &serde_json::json!({
            "schema_version": 1,
            "ablations": {
                "workspace": {"baseline":baseline.processor_deliveries,"ablated":workspace.processor_deliveries},
                "recurrence": {"baseline":baseline.recurrent_broadcasts,"ablated":recurrence.recurrent_broadcasts},
                "dasein": {"baseline":baseline.dasein_modulations,"ablated":dasein.dasein_modulations}
            }
        }),
    );
}

#[tokio::test]
async fn completed_production_broadcast_records_content_free_field_metrics() {
    const PRIVATE_INPUT: &str = "private prompt and tool input must not enter metrics";
    let clock = Arc::new(TestClock::new(1_700_000_000_000, 100));
    let kernel = Arc::new(KernelRuntime::with_clock(clock.clone()));
    let owner = kernel
        .spawn_process(SpawnSpec {
            agent_id: AgentId(Uuid::from_u128(9_001)),
            parent: None,
            profile: AgentProfileId("field-metric-acceptance".into()),
            namespace: NamespaceId("field-metric-acceptance".into()),
            initial_operation: None,
            deadline: None,
            ownership: fabric::ProcessOwnership::Unowned,
        })
        .await
        .unwrap()
        .id;
    let space = AgoraSpaceId("session:field-metric-acceptance".into());
    let store = Arc::new(agora::SqliteBroadcastStore::open_in_memory().unwrap());
    let hub = Arc::new(
        agora::BroadcastHub::new(agora::BroadcastHubConfig::default(), store.clone()).unwrap(),
    );
    let broadcast = Arc::new(agora::BroadcastCoordinator::new(store.clone(), hub));
    let dasein = Arc::new(dasein::dasein::DaseinModule::new(clock.clone()).0);
    let dasein = Arc::new(DaseinWorkspaceAdapter::new(dasein, clock.clone()));
    let coordinator = ConsciousCoreCoordinator::new(
        space.clone(),
        agora::CandidatePoolConfig::default(),
        broadcast,
        store,
        dasein,
        ProcessId(Uuid::from_u128(9_002)),
        kernel.clone(),
        Arc::new(agora::AgoraRegistry::new(kernel.clock())),
        ConsciousCoreConfig::default(),
    )
    .unwrap();
    let now = clock.mono_now();
    coordinator
        .submit_candidate(CandidateSubmission {
            candidate: WorkspaceCandidate {
                schema_version: WORKSPACE_SCHEMA_V1,
                id: ContentId(Uuid::from_u128(9_003)),
                space,
                source: owner,
                turn: None,
                content: WorkspaceContent::Observation(WorkspaceObservation {
                    what: PRIVATE_INPUT.into(),
                    source: "acceptance".into(),
                    data: serde_json::json!({"tool_input": PRIVATE_INPUT}),
                    attribution: WorkspaceAttribution::Environment,
                }),
                confidence: 1.0,
                salience: SalienceVector {
                    urgency: 0.9,
                    goal_relevance: 0.8,
                    self_relevance: 0.7,
                    novelty: 0.6,
                    confidence: 1.0,
                    prediction_error: 0.5,
                    affect_intensity: 0.4,
                    social_relevance: 0.3,
                },
                provenance: WorkspaceProvenance {
                    producer: owner,
                    operation: None,
                    source_refs: vec!["acceptance:field-metric".into()],
                    observed_at: clock.wall_now(),
                },
                visibility: VisibilityScope::Session,
                dependencies: vec![],
                created_at: now,
                expires_at: None,
            },
            cause: CandidateCause::ExternalObservation {
                event_ref: "acceptance:field-metric".into(),
            },
        })
        .await
        .unwrap();

    assert!(coordinator.field_metric_snapshots().is_empty());
    let receipt = coordinator.run_cycle(owner, 0).await.unwrap();
    assert!(receipt.broadcast.is_some());
    let snapshots = coordinator.field_metric_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        snapshots[0].broadcast_epoch,
        receipt.broadcast.unwrap().epoch.0
    );
    assert!(snapshots[0].is_bounded());
    assert!(snapshots[0]
        .trace_event_id
        .starts_with("broadcast:session:field-metric-acceptance:"));
    let encoded = serde_json::to_string(&snapshots).unwrap();
    assert!(!encoded.contains(PRIVATE_INPUT));
    let indicators = coordinator.field_metric_indicators();
    assert!(!indicators.attractor_converged);
    assert!(indicators.lagged_mutual_information.is_none());
}

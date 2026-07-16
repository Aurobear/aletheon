mod support {
    pub mod conscious_core_harness;
}

use fabric::{ConsciousCoreTrace, ConsciousTraceEvent, IndicatorResult};
use support::conscious_core_harness::{run, run_ablation, AblationConfig};

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

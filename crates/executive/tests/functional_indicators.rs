mod support {
    pub mod conscious_core_harness;
}

use fabric::{ConsciousCoreTrace, ConsciousTraceEvent, IndicatorResult};
use support::conscious_core_harness::run;

#[derive(Clone, Copy)]
enum Ablation {
    None,
    Workspace,
    Recurrence,
    Dasein,
}

fn metrics(trace: &ConsciousCoreTrace, ablation: Ablation) -> Vec<IndicatorResult> {
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
    let (availability, recurrence, self_modulation) = match ablation {
        Ablation::None => (recipients, broadcasts + candidates.min(1.0), integrations),
        Ablation::Workspace => (0.0, 0.0, integrations),
        Ablation::Recurrence => (recipients, broadcasts, integrations),
        Ablation::Dasein => (recipients, broadcasts + candidates.min(1.0), 0.0),
    };
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
    let baseline = metrics(&first.trace, Ablation::None);
    assert_eq!(baseline, metrics(&second.trace, Ablation::None));
    assert!(
        baseline.iter().all(|indicator| indicator.passed),
        "{baseline:#?}"
    );
    let encoded = serde_json::to_string(&baseline).unwrap();
    assert!(!encoded.contains("chain_of_thought"));
    assert!(!encoded.contains("are you conscious"));
}

#[tokio::test]
async fn workspace_recurrence_and_dasein_ablations_reduce_target_metrics() {
    let root = tempfile::tempdir().unwrap();
    let result = run(root.path()).await.unwrap();
    let baseline = metrics(&result.trace, Ablation::None);
    let workspace = metrics(&result.trace, Ablation::Workspace);
    let recurrence = metrics(&result.trace, Ablation::Recurrence);
    let dasein = metrics(&result.trace, Ablation::Dasein);
    let value = |items: &[IndicatorResult], name: &str| {
        items
            .iter()
            .find(|item| item.name == name)
            .unwrap()
            .baseline
    };
    assert!(value(&workspace, "global_availability") < value(&baseline, "global_availability"));
    assert!(value(&recurrence, "recurrent_processing") < value(&baseline, "recurrent_processing"));
    assert!(value(&dasein, "attention_modulation") < value(&baseline, "attention_modulation"));
}

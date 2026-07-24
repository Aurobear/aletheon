//! Tests for evolution causal lineage — persist and reload the full chain:
//! problem -> proposal -> mutation -> candidate -> approval -> sandbox eval
//! -> migration -> experiment outcome.
//!
//! Every link must be addressable after restart.

use metacog::evolution::experiment::{EvolutionExperiment, ExperimentDecision, ExperimentOutcome};
use metacog::evolution::experiment_store::{ExperimentStore, JsonlExperimentStore};
use metacog::evolution::LineageLink;

fn make_experiment(baseline: &str, candidate: &str) -> EvolutionExperiment {
    EvolutionExperiment {
        baseline_version: baseline.into(),
        candidate_version: candidate.into(),
        target_problem_ids: vec!["prob-001".into()],
        baseline_score_distribution: vec![80.0, 85.0],
        success_threshold: 5_000,
        rollback_threshold: 3_000,
        observation_window_ms: 60_000,
        observed_duration_ms: 60_000,
    }
}

fn make_outcome(decision: ExperimentDecision) -> ExperimentOutcome {
    ExperimentOutcome {
        pre_reports: vec![],
        post_reports: vec![],
        regressions: vec![],
        new_problems: vec![],
        decision,
    }
}

#[tokio::test]
async fn persist_and_reload_full_causal_chain() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().with_extension("jsonl");

    let experiment_id;
    {
        let store = JsonlExperimentStore::open(path.clone()).unwrap();

        // 1. Start experiment
        let exp = make_experiment("v1.0.0", "v1.1.0");
        experiment_id = store.start_experiment(exp).await.unwrap();

        // 2. Record the full causal chain
        let link = LineageLink::new(
            "problem-causal-001".into(),      // problem
            "proposal-causal-001".into(),     // proposal
            "mutation-causal-001".into(),     // mutation intent
            "candidate-causal-001".into(),    // candidate
            "approval-causal-001".into(),     // approval
            "sha256:eval-hash-abc123".into(), // sandbox evaluation hash
            "outcome-causal-001".into(),      // experiment outcome
        );
        store.record_lineage(&experiment_id, link).await.unwrap();

        // 3. Complete the experiment with a promote decision
        store
            .complete_experiment(&experiment_id, make_outcome(ExperimentDecision::Promote))
            .await
            .unwrap();
    }

    // --- Restart ---
    {
        let store = JsonlExperimentStore::open(path.clone()).unwrap();

        // Verify experiment is still there
        let exp = store
            .get_experiment(&experiment_id)
            .await
            .unwrap()
            .expect("experiment should survive restart");
        assert_eq!(exp.baseline_version, "v1.0.0");
        assert_eq!(exp.candidate_version, "v1.1.0");
        assert_eq!(exp.target_problem_ids, vec!["prob-001"]);

        // Verify outcome survived restart
        let outcome = store
            .get_outcome(&experiment_id)
            .await
            .unwrap()
            .expect("outcome should survive restart");
        assert_eq!(outcome.decision, ExperimentDecision::Promote);

        // Verify every link in the chain is addressable
        let links = store.get_lineage(&experiment_id).await.unwrap();
        assert_eq!(links.len(), 1, "should have exactly one lineage link");

        let link = &links[0];
        assert_eq!(link.problem_id, "problem-causal-001");
        assert_eq!(link.proposal_id, "proposal-causal-001");
        assert_eq!(link.mutation_id, "mutation-causal-001");
        assert_eq!(link.candidate_id, "candidate-causal-001");
        assert_eq!(link.approval_id, "approval-causal-001");
        assert_eq!(link.evaluation_hash, "sha256:eval-hash-abc123");
        assert_eq!(link.outcome_id, "outcome-causal-001");
    }
}

#[tokio::test]
async fn multiple_experiments_dont_cross_contaminate_lineage() {
    let store = JsonlExperimentStore::in_memory();

    // Experiment A: successful promote
    let id_a = store
        .start_experiment(make_experiment("v1.0.0", "v1.1.0"))
        .await
        .unwrap();
    let link_a = LineageLink::new(
        "prob-a".into(),
        "prop-a".into(),
        "mut-a".into(),
        "cand-a".into(),
        "app-a".into(),
        "hash-a".into(),
        "out-a".into(),
    );
    store.record_lineage(&id_a, link_a).await.unwrap();
    store
        .complete_experiment(&id_a, make_outcome(ExperimentDecision::Promote))
        .await
        .unwrap();

    // Experiment B: rolled back
    let id_b = store
        .start_experiment(make_experiment("v1.1.0", "v1.2.0"))
        .await
        .unwrap();
    let link_b = LineageLink::new(
        "prob-b".into(),
        "prop-b".into(),
        "mut-b".into(),
        "cand-b".into(),
        "app-b".into(),
        "hash-b".into(),
        "out-b".into(),
    );
    store.record_lineage(&id_b, link_b).await.unwrap();
    store
        .complete_experiment(&id_b, make_outcome(ExperimentDecision::Rollback))
        .await
        .unwrap();

    // Verify A's lineage only has A's data
    let links_a = store.get_lineage(&id_a).await.unwrap();
    assert_eq!(links_a.len(), 1);
    assert_eq!(links_a[0].problem_id, "prob-a");

    // Verify B's lineage only has B's data
    let links_b = store.get_lineage(&id_b).await.unwrap();
    assert_eq!(links_b.len(), 1);
    assert_eq!(links_b[0].problem_id, "prob-b");

    // Verify outcomes are per-experiment
    let out_a = store.get_outcome(&id_a).await.unwrap().unwrap();
    assert_eq!(out_a.decision, ExperimentDecision::Promote);
    let out_b = store.get_outcome(&id_b).await.unwrap().unwrap();
    assert_eq!(out_b.decision, ExperimentDecision::Rollback);
}

#[tokio::test]
async fn lineage_link_serialization_roundtrip() {
    let link = LineageLink::new(
        "p1".into(),
        "prop1".into(),
        "mut1".into(),
        "cand1".into(),
        "app1".into(),
        "abc123".into(),
        "out1".into(),
    );
    let json = serde_json::to_string(&link).unwrap();
    let deserialized: LineageLink = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, link);
}

#[tokio::test]
async fn empty_lineage_for_unknown_experiment_returns_empty_vec() {
    let store = JsonlExperimentStore::in_memory();
    let links = store.get_lineage("no-such-experiment").await.unwrap();
    assert!(links.is_empty());
}

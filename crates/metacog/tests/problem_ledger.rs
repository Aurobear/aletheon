//! Problem ledger integration tests — append-only JSONL ledger with projection rebuild.
//!
//! Covers: idempotent events, invalid transitions, occurrence count,
//! restart projection rebuild, and historical evidence preservation.

use metacog::problem::{
    JsonlProblemLedger, ProblemFinding, ProblemLedger, ProblemRecord, ProblemSeverity,
    ProblemState, ProblemTransition,
};

fn make_finding(id: &str, domain: &str, category: &str) -> ProblemFinding {
    ProblemFinding {
        problem_id: id.to_string(),
        category: category.to_string(),
        subtype: "test_subtype".to_string(),
        domain: domain.to_string(),
        subject: "component_a".to_string(),
        severity: ProblemSeverity::Medium,
        confidence_millis: 800,
        observed_at_ms: 100,
        affected_versions: vec!["v1.0".to_string()],
        expected_summary: "expected success".to_string(),
        observed_summary: "observed failure".to_string(),
        failure_signature: format!("failure_{id}"),
        evidence_ids: vec!["ev-1".to_string()],
        rubric_version: 1,
    }
}

fn make_transition(
    problem_id: &str,
    event_id: &str,
    old_state: ProblemState,
    new_state: ProblemState,
) -> ProblemTransition {
    ProblemTransition {
        problem_id: problem_id.to_string(),
        event_id: event_id.to_string(),
        old_state,
        new_state,
        reason: "test transition".to_string(),
        evidence_ids: vec![],
        timestamp_ms: 200,
    }
}

// ---------------------------------------------------------------------------
// Idempotent events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn idempotent_observation_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.jsonl");

    let ledger = JsonlProblemLedger::new(path.clone()).await.unwrap();

    let finding = make_finding("p1", "coding", "correctness");
    ledger.observe(finding.clone()).await.unwrap();

    // Second observation with same ID should be rejected
    let result = ledger.observe(finding).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("already exists"), "got: {err}");
}

// ---------------------------------------------------------------------------
// Invalid transitions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invalid_state_transition_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.jsonl");

    let ledger = JsonlProblemLedger::new(path.clone()).await.unwrap();

    let finding = make_finding("p1", "coding", "correctness");
    ledger.observe(finding).await.unwrap();

    // Try to transition Observed -> Resolved (invalid — must go through Confirmed, Active, Mitigated)
    let transition = make_transition(
        "p1",
        "evt-1",
        ProblemState::Observed,
        ProblemState::Resolved,
    );
    let result = ledger.transition(transition).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Invalid transition") || err.contains("invalid"),
        "got: {err}"
    );
}

#[tokio::test]
async fn transition_with_wrong_current_state_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.jsonl");

    let ledger = JsonlProblemLedger::new(path.clone()).await.unwrap();

    let finding = make_finding("p1", "coding", "correctness");
    ledger.observe(finding).await.unwrap();

    // Problem is Observed, but transition claims it's Confirmed -> Active
    let transition = make_transition("p1", "evt-1", ProblemState::Confirmed, ProblemState::Active);
    let result = ledger.transition(transition).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn valid_lifecycle_transitions_succeed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.jsonl");

    let ledger = JsonlProblemLedger::new(path.clone()).await.unwrap();

    let finding = make_finding("p1", "coding", "correctness");
    ledger.observe(finding).await.unwrap();

    // Observed -> Confirmed
    ledger
        .transition(make_transition(
            "p1",
            "evt-1",
            ProblemState::Observed,
            ProblemState::Confirmed,
        ))
        .await
        .unwrap();

    // Confirmed -> Active
    ledger
        .transition(make_transition(
            "p1",
            "evt-2",
            ProblemState::Confirmed,
            ProblemState::Active,
        ))
        .await
        .unwrap();

    // Active -> Mitigated
    ledger
        .transition(make_transition(
            "p1",
            "evt-3",
            ProblemState::Active,
            ProblemState::Mitigated,
        ))
        .await
        .unwrap();

    // Mitigated -> Resolved
    ledger
        .transition(make_transition(
            "p1",
            "evt-4",
            ProblemState::Mitigated,
            ProblemState::Resolved,
        ))
        .await
        .unwrap();

    let record = ledger.get("p1").await.unwrap().unwrap();
    assert_eq!(record.state, ProblemState::Resolved);
    // Each observation + 4 transitions = 5 occurrences
    assert_eq!(record.occurrence_count, 5);
}

// ---------------------------------------------------------------------------
// Occurrence count
// ---------------------------------------------------------------------------

#[tokio::test]
async fn occurrence_count_increments_on_transitions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.jsonl");

    let ledger = JsonlProblemLedger::new(path.clone()).await.unwrap();

    let finding = make_finding("p1", "coding", "correctness");
    ledger.observe(finding).await.unwrap();

    let record = ledger.get("p1").await.unwrap().unwrap();
    assert_eq!(record.occurrence_count, 1);

    ledger
        .transition(make_transition(
            "p1",
            "evt-1",
            ProblemState::Observed,
            ProblemState::Confirmed,
        ))
        .await
        .unwrap();

    let record = ledger.get("p1").await.unwrap().unwrap();
    assert_eq!(record.occurrence_count, 2);
    assert_eq!(record.state, ProblemState::Confirmed);
}

// ---------------------------------------------------------------------------
// Restart projection rebuild
// ---------------------------------------------------------------------------

#[tokio::test]
async fn restart_rebuilds_projection_from_jsonl() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.jsonl");

    // Create ledger, observe two problems, transition one
    {
        let ledger = JsonlProblemLedger::new(path.clone()).await.unwrap();

        ledger
            .observe(make_finding("p1", "coding", "correctness"))
            .await
            .unwrap();
        ledger
            .observe(make_finding("p2", "robot", "safety"))
            .await
            .unwrap();

        // Transition p1 to Confirmed
        ledger
            .transition(make_transition(
                "p1",
                "evt-1",
                ProblemState::Observed,
                ProblemState::Confirmed,
            ))
            .await
            .unwrap();
    }
    // Ledger is dropped, file persists

    // Re-open and verify all state is rebuilt
    {
        let ledger = JsonlProblemLedger::new(path.clone()).await.unwrap();

        let p1 = ledger.get("p1").await.unwrap().unwrap();
        assert_eq!(p1.problem_id, "p1");
        assert_eq!(p1.domain, "coding");
        assert_eq!(p1.state, ProblemState::Confirmed);
        assert_eq!(p1.occurrence_count, 2);

        let p2 = ledger.get("p2").await.unwrap().unwrap();
        assert_eq!(p2.problem_id, "p2");
        assert_eq!(p2.domain, "robot");
        assert_eq!(p2.category, "safety");
        assert_eq!(p2.state, ProblemState::Observed);
        assert_eq!(p2.occurrence_count, 1);

        // Active should return both (neither is Resolved/Disputed/AcceptedRisk)
        let active = ledger.active().await.unwrap();
        assert_eq!(active.len(), 2);
    }
}

// ---------------------------------------------------------------------------
// Historical evidence preservation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn historical_evidence_preserved_after_regression() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.jsonl");

    let ledger = JsonlProblemLedger::new(path.clone()).await.unwrap();

    let mut finding = make_finding("p1", "coding", "correctness");
    finding.evidence_ids = vec!["ev-original".to_string()];
    ledger.observe(finding).await.unwrap();

    // Lifecycle: Observed -> Confirmed -> Active -> Mitigated -> Resolved -> Regressed
    ledger
        .transition(make_transition(
            "p1",
            "evt-1",
            ProblemState::Observed,
            ProblemState::Confirmed,
        ))
        .await
        .unwrap();

    ledger
        .transition(make_transition(
            "p1",
            "evt-2",
            ProblemState::Confirmed,
            ProblemState::Active,
        ))
        .await
        .unwrap();

    ledger
        .transition(make_transition(
            "p1",
            "evt-3",
            ProblemState::Active,
            ProblemState::Mitigated,
        ))
        .await
        .unwrap();

    ledger
        .transition(make_transition(
            "p1",
            "evt-4",
            ProblemState::Mitigated,
            ProblemState::Resolved,
        ))
        .await
        .unwrap();

    // Now regress
    ledger
        .transition(make_transition(
            "p1",
            "evt-5",
            ProblemState::Resolved,
            ProblemState::Regressed,
        ))
        .await
        .unwrap();

    let record = ledger.get("p1").await.unwrap().unwrap();
    assert_eq!(record.state, ProblemState::Regressed);
    // Original evidence should still be preserved
    assert!(record.evidence_ids.contains(&"ev-original".to_string()));
    // Occurrence count: 1 observe + 5 transitions = 6
    assert_eq!(record.occurrence_count, 6);
}

// ---------------------------------------------------------------------------
// Active filter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn active_excludes_resolved_and_disputed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.jsonl");

    let ledger = JsonlProblemLedger::new(path.clone()).await.unwrap();

    ledger
        .observe(make_finding("p1", "coding", "correctness"))
        .await
        .unwrap();
    ledger
        .observe(make_finding("p2", "robot", "safety"))
        .await
        .unwrap();

    // Transition p1 to Disputed
    ledger
        .transition(make_transition(
            "p1",
            "evt-1",
            ProblemState::Observed,
            ProblemState::Disputed,
        ))
        .await
        .unwrap();

    let active = ledger.active().await.unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].problem_id, "p2");
}

#[tokio::test]
async fn transition_nonexistent_problem_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.jsonl");

    let ledger = JsonlProblemLedger::new(path.clone()).await.unwrap();

    let transition = make_transition(
        "nonexistent",
        "evt-1",
        ProblemState::Observed,
        ProblemState::Confirmed,
    );
    let result = ledger.transition(transition).await;
    assert!(result.is_err());
}

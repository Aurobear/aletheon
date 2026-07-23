//! End-to-end governed learning flow — synthetic domain.
//!
//! Builds the full metacognition loop using only in-memory stores:
//!
//!   1. append authoritative evidence
//!   2. ingest an experience
//!   3. evaluate it
//!   4. record and confirm a problem
//!   5. reflect and propose an improvement
//!   6. prove promotion fails before approval
//!   7. approve through authority boundary (mock)
//!   8. generate and sandbox a candidate
//!   9. record degraded candidate result
//!  10. choose rollback
//!  11. rebuild all stores after restart
//!
//! No Coding- or Robot-specific types are imported.

use metacog::evidence::integrity;
use metacog::evidence::model::{
    EvidenceId, EvidenceItem, EvidenceKind, EvidenceTrust, ExperienceId,
};
use metacog::evidence::store::{AppendOutcome, EvidenceStore, JsonlEvidenceStore};
use metacog::evolution::experiment::{
    decide_experiment, EvaluationReport as EvalReport, EvolutionExperiment, ExperimentDecision,
    ExperimentOutcome, GateResult, ProblemRecord, ProblemSeverity, ProblemState,
};
use metacog::evolution::experiment_store::{ExperimentStore, JsonlExperimentStore};
use metacog::evolution::LineageLink;
use metacog::experience::model::{
    DomainId, ExperienceEnvelope, ExperienceOutcome, SubjectId, METACOGNITION_SCHEMA_V1,
};
use metacog::improvement::model::{ImprovementProposal, ProposalId, ProposalState};

// ---------------------------------------------------------------------------
// Synthetic helper types — local shadows of types that will exist in
// completed Phases 3-4.  These avoid coupling to unfinished modules.
// ---------------------------------------------------------------------------

/// A minimal problem ledger for the synthetic test.
struct SyntheticProblemLedger {
    problems: std::sync::Mutex<Vec<ProblemRecord>>,
}

impl SyntheticProblemLedger {
    fn new() -> Self {
        Self {
            problems: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn observe(&self, record: ProblemRecord) {
        let mut problems = self.problems.lock().unwrap();
        problems.push(record);
    }

    fn get(&self, id: &str) -> Option<ProblemRecord> {
        let problems = self.problems.lock().unwrap();
        problems.iter().find(|p| p.problem_id == id).cloned()
    }

    fn all_active(&self) -> Vec<ProblemRecord> {
        let problems = self.problems.lock().unwrap();
        problems
            .iter()
            .filter(|p| p.state == ProblemState::Confirmed || p.state == ProblemState::Active)
            .cloned()
            .collect()
    }
}

/// A minimal improvement registry using in-memory storage.
struct SyntheticRegistry {
    proposals: std::sync::Mutex<Vec<ImprovementProposal>>,
}

impl SyntheticRegistry {
    fn new() -> Self {
        Self {
            proposals: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn propose(&self, proposal: ImprovementProposal) {
        let mut proposals = self.proposals.lock().unwrap();
        proposals.push(proposal);
    }

    fn get(&self, id: &ProposalId) -> Option<ImprovementProposal> {
        let proposals = self.proposals.lock().unwrap();
        proposals.iter().find(|p| &p.id == id).cloned()
    }

    fn transition(&self, id: &ProposalId, new_state: ProposalState) -> Option<ImprovementProposal> {
        let mut proposals = self.proposals.lock().unwrap();
        if let Some(p) = proposals.iter_mut().find(|p| &p.id == id) {
            p.state = new_state;
            Some(p.clone())
        } else {
            None
        }
    }
}

/// Verify that promoting an unapproved proposal fails.
fn try_promote_unapproved(registry: &SyntheticRegistry, proposal_id: &ProposalId) -> bool {
    let p = registry.get(proposal_id);
    if let Some(p) = p {
        p.state == ProposalState::Accepted
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// The full governed learning flow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn governed_learning_end_to_end() {
    // Step 1: Append authoritative evidence
    let evidence_store = JsonlEvidenceStore::in_memory();
    let ev = make_evidence(
        "ev-001",
        "exp-flow-001",
        EvidenceTrust::Authoritative,
        serde_json::json!({"latency_ms": 1200, "correctness": "pass", "safety_check": "pass"}),
    );
    let outcome = evidence_store.append(ev.clone()).await.unwrap();
    assert_eq!(outcome, AppendOutcome::Appended);

    let ev2 = make_evidence(
        "ev-002",
        "exp-flow-001",
        EvidenceTrust::Authoritative,
        serde_json::json!({"policy_compliance": "violation", "detail": "unauthorized mutation path"}),
    );
    evidence_store.append(ev2.clone()).await.unwrap();

    // Step 2: Ingest an experience
    let envelope = make_experience_envelope(
        "exp-flow-001",
        "synthetic",
        "tool.config",
        ExperienceOutcome::Failed,
        vec![EvidenceId("ev-001".into()), EvidenceId("ev-002".into())],
    );

    // Verify evidence exists for the experience
    let listed = evidence_store
        .list_for_experience(&envelope.experience_id)
        .await
        .unwrap();
    assert_eq!(listed.len(), 2, "both evidence items should be available");

    // Step 3: Evaluate the experience
    let baseline_report = EvalReport {
        weighted_total_millis: Some(72_000),
        eligible: true,
        gates: vec![
            GateResult {
                name: "safety_boundary".into(),
                passed: true,
            },
            GateResult {
                name: "policy_review".into(),
                passed: true,
            },
        ],
        evidence_coverage_millis: 800,
        confidence_millis: 850,
    };
    let degraded_report = EvalReport {
        weighted_total_millis: Some(58_000),
        eligible: false,
        gates: vec![
            GateResult {
                name: "safety_boundary".into(),
                passed: true,
            },
            GateResult {
                name: "policy_review".into(),
                passed: false,
            },
        ],
        evidence_coverage_millis: 750,
        confidence_millis: 700,
    };

    // Step 4: Record and confirm a problem
    let problem_ledger = SyntheticProblemLedger::new();
    let problem = ProblemRecord {
        problem_id: "prob-001".into(),
        description: "Policy compliance failure on unauthorized mutation path".into(),
        severity: ProblemSeverity::High,
        state: ProblemState::Confirmed,
    };
    problem_ledger.observe(problem.clone());

    let retrieved = problem_ledger.get("prob-001").unwrap();
    assert_eq!(retrieved.severity, ProblemSeverity::High);
    assert_eq!(retrieved.state, ProblemState::Confirmed);

    // Step 5: Reflect and propose an improvement
    let registry = SyntheticRegistry::new();
    let proposal = ImprovementProposal {
        id: ProposalId("prop-001".into()),
        proposer: "metacog".into(),
        target_capability: "self_field.mutation_paths".into(),
        problem_ids: vec!["prob-001".into()],
        proposed_change: "Restrict mutation path to approved governors only".into(),
        expected_benefit: "Eliminate policy compliance failures".into(),
        possible_regressions: vec!["May slow down approved mutation rate".into()],
        validation_plan: "Sandbox test with audit log verification".into(),
        rollback_plan: "Restore previous mutation path configuration".into(),
        authority_requirements: vec!["governor".into()],
        reversible: true,
        expires_at_ms: i64::MAX,
        state: ProposalState::Proposed,
    };
    registry.propose(proposal.clone());

    // Step 6: Prove promotion fails before approval
    assert!(
        !try_promote_unapproved(&registry, &proposal.id),
        "unapproved proposal must not be promoted"
    );

    // Step 7: Approve through authority boundary (mock governor)
    registry.transition(&proposal.id, ProposalState::PendingApproval);
    registry.transition(&proposal.id, ProposalState::Accepted);
    let approved = registry.get(&proposal.id).unwrap();
    assert_eq!(approved.state, ProposalState::Accepted);
    assert!(try_promote_unapproved(&registry, &proposal.id));

    // Step 8: Generate and sandbox a candidate (simulated)

    // Step 9: Record degraded candidate result
    let experiment = EvolutionExperiment {
        baseline_version: "v1.0.0".into(),
        candidate_version: "v1.1.0".into(),
        target_problem_ids: vec!["prob-001".into()],
        baseline_score_distribution: vec![72.0, 75.0, 78.0],
        success_threshold: 5_000,
        rollback_threshold: 3_000,
        observation_window_ms: 60_000,
    };

    let outcome = decide_experiment(&[baseline_report], &[degraded_report], &experiment);

    // Step 10: Choose rollback (non-safety gate failure with large regression)
    assert_eq!(
        outcome.decision,
        ExperimentDecision::Rollback,
        "policy gate failure with significant score drop should produce Rollback"
    );

    // Now test the rollback path specifically
    let regression_report = EvalReport {
        weighted_total_millis: Some(80_000),
        eligible: true,
        gates: vec![GateResult {
            name: "safety_boundary".into(),
            passed: false,
        }],
        evidence_coverage_millis: 600,
        confidence_millis: 500,
    };
    let rollback_outcome = decide_experiment(
        &[EvalReport {
            weighted_total_millis: Some(75_000),
            eligible: true,
            gates: vec![GateResult {
                name: "safety_boundary".into(),
                passed: true,
            }],
            evidence_coverage_millis: 800,
            confidence_millis: 800,
        }],
        &[regression_report],
        &experiment,
    );
    assert_eq!(
        rollback_outcome.decision,
        ExperimentDecision::Rollback,
        "safety gate failure must force rollback"
    );

    // Step 11: Rebuild all stores after restart
    let tmp_evidence = tempfile::NamedTempFile::new().unwrap();
    let evidence_path = tmp_evidence.path().with_extension("jsonl");

    let tmp_experiment = tempfile::NamedTempFile::new().unwrap();
    let experiment_path = tmp_experiment.path().with_extension("jsonl");

    let experiment_id;
    {
        // Start with persistent stores
        let ev_store = JsonlEvidenceStore::open(evidence_path.clone()).unwrap();
        ev_store.append(ev.clone()).await.unwrap();
        ev_store.append(ev2.clone()).await.unwrap();

        let exp_store = JsonlExperimentStore::open(experiment_path.clone()).unwrap();
        let exp = EvolutionExperiment {
            baseline_version: "v1.0.0".into(),
            candidate_version: "v1.2.0".into(),
            target_problem_ids: vec!["prob-001".into()],
            baseline_score_distribution: vec![72.0, 75.0],
            success_threshold: 5_000,
            rollback_threshold: 3_000,
            observation_window_ms: 60_000,
        };
        experiment_id = exp_store.start_experiment(exp).await.unwrap();

        let link = LineageLink::new(
            "prob-001".into(),
            "prop-001".into(),
            "mut-001".into(),
            "cand-001".into(),
            "app-001".into(),
            "sha256:candidate-eval-hash".into(),
            "outcome-001".into(),
        );
        exp_store
            .record_lineage(&experiment_id, link)
            .await
            .unwrap();

        let promote_outcome = ExperimentOutcome {
            pre_reports: vec![],
            post_reports: vec![],
            regressions: vec![],
            new_problems: vec![],
            decision: ExperimentDecision::Promote,
        };
        exp_store
            .complete_experiment(&experiment_id, promote_outcome)
            .await
            .unwrap();
    }

    // --- RESTART --- all stores rebuilt from files
    {
        let ev_store = JsonlEvidenceStore::open(evidence_path.clone()).unwrap();
        let items = ev_store
            .list_for_experience(&ExperienceId("exp-flow-001".into()))
            .await
            .unwrap();
        assert_eq!(items.len(), 2, "evidence should survive restart");

        let exp_store = JsonlExperimentStore::open(experiment_path.clone()).unwrap();
        let reloaded = exp_store
            .get_experiment(&experiment_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reloaded.baseline_version, "v1.0.0");
        assert_eq!(reloaded.candidate_version, "v1.2.0");

        let out = exp_store
            .get_outcome(&experiment_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(out.decision, ExperimentDecision::Promote);

        let links = exp_store.get_lineage(&experiment_id).await.unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].problem_id, "prob-001");
        assert_eq!(links[0].proposal_id, "prop-001");
        assert_eq!(links[0].mutation_id, "mut-001");
        assert_eq!(links[0].candidate_id, "cand-001");
        assert_eq!(links[0].approval_id, "app-001");
        assert_eq!(links[0].evaluation_hash, "sha256:candidate-eval-hash");
        assert_eq!(links[0].outcome_id, "outcome-001");
    }
}

#[tokio::test]
async fn evidence_integrity_check_rejects_tampered_payload() {
    let store = JsonlEvidenceStore::in_memory();
    let mut ev = make_evidence(
        "ev-tamper",
        "exp-tamper",
        EvidenceTrust::Authoritative,
        serde_json::json!({"key": "original"}),
    );
    // Tamper the SHA-256 digest
    ev.sha256 = "0000000000000000000000000000000000000000000000000000000000000000".into();
    let result = store.append(ev).await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_evidence(
    id: &str,
    exp_id: &str,
    trust: EvidenceTrust,
    payload: serde_json::Value,
) -> EvidenceItem {
    let sha256 = integrity::compute_digest(&payload);

    EvidenceItem {
        schema_version: METACOGNITION_SCHEMA_V1,
        evidence_id: EvidenceId(id.into()),
        experience_id: ExperienceId(exp_id.into()),
        kind: EvidenceKind::VerificationResult,
        source: "synthetic-test".into(),
        producer: "governed_learning_flow".into(),
        captured_at_ms: 1000,
        payload,
        sha256,
        trust,
        freshness_ms: Some(5000),
        redacted: false,
    }
}

fn make_experience_envelope(
    id: &str,
    domain: &str,
    subject: &str,
    outcome: ExperienceOutcome,
    evidence: Vec<EvidenceId>,
) -> ExperienceEnvelope {
    ExperienceEnvelope {
        schema_version: METACOGNITION_SCHEMA_V1,
        experience_id: ExperienceId(id.into()),
        domain: DomainId::new(domain).unwrap(),
        subject: SubjectId(subject.into()),
        goal_ref: Some("test-goal".into()),
        started_at_ms: 100,
        completed_at_ms: Some(200),
        outcome,
        correlations: std::collections::BTreeMap::new(),
        evidence,
    }
}

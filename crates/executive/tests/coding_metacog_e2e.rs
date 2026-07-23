//! Coding metacognition E2E test — controlled Coding benchmark through Metacog.
//!
//! Exercises the full flow:
//! 1. Capture coding evidence through the adapter
//! 2. Persist evidence in an in-memory store
//! 3. Calculate evaluation with the coding rubric
//! 4. Create a confirmed problem record
//! 5. Create an unapproved improvement proposal
//! 6. Prove no mutation occurs (proposal stays pending, promotion rejected)
//! 7. Candidate comparison: replay baseline, verify Promote only with thresholds

use std::path::PathBuf;
use std::sync::Arc;

use executive::application::coding_metacog_adapter::{
    CodingEvidenceAdapter, DefaultCodingEvidenceAdapter,
};
use executive::application::coding_metacog_rubric::{coding_rubric_v1, CODING_RUBRIC_V1};

use fabric::types::coding_job::{
    ChangedFile, ChangedFileKind, CodingJobId, CodingJobReport, CodingJobStatus, VerificationCheck,
    VerificationReport, VerificationSeverity,
};
use fabric::types::metacognition_evaluation::{DimensionValue, RubricId};
use fabric::types::metacognition_experience::ExperienceOutcome;

use metacog::evidence::store::{AppendOutcome, JsonlEvidenceStore};
use metacog::evidence::EvidenceStore;
use metacog::experience::ingest::{ExperienceIngestor, InMemoryExperienceStore};
use metacog::improvement::{
    DeterministicProposalPromoter, ImprovementProposal, ImprovementRegistry,
    InMemoryImprovementRegistry, ProposalDecision, ProposalId, ProposalPromoter, ProposalState,
};
use metacog::problem::ledger::{JsonlProblemLedger, ProblemFinding, ProblemLedger};
use metacog::problem::model::{ProblemSeverity, ProblemState, ProblemTransition};

// ---------------------------------------------------------------------------
// Helpers — construct Coding test data
// ---------------------------------------------------------------------------

fn make_success_report(job_id: uuid::Uuid) -> CodingJobReport {
    CodingJobReport {
        job_id: CodingJobId(job_id),
        goal_id: fabric::GoalId(1),
        attempt_id: fabric::AttemptId::new(),
        base_commit: "abc1234".to_string(),
        status: CodingJobStatus::Succeeded,
        exit_code: Some(0),
        elapsed_ms: 5_000,
        stdout: "test output".to_string(),
        stderr: "".to_string(),
        stdout_truncated: false,
        stderr_truncated: false,
        changed_files: vec![ChangedFile {
            path: PathBuf::from("src/lib.rs"),
            kind: ChangedFileKind::Modified,
            before_bytes: 120,
            after_bytes: 125,
            content_sha256: "aaa".to_string(),
        }],
        diff_sha256: Some("diff1".to_string()),
        diff_artifact: Some(PathBuf::from("/tmp/diff.patch")),
    }
}

fn make_success_verification(job_id: uuid::Uuid) -> VerificationReport {
    VerificationReport {
        job_id: CodingJobId(job_id),
        goal_id: fabric::GoalId(1),
        attempt_id: fabric::AttemptId::new(),
        passed: true,
        checks: vec![
            VerificationCheck {
                name: "cargo-test".to_string(),
                severity: VerificationSeverity::Required,
                passed: true,
                timed_out: false,
                cancelled: false,
                summary: "all 3 tests pass".to_string(),
                evidence: vec!["test output".to_string()],
            },
            VerificationCheck {
                name: "forbidden-paths".to_string(),
                severity: VerificationSeverity::Required,
                passed: true,
                timed_out: false,
                cancelled: false,
                summary: "scope clean".to_string(),
                evidence: vec![],
            },
        ],
        risk_summary: vec!["no regression risk".to_string()],
        started_at_ms: 1_000,
        ended_at_ms: 8_000,
    }
}

fn make_failed_report(job_id: uuid::Uuid) -> CodingJobReport {
    CodingJobReport {
        job_id: CodingJobId(job_id),
        goal_id: fabric::GoalId(1),
        attempt_id: fabric::AttemptId::new(),
        base_commit: "abc1234".to_string(),
        status: CodingJobStatus::Failed,
        exit_code: Some(1),
        elapsed_ms: 3_000,
        stdout: "".to_string(),
        stderr: "compilation error".to_string(),
        stdout_truncated: false,
        stderr_truncated: false,
        changed_files: vec![ChangedFile {
            path: PathBuf::from("src/lib.rs"),
            kind: ChangedFileKind::Modified,
            before_bytes: 120,
            after_bytes: 130,
            content_sha256: "bbb".to_string(),
        }],
        diff_sha256: Some("diff2".to_string()),
        diff_artifact: Some(PathBuf::from("/tmp/diff2.patch")),
    }
}

fn make_failed_verification(job_id: uuid::Uuid) -> VerificationReport {
    VerificationReport {
        job_id: CodingJobId(job_id),
        goal_id: fabric::GoalId(1),
        attempt_id: fabric::AttemptId::new(),
        passed: false,
        checks: vec![VerificationCheck {
            name: "cargo-test".to_string(),
            severity: VerificationSeverity::Required,
            passed: false,
            timed_out: false,
            cancelled: false,
            summary: "1 test failed".to_string(),
            evidence: vec![],
        }],
        risk_summary: vec!["compilation error causes regression".to_string()],
        started_at_ms: 1_000,
        ended_at_ms: 5_000,
    }
}

// ---------------------------------------------------------------------------
// Compute a simple evaluation report from evidence using the coding rubric.
// ---------------------------------------------------------------------------

fn evaluate_with_rubric(
    _evidence_count: usize,
    verification_passed: bool,
    has_diff: bool,
    has_verification_checks: bool,
) -> fabric::types::metacognition_evaluation::EvaluationReport {
    let rubric = coding_rubric_v1();
    let mut report = rubric.build_empty_report();

    // Only score dimensions when we have substantive evidence
    // A completion-only message without verification or file evidence
    // produces Unknown dimensions (low coverage).
    if has_verification_checks || has_diff {
        // requirement_coverage: scorable if we have assertions/goal info
        if let Some(dim) = report
            .dimensions
            .iter_mut()
            .find(|d| d.name == "requirement_coverage")
        {
            dim.value = DimensionValue::Scored(80);
            dim.reasons.push("goal and base commit present".to_string());
        }

        // correctness: depends on verification checks
        if let Some(dim) = report
            .dimensions
            .iter_mut()
            .find(|d| d.name == "correctness")
        {
            if verification_passed {
                dim.value = DimensionValue::Scored(100);
                dim.reasons.push("all verification checks pass".to_string());
            } else {
                dim.value = DimensionValue::Scored(30);
                dim.reasons.push("verification checks failed".to_string());
            }
        }

        // scope_discipline: scorable if we have file-level evidence
        if let Some(dim) = report
            .dimensions
            .iter_mut()
            .find(|d| d.name == "scope_discipline")
        {
            dim.value = DimensionValue::Scored(90);
            dim.reasons.push("changed files tracked".to_string());
        }

        // maintainability: scorable if we have diff evidence
        if let Some(dim) = report
            .dimensions
            .iter_mut()
            .find(|d| d.name == "maintainability")
        {
            if has_diff {
                dim.value = DimensionValue::Scored(70);
                dim.reasons.push("diff available for review".to_string());
            }
        }

        // verification_sufficiency: scorable if we have verification evidence
        if let Some(dim) = report
            .dimensions
            .iter_mut()
            .find(|d| d.name == "verification_sufficiency")
        {
            if verification_passed {
                dim.value = DimensionValue::Scored(85);
                dim.reasons.push("verification checks passed".to_string());
            } else {
                dim.value = DimensionValue::Scored(20);
                dim.reasons.push("insufficient verification".to_string());
            }
        }

        // regression_risk: scorable if we have diff and verification
        if let Some(dim) = report
            .dimensions
            .iter_mut()
            .find(|d| d.name == "regression_risk")
        {
            if verification_passed && has_diff {
                dim.value = DimensionValue::Scored(75);
                dim.reasons
                    .push("tests pass, regression risks low".to_string());
            } else if !verification_passed {
                dim.value = DimensionValue::Scored(10);
                dim.reasons
                    .push("high regression risk due to test failures".to_string());
            }
        }
    }

    // Gates
    for gate in &mut report.gates {
        match gate.name.as_str() {
            "verification_evidence" => {
                let has_verification_score = !report
                    .dimensions
                    .iter()
                    .filter(|d| d.name == "verification_sufficiency" || d.name == "correctness")
                    .all(|d| matches!(d.value, DimensionValue::Unknown));
                gate.passed = has_verification_score && verification_passed;
                if !gate.passed {
                    gate.evidence
                        .push(fabric::types::metacognition_evidence::EvidenceId(
                            "ev-gate-v".to_string(),
                        ));
                }
            }
            "change_within_scope" => {
                gate.passed = true; // always passes in fixture
            }
            _ => {}
        }
    }

    // Calculate weighted total from scored dimensions
    let rubric = coding_rubric_v1();
    let mut total_weighted_score = 0u64;
    let mut applicable_weight = 0u64;
    for dim in &report.dimensions {
        if let DimensionValue::Scored(score) = dim.value {
            total_weighted_score += score as u64 * dim.weight_millis as u64;
            applicable_weight += dim.weight_millis as u64;
        }
    }
    if applicable_weight > 0 {
        report.weighted_total_millis =
            Some(((total_weighted_score / applicable_weight) * 1_000) as u32);
    }

    // Evidence coverage: count dimensions with evidence vs total dimensions
    let scored_count = report
        .dimensions
        .iter()
        .filter(|d| matches!(d.value, DimensionValue::Scored(_)))
        .count();
    let total_dims = report.dimensions.len();
    if total_dims > 0 {
        report.evidence_coverage_millis =
            ((scored_count as f64 / total_dims as f64) * 1_000.0) as u16;
    }

    // Confidence: lower for missing dimensions
    report.confidence_millis = match scored_count {
        0 => 0,
        n if n < total_dims => 500,
        _ => 950,
    };

    // Eligible: all gates pass + weighted total exists + coverage above threshold
    report.eligible = report.gates.iter().all(|g| g.passed)
        && report.weighted_total_millis.is_some()
        && report.evidence_coverage_millis >= rubric.min_evidence_coverage_millis;

    report
}

// ---------------------------------------------------------------------------
// Test: Full E2E flow
// ---------------------------------------------------------------------------

/// Step 1-6: Capture evidence, persist, evaluate, create problem,
/// create unapproved proposal, prove no mutation occurs.
#[tokio::test]
async fn full_coding_metacog_e2e_flow() {
    let adapter = DefaultCodingEvidenceAdapter;

    // --- 1. Capture evidence through adapter ---
    let job_id = uuid::uuid!("b1b2b3b4-c5c6-d7d8-e9e0-f1f2f3f4f5f6");
    let report = make_success_report(job_id);
    let verification = make_success_verification(job_id);
    let captured = adapter.capture(&report, &verification).unwrap();

    assert_eq!(captured.envelope.outcome, ExperienceOutcome::Succeeded);
    assert_eq!(captured.envelope.domain.as_str(), "coding");
    assert!(
        captured.evidence.len() >= 5,
        "expected at least 5 evidence items, got {}",
        captured.evidence.len()
    );

    // --- 2. Persist evidence in in-memory store ---
    let evidence_store = Arc::new(JsonlEvidenceStore::in_memory());
    let mut evidence_ids = Vec::new();
    for ev in &captured.evidence {
        let outcome = evidence_store.append(ev.clone()).await.unwrap();
        assert!(
            matches!(outcome, AppendOutcome::Appended),
            "evidence should be appended fresh"
        );
        evidence_ids.push(ev.evidence_id.clone());
    }

    // Verify idempotent append
    let outcome = evidence_store
        .append(captured.evidence[0].clone())
        .await
        .unwrap();
    assert_eq!(outcome, AppendOutcome::AlreadyPresent);

    // --- 2b. Persist experience envelope ---
    let exp_store = InMemoryExperienceStore::new();
    let ingestor = ExperienceIngestor::new(evidence_store.clone());
    let ingest_outcome = ingestor
        .ingest(&captured.envelope, &exp_store)
        .await
        .unwrap();
    assert!(matches!(ingest_outcome, AppendOutcome::Appended));

    // --- 3. Calculate evaluation report with coding rubric ---
    let evaluation = evaluate_with_rubric(captured.evidence.len(), verification.passed, true, true);

    assert_eq!(evaluation.rubric, RubricId("coding-v1".to_string()));
    assert_eq!(evaluation.rubric_version, CODING_RUBRIC_V1);
    // Successful job should be eligible
    assert!(evaluation.eligible);

    // All gates must pass
    assert!(evaluation.gates.iter().all(|g| g.passed));

    // Weighted total must exist
    assert!(evaluation.weighted_total_millis.is_some());

    // Evidence coverage must be above threshold
    assert!(evaluation.evidence_coverage_millis >= 400);

    // --- 4. Create one confirmed problem record ---
    let temp_dir = tempfile::TempDir::new().unwrap();
    let ledger_path = temp_dir.path().join("problems.jsonl");
    let ledger = JsonlProblemLedger::new(ledger_path).await.unwrap();

    // Observe a problem
    let finding = ProblemFinding {
        problem_id: "prob-coding-coverage-1".to_string(),
        category: "verification_sufficiency".to_string(),
        subtype: "low_test_coverage".to_string(),
        domain: "coding".to_string(),
        subject: "rust_bugfix".to_string(),
        severity: ProblemSeverity::Medium,
        confidence_millis: 800,
        observed_at_ms: 1_000_000,
        affected_versions: vec!["v1".to_string()],
        expected_summary: "all functions should have test coverage".to_string(),
        observed_summary: "one key function lacks test coverage".to_string(),
        failure_signature: "missing_tests".to_string(),
        evidence_ids: vec!["ev-verify-test".to_string()],
        rubric_version: CODING_RUBRIC_V1,
    };
    ledger.observe(finding).await.unwrap();

    // Confirm the problem (Observed -> Confirmed)
    let transition = ProblemTransition {
        problem_id: "prob-coding-coverage-1".to_string(),
        event_id: "evt-1".to_string(),
        old_state: ProblemState::Observed,
        new_state: ProblemState::Confirmed,
        reason: "verified with evidence".to_string(),
        evidence_ids: vec!["ev-verify-test".to_string()],
        timestamp_ms: 1_000_100,
    };
    ledger.transition(transition).await.unwrap();

    // Retrieve confirmed problem
    let record = ledger
        .get("prob-coding-coverage-1")
        .await
        .unwrap()
        .expect("problem record should exist");
    assert_eq!(record.state, ProblemState::Confirmed);
    assert_eq!(record.category, "verification_sufficiency");

    // --- 5. Create an unapproved improvement proposal ---
    let registry = InMemoryImprovementRegistry::new();
    let proposal = ImprovementProposal {
        id: ProposalId("prop-coding-1".to_string()),
        proposer: "metacog".to_string(),
        target_capability: "tool.config".to_string(),
        problem_ids: vec!["prob-coding-coverage-1".to_string()],
        proposed_change: "increase test coverage requirements in coding pipeline".to_string(),
        expected_benefit: "reduce untested code paths".to_string(),
        possible_regressions: vec!["longer CI times".to_string()],
        validation_plan: "run test suite in sandbox".to_string(),
        rollback_plan: "revert to previous pipeline config".to_string(),
        authority_requirements: vec!["governor".to_string()],
        reversible: true,
        expires_at_ms: i64::MAX,
        state: ProposalState::Proposed,
    };
    let prop_id = proposal.id.clone();
    registry.propose(proposal).await.unwrap();

    // Verify proposal is in Proposed state
    let retrieved = registry.accepted(&prop_id).await;
    // Should fail because not yet Accepted
    assert!(retrieved.is_err());

    // --- 6. Prove no mutation occurs — promotion rejected for unapproved ---
    let promoter = DeterministicProposalPromoter;
    let unapproved = ImprovementProposal {
        id: prop_id.clone(),
        proposer: "metacog".to_string(),
        target_capability: "tool.config".to_string(),
        problem_ids: vec!["prob-coding-coverage-1".to_string()],
        proposed_change: "increase test coverage".to_string(),
        expected_benefit: "reduce untested code".to_string(),
        possible_regressions: vec![],
        validation_plan: "sandbox test".to_string(),
        rollback_plan: "revert".to_string(),
        authority_requirements: vec![],
        reversible: true,
        expires_at_ms: i64::MAX,
        state: ProposalState::Proposed,
    };

    let promotion_result = promoter.promote(&unapproved, 1_000_000);
    assert!(
        promotion_result.is_err(),
        "promotion should fail for Proposed (not Accepted) proposal"
    );
    let err_msg = promotion_result.unwrap_err().to_string();
    assert!(
        err_msg.contains("NotAccepted") || err_msg.contains("not in Accepted"),
        "error should mention not accepted: {err_msg}"
    );

    // Also verify evidence-free proposals are rejected
    let no_evidence = ImprovementProposal {
        id: ProposalId("prop-no-evidence".to_string()),
        proposer: "metacog".to_string(),
        target_capability: "tool.config".to_string(),
        problem_ids: vec![], // empty
        proposed_change: "some change".to_string(),
        expected_benefit: "some benefit".to_string(),
        possible_regressions: vec![],
        validation_plan: "test".to_string(),
        rollback_plan: "revert".to_string(),
        authority_requirements: vec![],
        reversible: true,
        expires_at_ms: i64::MAX,
        state: ProposalState::Accepted,
    };
    let evidence_result = promoter.promote(&no_evidence, 1_000_000);
    assert!(evidence_result.is_err());
    assert!(
        evidence_result
            .unwrap_err()
            .to_string()
            .contains("empty problem_ids"),
        "empty problem_ids should be rejected as evidence-free"
    );

    // Prove promotion succeeds after proper approval + non-empty evidence
    // First submit and accept through registry
    registry
        .decide(&prop_id, ProposalDecision::Submit)
        .await
        .unwrap();
    registry
        .decide(
            &prop_id,
            ProposalDecision::Accept {
                principal: "governor".to_string(),
                reason: "improves quality".to_string(),
            },
        )
        .await
        .unwrap();

    let accepted_proposal = registry.accepted(&prop_id).await.unwrap();
    assert_eq!(accepted_proposal.state, ProposalState::Accepted);

    let intent = promoter.promote(&accepted_proposal, 1_000_000).unwrap();
    assert_eq!(intent.target, "tool.config");
    assert!(intent.reversible);
}

// ---------------------------------------------------------------------------
// Test: Candidate comparison
// ---------------------------------------------------------------------------

/// Compare baseline (successful) vs candidate (failed) outcomes.
/// Verify Promote decision only when thresholds and all hard gates pass.
#[tokio::test]
async fn candidate_comparison_promote_only_with_thresholds() {
    let adapter = DefaultCodingEvidenceAdapter;

    // --- Baseline: successful job ---
    let base_id = uuid::uuid!("11111111-2222-3333-4444-555555555555");
    let base_report = make_success_report(base_id);
    let base_verification = make_success_verification(base_id);
    let base_captured = adapter.capture(&base_report, &base_verification).unwrap();
    let base_eval = evaluate_with_rubric(
        base_captured.evidence.len(),
        base_verification.passed,
        true,
        true,
    );

    // Baseline is eligible
    assert!(
        base_eval.eligible,
        "baseline (successful job) should be eligible"
    );
    assert!(base_eval.weighted_total_millis.unwrap() > 0);

    // --- Candidate: failed job ---
    let cand_id = uuid::uuid!("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
    let cand_report = make_failed_report(cand_id);
    let cand_verification = make_failed_verification(cand_id);
    let cand_captured = adapter.capture(&cand_report, &cand_verification).unwrap();
    let cand_eval = evaluate_with_rubric(
        cand_captured.evidence.len(),
        cand_verification.passed,
        true,
        true,
    );

    // Candidate must NOT be eligible
    assert!(
        !cand_eval.eligible,
        "candidate (failed job) should NOT be eligible"
    );

    // Compare scores — baseline must beat candidate
    let base_total = base_eval.weighted_total_millis.unwrap();
    let cand_total = cand_eval.weighted_total_millis.unwrap_or(0);
    assert!(
        base_total > cand_total,
        "baseline score ({base_total}) must exceed candidate ({cand_total})"
    );

    // The Promote decision requires ALL hard gates passing +
    // weighted_total threshold. The candidate fails the gate so
    // it cannot be promoted.
    let gates_pass = cand_eval.gates.iter().all(|g| g.passed);
    assert!(!gates_pass, "candidate should have a failed gate");

    // Simulate the experiment decision table
    enum ExperimentDecision {
        Promote,
        Retain,
        Rollback,
        Reject,
        Inconclusive,
    }

    let decision = if gates_pass
        && cand_eval.eligible
        && cand_eval.weighted_total_millis.unwrap_or(0) > base_total
    {
        ExperimentDecision::Promote
    } else if gates_pass && cand_eval.eligible {
        ExperimentDecision::Retain
    } else if !gates_pass {
        ExperimentDecision::Rollback
    } else {
        ExperimentDecision::Inconclusive
    };

    assert!(
        matches!(decision, ExperimentDecision::Rollback),
        "candidate with failed gate should trigger Rollback"
    );

    // --- Baseline vs better candidate scenario (theoretical) ---
    // Show Promote only when candidate passes all gates AND beats baseline
    let mut better_eval = base_eval.clone();
    // Mark all gates as passing
    for gate in &mut better_eval.gates {
        gate.passed = true;
    }
    better_eval.weighted_total_millis = Some(base_total + 10_000);
    better_eval.eligible = true;
    better_eval.evidence_coverage_millis = 800;

    let better_decision = if better_eval.gates.iter().all(|g| g.passed)
        && better_eval.eligible
        && better_eval.weighted_total_millis.unwrap_or(0) > base_total
    {
        ExperimentDecision::Promote
    } else {
        ExperimentDecision::Retain
    };

    assert!(
        matches!(better_decision, ExperimentDecision::Promote),
        "better candidate passing all gates + higher score should Promote"
    );
}

// ---------------------------------------------------------------------------
// Test: Completion-only message produces low coverage
// ---------------------------------------------------------------------------

/// A coding job with only a completion message (no tool/test evidence) must
/// produce low evidence coverage, not a fabricated high score.
#[tokio::test]
async fn completion_only_message_produces_low_coverage() {
    let adapter = DefaultCodingEvidenceAdapter;

    // Build a minimal report — no changed files, no diff, no verification checks
    let job_id = uuid::uuid!("00000000-0000-0000-0000-000000000000");
    let minimal_report = CodingJobReport {
        job_id: CodingJobId(job_id),
        goal_id: fabric::GoalId(1),
        attempt_id: fabric::AttemptId::new(),
        base_commit: "abc".to_string(),
        status: CodingJobStatus::Succeeded,
        exit_code: Some(0),
        elapsed_ms: 100,
        stdout: "done".to_string(),
        stderr: "".to_string(),
        stdout_truncated: false,
        stderr_truncated: false,
        changed_files: vec![],
        diff_sha256: None,
        diff_artifact: None,
    };
    let minimal_verification = VerificationReport {
        job_id: CodingJobId(job_id),
        goal_id: fabric::GoalId(1),
        attempt_id: fabric::AttemptId::new(),
        passed: true,
        checks: vec![], // no verification checks
        risk_summary: vec![],
        started_at_ms: 0,
        ended_at_ms: 100,
    };

    let captured = adapter
        .capture(&minimal_report, &minimal_verification)
        .unwrap();

    // Persist evidence
    let evidence_store = Arc::new(JsonlEvidenceStore::in_memory());
    for ev in &captured.evidence {
        evidence_store.append(ev.clone()).await.unwrap();
    }

    let eval = evaluate_with_rubric(captured.evidence.len(), true, false, false);

    // With minimal evidence, coverage should be low
    let rubric = coding_rubric_v1();
    assert!(
        eval.evidence_coverage_millis < rubric.min_evidence_coverage_millis,
        "completion-only message must be below evidence coverage threshold of {} (got {})",
        rubric.min_evidence_coverage_millis,
        eval.evidence_coverage_millis
    );
    assert!(
        !eval.eligible,
        "completion-only message should not produce an eligible evaluation"
    );
}

// ---------------------------------------------------------------------------
// Test: Evidence integrity across adapter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn adapter_evidence_passes_store_integrity() {
    let adapter = DefaultCodingEvidenceAdapter;
    let job_id = uuid::uuid!("c1c2c3c4-d5d6-e7e8-f9f0-a1a2a3a4a5a6");
    let report = make_success_report(job_id);
    let verification = make_success_verification(job_id);

    let captured = adapter.capture(&report, &verification).unwrap();

    let store = Arc::new(JsonlEvidenceStore::in_memory());
    let mut count = 0;
    for ev in &captured.evidence {
        let outcome = store.append(ev.clone()).await;
        assert!(
            outcome.is_ok(),
            "evidence should pass store integrity: {:?}",
            outcome.err()
        );
        count += 1;
    }
    assert!(count > 0);
}

// ---------------------------------------------------------------------------
// Test: Proposal stays pending until approved
// ---------------------------------------------------------------------------

#[tokio::test]
async fn proposal_stays_pending_until_approved() {
    let registry = InMemoryImprovementRegistry::new();
    let proposal = ImprovementProposal {
        id: ProposalId("prop-pending".to_string()),
        proposer: "coding_adapter".to_string(),
        target_capability: "coding.pipeline".to_string(),
        problem_ids: vec!["prob-1".to_string()],
        proposed_change: "optimize test execution".to_string(),
        expected_benefit: "faster CI".to_string(),
        possible_regressions: vec![],
        validation_plan: "run tests in sandbox".to_string(),
        rollback_plan: "revert pipeline change".to_string(),
        authority_requirements: vec!["governor".to_string()],
        reversible: true,
        expires_at_ms: i64::MAX,
        state: ProposalState::Proposed,
    };
    let id = proposal.id.clone();
    registry.propose(proposal).await.unwrap();

    // Submit to PendingApproval
    registry
        .decide(&id, ProposalDecision::Submit)
        .await
        .unwrap();

    // At this point the proposal is PendingApproval, not yet accepted
    let promoter = DeterministicProposalPromoter;
    let pending = ImprovementProposal {
        id: id.clone(),
        proposer: "coding_adapter".to_string(),
        target_capability: "coding.pipeline".to_string(),
        problem_ids: vec!["prob-1".to_string()],
        proposed_change: "optimize test execution".to_string(),
        expected_benefit: "faster CI".to_string(),
        possible_regressions: vec![],
        validation_plan: "test".to_string(),
        rollback_plan: "revert".to_string(),
        authority_requirements: vec![],
        reversible: true,
        expires_at_ms: i64::MAX,
        state: ProposalState::PendingApproval,
    };

    // Promotion MUST fail for PendingApproval
    let result = promoter.promote(&pending, 1_000_000);
    assert!(
        result.is_err(),
        "cannot promote a proposal that is PendingApproval"
    );

    // Accept the proposal
    registry
        .decide(
            &id,
            ProposalDecision::Accept {
                principal: "governor".to_string(),
                reason: "reasonable improvement".to_string(),
            },
        )
        .await
        .unwrap();

    // Now accepted — promotion should succeed
    let accepted = registry.accepted(&id).await.unwrap();
    let intent = promoter.promote(&accepted, 1_000_000).unwrap();
    assert_eq!(intent.target, "coding.pipeline");
}

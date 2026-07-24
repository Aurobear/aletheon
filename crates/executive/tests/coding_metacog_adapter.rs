//! Coding evidence adapter tests — verify CodingJobReport fields map to generic EvidenceItems
//! without leaking Coding-private types into Metacog contracts.

use executive::application::coding_metacog_adapter::{
    CodingEvidenceAdapter, DefaultCodingEvidenceAdapter,
};
use fabric::types::coding_job::{
    ChangedFile, ChangedFileKind, CodingJobId, CodingJobReport, CodingJobStatus, VerificationCheck,
    VerificationReport, VerificationSeverity,
};
use fabric::types::metacognition_evidence::EvidenceKind;
use fabric::types::metacognition_experience::ExperienceOutcome;
use sha2::Digest;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_report(status: CodingJobStatus, exit_code: i32) -> CodingJobReport {
    CodingJobReport {
        job_id: CodingJobId(uuid::uuid!("a1a2a3a4-b5b6-c7c8-d9d0-e1e2e3e4e5e6")),
        goal_id: fabric::GoalId(1),
        attempt_id: fabric::AttemptId::new(),
        base_commit: "deadbeef".to_string(),
        status,
        exit_code: Some(exit_code),
        elapsed_ms: 1_200,
        stdout: "build output here".to_string(),
        stderr: "".to_string(),
        stdout_truncated: false,
        stderr_truncated: false,
        changed_files: vec![ChangedFile {
            path: PathBuf::from("src/lib.rs"),
            kind: ChangedFileKind::Modified,
            before_bytes: 256,
            after_bytes: 280,
            content_sha256: "abc123".to_string(),
        }],
        diff_sha256: Some("diff456".to_string()),
        diff_artifact: Some(PathBuf::from("/tmp/diff.patch")),
    }
}

fn make_verification(passed: bool) -> VerificationReport {
    VerificationReport {
        job_id: CodingJobId(uuid::uuid!("a1a2a3a4-b5b6-c7c8-d9d0-e1e2e3e4e5e6")),
        goal_id: fabric::GoalId(1),
        attempt_id: fabric::AttemptId::new(),
        passed,
        checks: vec![
            VerificationCheck {
                name: "cargo-test".to_string(),
                severity: VerificationSeverity::Required,
                passed: true,
                timed_out: false,
                cancelled: false,
                summary: "all tests pass".to_string(),
                evidence: vec!["cargo test output".to_string()],
            },
            VerificationCheck {
                name: "forbidden-paths".to_string(),
                severity: VerificationSeverity::Required,
                passed: true,
                timed_out: false,
                cancelled: false,
                summary: "no forbidden paths changed".to_string(),
                evidence: vec!["path check log".to_string()],
            },
        ],
        risk_summary: vec!["low risk of regression".to_string()],
        started_at_ms: 1_000,
        ended_at_ms: 10_000,
    }
}

// ---------------------------------------------------------------------------
// Core mapping tests
// ---------------------------------------------------------------------------

#[test]
fn maps_successful_job_to_succeeded_outcome() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Succeeded, 0);
    let verification = make_verification(true);

    let captured = adapter.capture(&report, &verification).unwrap();

    assert_eq!(captured.envelope.outcome, ExperienceOutcome::Succeeded);
    assert!(!captured.evidence.is_empty());
}

#[test]
fn maps_failed_job_to_failed_outcome() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Failed, 1);
    let verification = make_verification(false);

    let captured = adapter.capture(&report, &verification).unwrap();

    assert_eq!(captured.envelope.outcome, ExperienceOutcome::Failed);
}

#[test]
fn maps_timeout_to_timed_out_outcome() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::TimedOut, -1);
    let verification = make_verification(false);

    let captured = adapter.capture(&report, &verification).unwrap();

    assert_eq!(captured.envelope.outcome, ExperienceOutcome::TimedOut);
}

#[test]
fn maps_cancelled_to_cancelled_outcome() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Cancelled, -1);
    let verification = make_verification(false);

    let captured = adapter.capture(&report, &verification).unwrap();

    assert_eq!(captured.envelope.outcome, ExperienceOutcome::Cancelled);
}

#[test]
fn domain_is_coding() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Succeeded, 0);
    let verification = make_verification(true);

    let captured = adapter.capture(&report, &verification).unwrap();

    assert_eq!(captured.envelope.domain.as_str(), "coding");
}

// ---------------------------------------------------------------------------
// Evidence mapping tests
// ---------------------------------------------------------------------------

#[test]
fn produces_requirement_reference_evidence() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Succeeded, 0);
    let verification = make_verification(true);

    let captured = adapter.capture(&report, &verification).unwrap();

    let req_ev = captured
        .evidence
        .iter()
        .find(|e| matches!(e.kind, EvidenceKind::Assertion))
        .expect("must have requirement assertion evidence");

    assert_eq!(req_ev.payload["goal_id"], 1);
    assert_eq!(req_ev.payload["base_commit"], "deadbeef");
}

#[test]
fn produces_command_result_evidence() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Succeeded, 0);
    let verification = make_verification(true);

    let captured = adapter.capture(&report, &verification).unwrap();

    let cmd_ev = captured
        .evidence
        .iter()
        .find(|e| matches!(e.kind, EvidenceKind::ActionResult))
        .expect("must have action result evidence");

    assert_eq!(cmd_ev.payload["exit_code"], 0);
    assert_eq!(cmd_ev.payload["elapsed_ms"], 1_200);
}

#[test]
fn produces_changed_file_evidence() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Succeeded, 0);
    let verification = make_verification(true);

    let captured = adapter.capture(&report, &verification).unwrap();

    let file_ev = captured
        .evidence
        .iter()
        .find(|e| matches!(e.kind, EvidenceKind::Observation))
        .expect("must have observation evidence for changed files");

    let files = file_ev.payload["changed_files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "src/lib.rs");
    assert_eq!(files[0]["kind"], "modified");
}

#[test]
fn produces_diff_artifact_evidence() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Succeeded, 0);
    let verification = make_verification(true);

    let captured = adapter.capture(&report, &verification).unwrap();

    let diff_ev = captured
        .evidence
        .iter()
        .find(|e| matches!(e.kind, EvidenceKind::Artifact))
        .expect("must have diff artifact evidence");

    assert_eq!(diff_ev.payload["diff_sha256"], "diff456");
}

#[test]
fn produces_verification_check_evidence() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Succeeded, 0);
    let verification = make_verification(true);

    let captured = adapter.capture(&report, &verification).unwrap();

    let verify_ev: Vec<_> = captured
        .evidence
        .iter()
        .filter(|e| matches!(e.kind, EvidenceKind::VerificationResult))
        .collect();

    assert_eq!(verify_ev.len(), 2);
    assert_eq!(verify_ev[0].payload["name"], "cargo-test");
    assert_eq!(verify_ev[1].payload["name"], "forbidden-paths");
}

#[test]
fn produces_review_finding_evidence() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Succeeded, 0);
    let verification = make_verification(true);

    let captured = adapter.capture(&report, &verification).unwrap();

    let review_ev = captured
        .evidence
        .iter()
        .find(|e| matches!(e.kind, EvidenceKind::PolicyDecision))
        .expect("must have review policy decision evidence");

    let risks = review_ev.payload["risk_summary"].as_array().unwrap();
    assert_eq!(risks.len(), 1);
    assert_eq!(risks[0], "low risk of regression");
}

#[test]
fn evidence_has_sha256_integrity() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Succeeded, 0);
    let verification = make_verification(true);

    let captured = adapter.capture(&report, &verification).unwrap();

    for ev in &captured.evidence {
        assert!(
            !ev.sha256.is_empty(),
            "every evidence item must have sha256"
        );
        // Re-compute and verify
        let bytes = serde_json::to_vec(&ev.payload).unwrap();
        let expected = format!("{:x}", sha2::Sha256::digest(bytes));
        assert_eq!(
            ev.sha256, expected,
            "sha256 mismatch for {}",
            ev.evidence_id.0
        );
    }
}

// ---------------------------------------------------------------------------
// Envelope structure tests
// ---------------------------------------------------------------------------

#[test]
fn envelope_has_schema_version() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Succeeded, 0);
    let verification = make_verification(true);

    let captured = adapter.capture(&report, &verification).unwrap();

    assert_eq!(captured.envelope.schema_version, 1);
}

#[test]
fn envelope_references_all_evidence() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Succeeded, 0);
    let verification = make_verification(true);

    let captured = adapter.capture(&report, &verification).unwrap();

    for ev in &captured.evidence {
        assert!(captured.envelope.evidence.contains(&ev.evidence_id));
    }
    assert_eq!(captured.envelope.evidence.len(), captured.evidence.len());
}

#[test]
fn envelope_correlations_include_task_goal_attempt() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Succeeded, 0);
    let verification = make_verification(true);

    let captured = adapter.capture(&report, &verification).unwrap();

    assert!(captured.envelope.correlations.contains_key("task"));
    assert!(captured.envelope.correlations.contains_key("goal"));
    assert!(captured.envelope.correlations.contains_key("attempt"));
}

// ---------------------------------------------------------------------------
// Negative tests
// ---------------------------------------------------------------------------

#[test]
fn no_diff_evidence_when_diff_sha256_is_none() {
    let adapter = DefaultCodingEvidenceAdapter;
    let mut report = make_report(CodingJobStatus::Succeeded, 0);
    report.diff_sha256 = None;
    let verification = make_verification(true);

    let captured = adapter.capture(&report, &verification).unwrap();

    let diff_ev = captured
        .evidence
        .iter()
        .filter(|e| matches!(e.kind, EvidenceKind::Artifact));
    assert_eq!(diff_ev.count(), 0);
}

#[test]
fn no_review_evidence_when_risk_summary_is_empty() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Succeeded, 0);
    let mut verification = make_verification(true);
    verification.risk_summary = vec![];

    let captured = adapter.capture(&report, &verification).unwrap();

    let review_ev = captured
        .evidence
        .iter()
        .filter(|e| matches!(e.kind, EvidenceKind::PolicyDecision));
    assert_eq!(review_ev.count(), 0);
}

#[test]
fn running_job_maps_to_unknown_outcome() {
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Running, -1);
    let verification = make_verification(false);

    let captured = adapter.capture(&report, &verification).unwrap();

    assert_eq!(captured.envelope.outcome, ExperienceOutcome::Unknown);
}

// ---------------------------------------------------------------------------
// No Coding-private types leak into Metacog contracts
// ---------------------------------------------------------------------------

#[test]
fn captured_experience_uses_only_fabric_contract_types() {
    // This is a compile-time assertion that CapturedExperience and its
    // evidence items use only Fabric metacognition contracts, not
    // CodingJobReport or verification internals.
    let adapter = DefaultCodingEvidenceAdapter;
    let report = make_report(CodingJobStatus::Succeeded, 0);
    let verification = make_verification(true);

    let captured = adapter.capture(&report, &verification).unwrap();

    // All evidence items are EvidenceItem (generic contract type)
    for ev in &captured.evidence {
        let _: &fabric::types::metacognition_evidence::EvidenceItem = ev;
    }

    // The envelope is ExperienceEnvelope (generic contract type)
    let _: &fabric::types::metacognition_experience::ExperienceEnvelope = &captured.envelope;
}

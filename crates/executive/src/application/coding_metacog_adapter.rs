//! Coding evidence adapter — translates Coding-job artifacts into generic Metacog evidence.
//!
//! Maps `fabric::CodingJobReport` and `fabric::VerificationReport` into
//! `CapturedExperience` containing an `ExperienceEnvelope` and `Vec<EvidenceItem>`.
//! This adapter lives in the executive (domain-side) crate, not in Metacog core.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use fabric::types::coding_job::{
    ChangedFileKind, CodingJobReport, CodingJobStatus, VerificationReport,
};
use fabric::types::metacognition_evidence::{
    EvidenceId, EvidenceItem, EvidenceKind, EvidenceTrust,
};
use fabric::types::metacognition_experience::{
    DomainId, ExperienceEnvelope, ExperienceId, ExperienceOutcome, SubjectId,
    METACOGNITION_SCHEMA_V1,
};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A captured experience bundles an envelope with its supporting evidence.
#[derive(Debug, Clone)]
pub struct CapturedExperience {
    pub envelope: ExperienceEnvelope,
    pub evidence: Vec<EvidenceItem>,
}

/// Errors that may occur while capturing coding evidence.
#[derive(Debug, thiserror::Error)]
pub enum CodingEvidenceError {
    #[error("invalid domain identifier: {0}")]
    InvalidDomain(String),
    #[error("evidence serialization failed: {0}")]
    Serialization(String),
}

// ---------------------------------------------------------------------------
// Adapter trait
// ---------------------------------------------------------------------------

/// Translates Coding job artifacts into generic Metacog evidence contracts.
///
/// Implementations live in the executive crate; Metacog must never import
/// `CodingJobReport`, `VerificationReport`, or any other Coding-private type.
pub trait CodingEvidenceAdapter {
    /// Capture evidence from a completed coding job and its verification report.
    fn capture(
        &self,
        report: &CodingJobReport,
        verification: &VerificationReport,
    ) -> Result<CapturedExperience, CodingEvidenceError>;
}

// ---------------------------------------------------------------------------
// Default implementation
// ---------------------------------------------------------------------------

/// Default adapter that maps Coding job fields to generic evidence items.
#[derive(Default)]
pub struct DefaultCodingEvidenceAdapter;

impl CodingEvidenceAdapter for DefaultCodingEvidenceAdapter {
    fn capture(
        &self,
        report: &CodingJobReport,
        verification: &VerificationReport,
    ) -> Result<CapturedExperience, CodingEvidenceError> {
        let domain = DomainId::new("coding")
            .map_err(|e| CodingEvidenceError::InvalidDomain(e.to_string()))?;
        let exp_id = ExperienceId(format!("coding-job-{}", report.job_id.0));
        let subject = SubjectId(format!("job-{}", report.job_id.0));
        let now_ms = system_time_ms();

        let outcome = match report.status {
            CodingJobStatus::Succeeded => ExperienceOutcome::Succeeded,
            CodingJobStatus::Failed => ExperienceOutcome::Failed,
            CodingJobStatus::Cancelled => ExperienceOutcome::Cancelled,
            CodingJobStatus::TimedOut => ExperienceOutcome::TimedOut,
            CodingJobStatus::Running | CodingJobStatus::Retained => ExperienceOutcome::Unknown,
        };

        let mut evidence = Vec::new();

        // 1. Requirement references — the goal and base commit are assertions
        let req_payload = serde_json::json!({
            "goal_id": report.goal_id.0,
            "attempt_id": report.attempt_id.0.to_string(),
            "base_commit": report.base_commit,
        });
        evidence.push(build_evidence(
            &exp_id,
            EvidenceId(format!("ev-req-{}", report.job_id.0)),
            EvidenceKind::Assertion,
            "coding_job_spec",
            "coding_evidence_adapter",
            now_ms,
            req_payload,
            EvidenceTrust::Corroborated,
            None,
        )?);

        // 2. Command result evidence — stdout, stderr, exit code
        let cmd_payload = serde_json::json!({
            "exit_code": report.exit_code,
            "elapsed_ms": report.elapsed_ms,
            "stdout_truncated": report.stdout_truncated,
            "stderr_truncated": report.stderr_truncated,
        });
        evidence.push(build_evidence(
            &exp_id,
            EvidenceId(format!("ev-cmd-{}", report.job_id.0)),
            EvidenceKind::ActionResult,
            "coding_job_execution",
            "coding_evidence_adapter",
            now_ms,
            cmd_payload,
            EvidenceTrust::Authoritative,
            None,
        )?);

        // 3. Files read / changed — observation evidence
        let files_payload = serde_json::json!({
            "changed_files": report.changed_files.iter().map(|cf| {
                serde_json::json!({
                    "path": cf.path.to_string_lossy(),
                    "kind": match cf.kind {
                        ChangedFileKind::Added => "added",
                        ChangedFileKind::Modified => "modified",
                        ChangedFileKind::Deleted => "deleted",
                    },
                    "before_bytes": cf.before_bytes,
                    "after_bytes": cf.after_bytes,
                    "content_sha256": cf.content_sha256,
                })
            }).collect::<Vec<_>>(),
        });
        evidence.push(build_evidence(
            &exp_id,
            EvidenceId(format!("ev-files-{}", report.job_id.0)),
            EvidenceKind::Observation,
            "coding_job_changed_files",
            "coding_evidence_adapter",
            now_ms,
            files_payload,
            EvidenceTrust::Authoritative,
            None,
        )?);

        // 4. Net diff — artifact evidence
        if let Some(ref diff_sha256) = report.diff_sha256 {
            let diff_payload = serde_json::json!({
                "diff_sha256": diff_sha256,
                "diff_artifact": report.diff_artifact.as_ref().map(|p| p.to_string_lossy().to_string()),
            });
            evidence.push(build_evidence(
                &exp_id,
                EvidenceId(format!("ev-diff-{}", report.job_id.0)),
                EvidenceKind::Artifact,
                "coding_job_diff",
                "coding_evidence_adapter",
                now_ms,
                diff_payload,
                EvidenceTrust::Authoritative,
                None,
            )?);
        }

        // 5. Verification results — test results, review findings
        for (i, check) in verification.checks.iter().enumerate() {
            let check_payload = serde_json::json!({
                "name": check.name,
                "severity": match check.severity {
                    fabric::types::coding_job::VerificationSeverity::Advisory => "advisory",
                    fabric::types::coding_job::VerificationSeverity::Required => "required",
                },
                "passed": check.passed,
                "timed_out": check.timed_out,
                "cancelled": check.cancelled,
                "summary": check.summary,
            });
            let trust = if check.passed {
                EvidenceTrust::Authoritative
            } else {
                EvidenceTrust::Corroborated
            };
            evidence.push(build_evidence(
                &exp_id,
                EvidenceId(format!("ev-verify-{}-{}", report.job_id.0, i)),
                EvidenceKind::VerificationResult,
                "coding_verification",
                "coding_evidence_adapter",
                now_ms,
                check_payload,
                trust,
                None,
            )?);
        }

        // 6. Review findings (risk summary) — policy decision evidence
        if !verification.risk_summary.is_empty() {
            let review_payload = serde_json::json!({
                "risk_summary": verification.risk_summary,
                "passed": verification.passed,
            });
            evidence.push(build_evidence(
                &exp_id,
                EvidenceId(format!("ev-review-{}", report.job_id.0)),
                EvidenceKind::PolicyDecision,
                "coding_verification_review",
                "coding_evidence_adapter",
                now_ms,
                review_payload,
                EvidenceTrust::Authoritative,
                None,
            )?);
        }

        let evidence_ids: Vec<_> = evidence.iter().map(|e| e.evidence_id.clone()).collect();

        let mut correlations = BTreeMap::new();
        correlations.insert("task".to_string(), report.job_id.0.to_string());
        correlations.insert("goal".to_string(), report.goal_id.0.to_string());
        correlations.insert("attempt".to_string(), report.attempt_id.0.to_string());

        let envelope = ExperienceEnvelope {
            schema_version: METACOGNITION_SCHEMA_V1,
            experience_id: exp_id,
            domain,
            subject,
            goal_ref: Some(format!("goal-{}", report.goal_id.0)),
            started_at_ms: verification.started_at_ms,
            completed_at_ms: Some(now_ms),
            outcome,
            correlations,
            evidence: evidence_ids,
        };

        Ok(CapturedExperience { envelope, evidence })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn system_time_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn build_evidence(
    exp_id: &ExperienceId,
    ev_id: EvidenceId,
    kind: EvidenceKind,
    source: &str,
    producer: &str,
    captured_at_ms: i64,
    payload: serde_json::Value,
    trust: EvidenceTrust,
    freshness_ms: Option<u64>,
) -> Result<EvidenceItem, CodingEvidenceError> {
    let bytes = serde_json::to_vec(&payload)
        .map_err(|e| CodingEvidenceError::Serialization(e.to_string()))?;
    let sha256 = format!("{:x}", Sha256::digest(bytes));

    Ok(EvidenceItem {
        schema_version: 1,
        evidence_id: ev_id,
        experience_id: exp_id.clone(),
        kind,
        source: source.to_string(),
        producer: producer.to_string(),
        captured_at_ms,
        payload,
        sha256,
        trust,
        freshness_ms,
        redacted: false,
    })
}

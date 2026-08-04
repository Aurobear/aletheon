use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use fabric::types::metacognition_evidence::{
    EvidenceId, EvidenceItem, EvidenceKind, EvidenceTrust,
};
use fabric::types::metacognition_experience::{ExperienceId, METACOGNITION_SCHEMA_V1};
use fabric::{CapabilityTerminalStatus, EvaluationEvidenceSnapshot, TaskEvaluationContract};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{EvaluationApplicationError, TurnEvaluationArtifacts};

const PRODUCER: &str = "executive.evaluation/coding-v2";
pub(crate) const VERIFICATION_SUMMARY_SOURCE: &str = "coding_v2.verification_summary";
pub(crate) const SCOPE_SUMMARY_SOURCE: &str = "coding_v2.scope_summary";

pub trait CodingEvidenceCollector: Send + Sync {
    fn collect(
        &self,
        contract: &TaskEvaluationContract,
        artifacts: &TurnEvaluationArtifacts,
    ) -> Result<EvaluationEvidenceSnapshot, EvaluationApplicationError>;
}

#[derive(Debug, Default)]
pub struct DefaultCodingEvidenceCollector;

#[derive(Debug, Clone, Serialize)]
struct VerificationSummary {
    validation_count: usize,
    all_passed: bool,
    statuses: Vec<CapabilityTerminalStatus>,
}

#[derive(Debug, Clone, Serialize)]
struct ScopeSummary {
    delta_count: usize,
    within_scope: bool,
    violations: Vec<String>,
}

impl CodingEvidenceCollector for DefaultCodingEvidenceCollector {
    fn collect(
        &self,
        contract: &TaskEvaluationContract,
        artifacts: &TurnEvaluationArtifacts,
    ) -> Result<EvaluationEvidenceSnapshot, EvaluationApplicationError> {
        contract.validate()?;
        let now = Utc::now().timestamp_millis();
        let experience = ExperienceId(format!("evaluation:{}", contract.contract_id.0));
        let mut evidence = artifacts.supplemental_evidence.clone();

        let validation_receipts = artifacts
            .capability_receipts
            .iter()
            .filter(|receipt| receipt.capability == "validation_run")
            .collect::<Vec<_>>();
        for receipt in &validation_receipts {
            evidence.push(build_evidence(
                &experience,
                format!(
                    "{}:validation:{}",
                    contract.contract_id.0, receipt.invocation_id
                ),
                EvidenceKind::VerificationResult,
                "capability_terminal_receipt",
                receipt,
                EvidenceTrust::Authoritative,
                now,
            )?);
        }
        let verification_summary = VerificationSummary {
            validation_count: validation_receipts.len(),
            all_passed: !validation_receipts.is_empty()
                && validation_receipts
                    .iter()
                    .all(|receipt| receipt.status == CapabilityTerminalStatus::Succeeded),
            statuses: validation_receipts
                .iter()
                .map(|receipt| receipt.status)
                .collect(),
        };
        evidence.push(build_evidence(
            &experience,
            format!("{}:verification-summary", contract.contract_id.0),
            EvidenceKind::PolicyDecision,
            VERIFICATION_SUMMARY_SOURCE,
            &verification_summary,
            EvidenceTrust::Authoritative,
            now,
        )?);

        let (within_scope, violations) = assess_scope(artifacts);
        let scope_summary = ScopeSummary {
            delta_count: artifacts.file_deltas.len(),
            within_scope,
            violations,
        };
        evidence.push(build_evidence(
            &experience,
            format!("{}:scope-summary", contract.contract_id.0),
            EvidenceKind::PolicyDecision,
            SCOPE_SUMMARY_SOURCE,
            &scope_summary,
            EvidenceTrust::Authoritative,
            now,
        )?);

        for (index, delta) in artifacts.file_deltas.iter().enumerate() {
            evidence.push(build_evidence(
                &experience,
                format!("{}:file-delta:{index}", contract.contract_id.0),
                EvidenceKind::Artifact,
                "turn_file_delta",
                delta,
                EvidenceTrust::Authoritative,
                now,
            )?);
        }
        for (index, fault) in artifacts.runtime_faults.iter().enumerate() {
            evidence.push(build_evidence(
                &experience,
                format!("{}:runtime-fault:{index}", contract.contract_id.0),
                EvidenceKind::RuntimeFault,
                "turn_runtime_fault",
                &serde_json::json!({"message": fault}),
                EvidenceTrust::Authoritative,
                now,
            )?);
        }

        EvaluationEvidenceSnapshot::new(contract.contract_id, evidence, now)
            .map_err(EvaluationApplicationError::from)
    }
}

fn build_evidence(
    experience_id: &ExperienceId,
    id: String,
    kind: EvidenceKind,
    source: &str,
    payload: &impl Serialize,
    trust: EvidenceTrust,
    captured_at_ms: i64,
) -> Result<EvidenceItem, EvaluationApplicationError> {
    let payload = serde_json::to_value(payload)
        .map_err(|error| EvaluationApplicationError::Evidence(error.to_string()))?;
    let bytes = serde_json::to_vec(&payload)
        .map_err(|error| EvaluationApplicationError::Evidence(error.to_string()))?;
    Ok(EvidenceItem {
        schema_version: METACOGNITION_SCHEMA_V1,
        evidence_id: EvidenceId(id),
        experience_id: experience_id.clone(),
        kind,
        source: source.into(),
        producer: PRODUCER.into(),
        captured_at_ms,
        payload,
        sha256: format!("{:x}", Sha256::digest(bytes)),
        trust,
        freshness_ms: Some(0),
        redacted: false,
    })
}

fn assess_scope(artifacts: &TurnEvaluationArtifacts) -> (bool, Vec<String>) {
    let Some(workspace) = &artifacts.workspace else {
        return (false, vec!["authenticated_workspace_missing".into()]);
    };
    let roots = workspace
        .writable_roots()
        .iter()
        .map(|root| materialize(root).unwrap_or_else(|_| root.clone()))
        .collect::<Vec<_>>();
    let protected = workspace.protected_paths().credential_paths();
    let mut violations = Vec::new();
    for delta in &artifacts.file_deltas {
        let relative = Path::new(&delta.path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            violations.push(format!("invalid_relative_path:{}", delta.path));
            continue;
        }
        let candidate = workspace.cwd().join(relative);
        let resolved = materialize(&candidate).unwrap_or(candidate);
        if !roots.iter().any(|root| resolved.starts_with(root)) {
            violations.push(format!("outside_writable_roots:{}", delta.path));
        }
        if protected.iter().any(|path| resolved.starts_with(path)) {
            violations.push(format!("protected_path:{}", delta.path));
        }
    }
    (violations.is_empty(), violations)
}

fn materialize(path: &Path) -> std::io::Result<PathBuf> {
    let mut missing = Vec::new();
    let mut ancestor = path;
    while !ancestor.exists() {
        let Some(name) = ancestor.file_name() else {
            return Ok(path.to_path_buf());
        };
        missing.push(name.to_os_string());
        let Some(parent) = ancestor.parent() else {
            return Ok(path.to_path_buf());
        };
        ancestor = parent;
    }
    let mut resolved = std::fs::canonicalize(ancestor)?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

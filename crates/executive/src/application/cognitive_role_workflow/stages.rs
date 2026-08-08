//! Typed artifact validation and bounded workspace-scope gates for the private workflow.

use fabric::cognitive_workflow::{
    AgentTaskPacket, ArtifactLifecycle, CognitiveArtifact, CognitiveArtifactEnvelope, CognitiveRole,
};

use super::CognitiveWorkspaceError;

pub(super) fn workspace_error(error: CognitiveWorkspaceError) -> anyhow::Error {
    anyhow::Error::new(error)
}

pub(super) fn validate_stage_artifact(
    role: CognitiveRole,
    packet: &AgentTaskPacket,
    envelope: &CognitiveArtifactEnvelope,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        envelope.lifecycle == ArtifactLifecycle::Proposed,
        "stage artifact is not a proposal"
    );
    match (&role, &envelope.artifact) {
        (CognitiveRole::Planner, CognitiveArtifact::Plan(plan)) => {
            let contract = packet
                .selected_artifacts
                .iter()
                .find_map(|artifact| match &artifact.artifact {
                    CognitiveArtifact::TaskContract(contract) => Some(contract),
                    _ => None,
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("planner gate requires a versioned task contract")
                })?;
            anyhow::ensure!(!plan.steps.is_empty(), "planner gate rejects an empty plan");
            let step_ids = plan
                .steps
                .iter()
                .map(|step| step.id.as_str())
                .collect::<std::collections::HashSet<_>>();
            anyhow::ensure!(
                plan.steps.iter().all(|step| !step.id.trim().is_empty()
                    && step
                        .dependencies
                        .iter()
                        .all(|dependency| step_ids.contains(dependency.as_str()))),
                "planner gate rejects invalid step identity or dependency"
            );
            anyhow::ensure!(
                contract.requirement_refs.iter().all(|requirement| plan
                    .steps
                    .iter()
                    .any(|step| step.requirement_refs.contains(requirement))),
                "planner gate rejects incomplete requirement mappings"
            );
        }
        (CognitiveRole::Explorer, CognitiveArtifact::Investigation(report)) => {
            anyhow::ensure!(
                !report.findings.is_empty(),
                "explorer gate requires grounded findings"
            );
            anyhow::ensure!(
                report
                    .findings
                    .iter()
                    .all(|finding| !finding.claim.trim().is_empty()
                        && !finding.evidence_refs.is_empty()
                        && finding.confidence.is_finite()
                        && (0.0..=1.0).contains(&finding.confidence)),
                "explorer gate rejects unsupported findings"
            );
        }
        (CognitiveRole::Executor, CognitiveArtifact::ChangeSet(change)) => {
            anyhow::ensure!(
                !change.transaction_id.trim().is_empty()
                    && !change.workspace_version.trim().is_empty()
                    && !change.diff_artifact_ref.trim().is_empty(),
                "executor gate requires an exact change transaction and diff artifact"
            );
            anyhow::ensure!(
                !change.changed_paths.is_empty(),
                "executor gate requires a material change set"
            );
            anyhow::ensure!(
                change
                    .changed_paths
                    .iter()
                    .all(|path| path_is_within_roots(path, &packet.workspace_roots)),
                "executor gate rejects a changed path outside the owned scope"
            );
        }
        _ => anyhow::bail!("role returned the wrong typed stage artifact"),
    }
    Ok(())
}

pub(super) fn latest_change_set(
    packet: &AgentTaskPacket,
) -> anyhow::Result<&fabric::cognitive_workflow::CognitiveChangeSetReceipt> {
    packet
        .selected_artifacts
        .iter()
        .rev()
        .find_map(|artifact| match &artifact.artifact {
            CognitiveArtifact::ChangeSet(change) => Some(change),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("acceptance role requires an exact committed change set"))
}

pub(super) fn validate_validation_artifact<'a>(
    packet: &AgentTaskPacket,
    envelope: &'a CognitiveArtifactEnvelope,
) -> anyhow::Result<&'a fabric::cognitive_workflow::CognitiveValidationRecord> {
    let change = latest_change_set(packet)?;
    let CognitiveArtifact::Validation(validation) = &envelope.artifact else {
        anyhow::bail!("tester returned the wrong typed artifact")
    };
    anyhow::ensure!(
        validation.transaction_id == change.transaction_id
            && validation.workspace_version == change.workspace_version,
        "validation targets a stale change-set version"
    );
    anyhow::ensure!(
        !validation.validation_receipt_refs.is_empty(),
        "validation has no authoritative terminal receipts"
    );
    Ok(validation)
}

pub(super) fn validate_review_artifact<'a>(
    packet: &AgentTaskPacket,
    envelope: &'a CognitiveArtifactEnvelope,
    task_roots: &[String],
    expected_findings: Option<&[fabric::cognitive_workflow::ReviewFinding]>,
) -> anyhow::Result<&'a [fabric::cognitive_workflow::ReviewFinding]> {
    let change = latest_change_set(packet)?;
    let CognitiveArtifact::Review(review) = &envelope.artifact else {
        anyhow::bail!("reviewer returned the wrong typed artifact")
    };
    anyhow::ensure!(
        review.transaction_id == change.transaction_id
            && review.workspace_version == change.workspace_version,
        "review targets a stale change-set version"
    );
    let mut ids = std::collections::HashSet::new();
    anyhow::ensure!(
        review.findings.iter().all(|finding| {
            let unique_paths = finding
                .affected_paths
                .iter()
                .collect::<std::collections::HashSet<_>>();
            !finding.id.trim().is_empty()
                && !finding.summary.trim().is_empty()
                && !finding.evidence_refs.is_empty()
                && ids.insert(finding.id.clone())
                && (finding.resolved || !finding.affected_paths.is_empty())
                && unique_paths.len() == finding.affected_paths.len()
                && finding
                    .affected_paths
                    .iter()
                    .all(|path| path_is_within_roots(path, task_roots))
        }),
        "review contains invalid, duplicate, or out-of-scope typed findings"
    );
    if let Some(expected_findings) = expected_findings {
        let actual = review
            .findings
            .iter()
            .map(|finding| finding.id.clone())
            .collect::<std::collections::HashSet<_>>();
        let expected = expected_findings
            .iter()
            .map(|finding| finding.id.clone())
            .collect::<std::collections::HashSet<_>>();
        anyhow::ensure!(
            actual == expected,
            "re-review lost or duplicated preserved finding identities"
        );
        for previous in expected_findings {
            let current = review
                .findings
                .iter()
                .find(|finding| finding.id == previous.id)
                .expect("finding identity sets were already checked");
            anyhow::ensure!(
                current.affected_paths == previous.affected_paths,
                "re-review changed preserved finding paths"
            );
        }
    }
    Ok(&review.findings)
}

pub(super) fn unresolved_finding_scope(
    findings: &[fabric::cognitive_workflow::ReviewFinding],
    task_roots: &[String],
) -> anyhow::Result<Vec<String>> {
    let mut scope = Vec::new();
    for finding in findings.iter().filter(|finding| !finding.resolved) {
        anyhow::ensure!(
            !finding.affected_paths.is_empty(),
            "unresolved finding has no affected paths: {}",
            finding.id
        );
        for path in &finding.affected_paths {
            anyhow::ensure!(
                path_is_within_roots(path, task_roots),
                "finding path is outside the owned task scope: {path}"
            );
            if !scope.contains(path) {
                scope.push(path.clone());
            }
        }
    }
    scope.sort();
    anyhow::ensure!(
        !scope.is_empty(),
        "unresolved findings have no repair scope"
    );
    Ok(scope)
}

pub(super) fn validate_fixer_artifact(
    packet: &AgentTaskPacket,
    envelope: &CognitiveArtifactEnvelope,
    finding_ids: &[String],
) -> anyhow::Result<()> {
    let previous = latest_change_set(packet)?;
    let CognitiveArtifact::ChangeSet(repair) = &envelope.artifact else {
        anyhow::bail!("fixer returned the wrong typed artifact")
    };
    anyhow::ensure!(
        repair.transaction_id != previous.transaction_id
            && !repair.workspace_version.trim().is_empty()
            && !repair.diff_artifact_ref.trim().is_empty(),
        "fixer did not produce a new exact change transaction"
    );
    anyhow::ensure!(
        finding_ids.iter().all(|finding| envelope
            .evidence_refs
            .contains(&format!("finding:{finding}"))),
        "fixer output is not bound to every rejected finding"
    );
    anyhow::ensure!(
        repair.changed_paths.iter().all(|path| packet
            .task
            .workspace_scope
            .iter()
            .any(|allowed| path == allowed)),
        "fixer changed a path outside the finding scope"
    );
    Ok(())
}

pub(super) fn path_is_within_roots(path: &str, roots: &[String]) -> bool {
    use std::path::Component;
    let path = std::path::Path::new(path);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return false;
    }
    roots.iter().any(|root| {
        let root = std::path::Path::new(root);
        path.is_absolute() == root.is_absolute() && path.starts_with(root)
    })
}

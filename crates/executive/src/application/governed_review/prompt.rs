use fabric::types::governed_review::GovernedReviewJob;
use fabric::Message;
use serde::Serialize;

pub const GOVERNED_REVIEW_SYSTEM_PROMPT: &str = "You are a governed evidence reviewer. Assess only the supplied evidence. Do not use tools, external knowledge, or unstated facts. Return one JSON object matching the supplied receipt-result schema. Proposed operations must be selected only from allowed_operations. Preserve unresolved conflicts; do not invent a verdict.";

#[derive(Serialize)]
struct ReviewInput<'a> {
    schema_version: u16,
    subject_type: &'a str,
    subject_refs: &'a [String],
    evidence: &'a [fabric::types::governed_review::ReviewEvidence],
    evidence_digest: &'a str,
    policy_ref: &'a str,
    allowed_operations: &'a [String],
    result_schema: ResultSchema,
}

#[derive(Serialize)]
struct ResultSchema {
    evidence_assessed: &'static str,
    findings: &'static str,
    proposed_changes: &'static str,
    confidence_millis: &'static str,
    unresolved_conflicts: &'static str,
    policy_decision: &'static str,
}

pub fn compile(job: &GovernedReviewJob) -> anyhow::Result<(Vec<Message>, usize, u64)> {
    let user = serde_json::to_string(&ReviewInput {
        schema_version: job.schema_version,
        subject_type: &job.subject_type,
        subject_refs: &job.subject_refs,
        evidence: &job.evidence,
        evidence_digest: &job.evidence_digest,
        policy_ref: &job.policy_ref,
        allowed_operations: &job.allowed_operations,
        result_schema: ResultSchema {
            evidence_assessed: "array<string: supplied evidence reference>",
            findings: "array<string>",
            proposed_changes: "array<{operation,target_ref,preconditions,rationale}>",
            confidence_millis: "integer 0..1000",
            unresolved_conflicts: "array<string>",
            policy_decision: "string",
        },
    })?;
    let input_bytes = GOVERNED_REVIEW_SYSTEM_PROMPT
        .len()
        .saturating_add(user.len());
    let estimated_tokens = (input_bytes.saturating_add(3) / 4).saturating_add(32) as u64;
    Ok((
        vec![
            Message::system(GOVERNED_REVIEW_SYSTEM_PROMPT),
            Message::user(user),
        ],
        input_bytes,
        estimated_tokens,
    ))
}

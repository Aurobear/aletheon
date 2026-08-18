use ::contracts::Message;
use application::governed_review::GovernedReviewJob;
use serde::Serialize;

pub const GOVERNED_REVIEW_SYSTEM_PROMPT: &str = "You are a governed evidence reviewer. Assess only the supplied evidence. Do not use tools, external knowledge, or unstated facts. Return one JSON object matching the supplied receipt-result schema. Proposed operations must be selected only from allowed_operations. Preserve unresolved conflicts; do not invent a verdict.";

#[derive(Serialize)]
struct ReviewInput<'a> {
    schema_version: u16,
    subject_type: &'a str,
    subject_refs: &'a [String],
    evidence: &'a [application::governed_review::ReviewEvidence],
    evidence_digest: &'a str,
    policy_ref: &'a str,
    allowed_operations: &'a [String],
    result_schema: ResultSchema,
    result_example: serde_json::Value,
}

#[derive(Serialize)]
struct ResultSchema {
    evidence_assessed: &'static str,
    findings: &'static str,
    proposed_changes: serde_json::Value,
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
            proposed_changes: serde_json::json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["operation", "target_ref", "preconditions", "rationale"],
                    "properties": {
                        "operation": "string from allowed_operations",
                        "target_ref": "string from subject_refs",
                        "preconditions": "array<string>",
                        "rationale": "string"
                    }
                }
            }),
            confidence_millis: "integer 0..1000",
            unresolved_conflicts: "array<string>",
            policy_decision: "string",
        },
        result_example: serde_json::json!({
            "evidence_assessed": ["evidence/reference"],
            "findings": ["finding grounded in supplied evidence"],
            "proposed_changes": [{
                "operation": "allowed_operation",
                "target_ref": "subject/reference",
                "preconditions": ["typed precondition"],
                "rationale": "evidence-grounded rationale"
            }],
            "confidence_millis": 950,
            "unresolved_conflicts": [],
            "policy_decision": "reviewed"
        }),
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

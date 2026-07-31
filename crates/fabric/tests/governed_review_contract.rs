use fabric::governed_review::*;

fn digest(byte: char) -> String {
    std::iter::repeat_n(byte, 64).collect()
}

fn job() -> GovernedReviewJob {
    GovernedReviewJob {
        schema_version: REVIEW_SCHEMA_CURRENT,
        job_id: "job-1".into(),
        subject_type: "document".into(),
        subject_refs: vec!["record/1".into()],
        evidence: vec![ReviewEvidence {
            reference: "evidence/1".into(),
            media_type: "text/plain".into(),
            content: Some("bounded observation".into()),
            digest: digest('a'),
            observed_version: "v1".into(),
        }],
        evidence_digest: digest('b'),
        policy_ref: "policy/default".into(),
        allowed_operations: vec!["retain".into()],
        budget: ReviewBudget {
            max_input_bytes: 4096,
            max_output_bytes: 4096,
            max_tokens: 1024,
            max_tool_calls: 0,
            wall_time_ms: 1000,
        },
        deadline_unix_ms: 4_000_000_000_000,
        idempotency_key: "once-1".into(),
    }
}

#[test]
fn v2_job_and_receipt_round_trip_without_domain_fields() {
    let job = job();
    job.validate().unwrap();
    let encoded = serde_json::to_value(&job).unwrap();
    assert_eq!(
        serde_json::from_value::<GovernedReviewJob>(encoded).unwrap(),
        job
    );

    let receipt = GovernedReviewReceipt {
        schema_version: REVIEW_SCHEMA_CURRENT,
        job_id: job.job_id.clone(),
        status: ReviewStatus::Completed,
        evidence_digest: job.evidence_digest.clone(),
        evidence_assessed: vec!["evidence/1".into()],
        findings: vec!["evidence is internally consistent".into()],
        proposed_changes: vec![ProposedReviewChange {
            operation: "retain".into(),
            target_ref: "record/1".into(),
            preconditions: vec!["observed version remains v1".into()],
            rationale: "supported by supplied evidence".into(),
        }],
        confidence_millis: 900,
        unresolved_conflicts: vec![],
        policy_decision: "allowed".into(),
        runtime_capabilities: vec!["evidence_review".into()],
        usage: ReviewUsage::default(),
        started_at_unix_ms: Some(1),
        completed_at_unix_ms: Some(2),
        error: None,
    };
    receipt.validate().unwrap();
    let json = serde_json::to_string(&receipt).unwrap();
    assert_eq!(
        serde_json::from_str::<GovernedReviewReceipt>(&json).unwrap(),
        receipt
    );
}

#[test]
fn accepts_current_and_previous_schema_only() {
    assert!(ReviewSchemaVersion::new(2).is_ok());
    assert!(ReviewSchemaVersion::new(1).is_ok());
    assert!(ReviewSchemaVersion::new(0).is_err());
    assert!(ReviewSchemaVersion::new(3).is_err());
}

#[test]
fn terminal_status_excludes_queued_and_running() {
    assert!(!ReviewStatus::Queued.is_terminal());
    assert!(!ReviewStatus::Running.is_terminal());
    assert!(ReviewStatus::Completed.is_terminal());
    assert!(ReviewStatus::Rejected.is_terminal());
    assert!(ReviewStatus::Cancelled.is_terminal());
    assert!(ReviewStatus::Expired.is_terminal());
    assert!(ReviewStatus::Failed.is_terminal());
}

#[test]
fn validation_rejects_tools_uppercase_digest_and_unbounded_fields() {
    let mut value = job();
    value.budget.max_tool_calls = 1;
    assert_eq!(
        value.validate(),
        Err(ReviewContractError::ToolCallsForbidden)
    );

    let mut value = job();
    value.evidence_digest = digest('A');
    assert!(matches!(
        value.validate(),
        Err(ReviewContractError::InvalidDigest(_))
    ));

    let mut value = job();
    value.subject_refs = vec!["x".into(); MAX_REVIEW_SUBJECT_REFS + 1];
    assert!(matches!(
        value.validate(),
        Err(ReviewContractError::TooMany { .. })
    ));
}

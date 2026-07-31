use std::os::unix::fs::PermissionsExt;

use executive::application::governed_review::{GovernedReviewStore, ReviewStoreError};
use fabric::types::governed_review::*;

fn digest(byte: char) -> String {
    std::iter::repeat_n(byte, 64).collect()
}

fn job(id: &str, idempotency: &str, evidence_digest: &str) -> GovernedReviewJob {
    GovernedReviewJob {
        schema_version: REVIEW_SCHEMA_CURRENT,
        job_id: id.into(),
        subject_type: "evidence-set".into(),
        subject_refs: vec!["record/1".into()],
        evidence: vec![ReviewEvidence {
            reference: "evidence/1".into(),
            media_type: "text/plain".into(),
            content: Some("observation".into()),
            digest: digest('a'),
            observed_version: "v1".into(),
        }],
        evidence_digest: evidence_digest.into(),
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
        idempotency_key: idempotency.into(),
    }
}

#[tokio::test]
async fn enqueue_is_private_durable_and_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let store = GovernedReviewStore::open(temp.path()).unwrap();
    let first = store
        .enqueue("principal-a", job("job-1", "once", &digest('b')))
        .await
        .unwrap();
    let replay = store
        .enqueue("principal-a", job("different-id", "once", &digest('b')))
        .await
        .unwrap();
    assert_eq!(replay.job.job_id, first.job.job_id);
    assert_eq!(replay.revision, 1);

    let job_file = std::fs::read_dir(store.root().join("jobs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        std::fs::metadata(job_file).unwrap().permissions().mode() & 0o777,
        0o600
    );

    assert!(matches!(
        store
            .enqueue("principal-a", job("job-2", "once", &digest('c')))
            .await,
        Err(ReviewStoreError::IdempotencyConflict)
    ));
}

#[tokio::test]
async fn terminal_receipt_survives_reconstruction_and_cannot_regress() {
    let temp = tempfile::tempdir().unwrap();
    let store = GovernedReviewStore::open(temp.path()).unwrap();
    let queued = store
        .enqueue("principal-a", job("job-1", "once", &digest('b')))
        .await
        .unwrap();
    let mut receipt = queued.receipt;
    receipt.status = ReviewStatus::Completed;
    receipt.started_at_unix_ms = Some(1);
    receipt.completed_at_unix_ms = Some(2);
    receipt.policy_decision = "allowed".into();
    store
        .update_receipt("principal-a", "job-1", receipt)
        .await
        .unwrap();
    drop(store);

    let reopened = GovernedReviewStore::open(temp.path()).unwrap();
    let terminal = reopened.get("principal-a", "job-1").await.unwrap();
    assert_eq!(terminal.receipt.status, ReviewStatus::Completed);
    let mut running = terminal.receipt;
    running.status = ReviewStatus::Running;
    running.completed_at_unix_ms = None;
    assert!(matches!(
        reopened
            .update_receipt("principal-a", "job-1", running)
            .await,
        Err(ReviewStoreError::InvalidTransition { .. })
    ));
}

#[tokio::test]
async fn temporary_files_are_ignored_but_corrupt_authority_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let store = GovernedReviewStore::open(temp.path()).unwrap();
    std::fs::write(store.root().join("jobs/.partial.tmp"), b"{").unwrap();
    drop(store);
    assert!(GovernedReviewStore::open(temp.path()).is_ok());

    std::fs::write(temp.path().join("governed-review/jobs/corrupt.json"), b"{").unwrap();
    assert!(matches!(
        GovernedReviewStore::open(temp.path()),
        Err(ReviewStoreError::Corrupt(_))
    ));
}

#[tokio::test]
async fn principal_scope_is_part_of_both_authoritative_keys() {
    let temp = tempfile::tempdir().unwrap();
    let store = GovernedReviewStore::open(temp.path()).unwrap();
    store
        .enqueue("principal-a", job("same", "same", &digest('b')))
        .await
        .unwrap();
    store
        .enqueue("principal-b", job("same", "same", &digest('c')))
        .await
        .unwrap();
    assert_eq!(
        store
            .get("principal-a", "same")
            .await
            .unwrap()
            .job
            .evidence_digest,
        digest('b')
    );
    assert_eq!(
        store
            .get("principal-b", "same")
            .await
            .unwrap()
            .job
            .evidence_digest,
        digest('c')
    );
}

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use executive::application::governed_review::{
    GovernedReviewLimits, GovernedReviewService, GovernedReviewStore,
};
use executive::application::inference_port::{CoreInferenceRequest, InferenceError, InferencePort};
use fabric::types::governed_review::*;
use fabric::{ContentBlock, LlmResponse, LlmStream, StopReason, Usage};
use tokio::sync::Mutex;

#[derive(Clone)]
enum Reply {
    Response { body: String, delay: Duration },
    Failure(String),
}

struct FakeInference {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<CoreInferenceRequest>>,
}

impl FakeInference {
    fn new(replies: impl IntoIterator<Item = Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into_iter().collect()),
            calls: Mutex::new(vec![]),
        })
    }
}

#[async_trait::async_trait]
impl InferencePort for FakeInference {
    async fn complete(&self, request: CoreInferenceRequest) -> Result<LlmResponse, InferenceError> {
        self.calls.lock().await.push(request);
        match self.replies.lock().await.pop_front().expect("fake reply") {
            Reply::Response { body, delay } => {
                tokio::time::sleep(delay).await;
                Ok(LlmResponse {
                    content: vec![ContentBlock::Text { text: body }],
                    stop_reason: StopReason::EndTurn,
                    usage: Usage {
                        input_tokens: 12,
                        output_tokens: 8,
                    },
                    cache_hit_tokens: 0,
                    cache_miss_tokens: 12,
                })
            }
            Reply::Failure(message) => Err(anyhow::anyhow!(message).into()),
        }
    }

    async fn stream(&self, _request: CoreInferenceRequest) -> Result<LlmStream, InferenceError> {
        Ok(Box::pin(futures::stream::empty()))
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn digest(byte: char) -> String {
    std::iter::repeat_n(byte, 64).collect()
}

fn job(id: &str) -> GovernedReviewJob {
    GovernedReviewJob {
        schema_version: REVIEW_SCHEMA_CURRENT,
        job_id: id.into(),
        subject_type: "document".into(),
        subject_refs: vec![format!("record/{id}")],
        evidence: vec![ReviewEvidence {
            reference: format!("evidence/{id}"),
            media_type: "text/plain".into(),
            content: Some(format!("observation for {id}")),
            digest: digest('a'),
            observed_version: "v1".into(),
        }],
        evidence_digest: digest('b'),
        policy_ref: "policy/default".into(),
        allowed_operations: vec!["retain".into()],
        budget: ReviewBudget {
            max_input_bytes: 16_384,
            max_output_bytes: 4096,
            max_tokens: 4096,
            max_tool_calls: 0,
            wall_time_ms: 2_000,
        },
        deadline_unix_ms: now_ms() + 10_000,
        idempotency_key: format!("once-{id}"),
    }
}

fn completed_json(operation: &str) -> String {
    serde_json::json!({
        "evidence_assessed": ["evidence/job-1"],
        "findings": ["supported"],
        "proposed_changes": [{
            "operation": operation,
            "target_ref": "record/job-1",
            "preconditions": ["version remains v1"],
            "rationale": "supplied evidence"
        }],
        "confidence_millis": 850,
        "unresolved_conflicts": [],
        "policy_decision": "allowed"
    })
    .to_string()
}

fn service(
    temp: &tempfile::TempDir,
    inference: Arc<dyn InferencePort>,
) -> Arc<GovernedReviewService> {
    GovernedReviewService::new(
        Arc::new(GovernedReviewStore::open(temp.path()).unwrap()),
        inference,
        "provider/model",
        GovernedReviewLimits::default(),
        "daemon-1",
        now_ms(),
    )
    .unwrap()
}

#[tokio::test]
async fn completed_review_uses_only_evidence_messages_and_zero_tools() {
    let temp = tempfile::tempdir().unwrap();
    let inference = FakeInference::new([Reply::Response {
        body: completed_json("retain"),
        delay: Duration::ZERO,
    }]);
    let review = service(&temp, inference.clone());
    let queued = review.submit("principal-a", job("job-1")).await.unwrap();
    assert_eq!(queued.receipt.status, ReviewStatus::Queued);
    let receipt = review
        .wait("principal-a", "job-1", Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(receipt.status, ReviewStatus::Completed);
    assert_eq!(receipt.evidence_digest, digest('b'));
    assert_eq!(receipt.usage.inference_requests, 1);

    let calls = inference.calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert!(calls[0].tools.is_empty());
    let encoded = serde_json::to_string(&calls[0].messages).unwrap();
    assert!(encoded.contains("observation for job-1"));
    assert!(encoded.contains("policy/default"));
    assert!(!encoded.contains("idempotency_key"));
}

#[tokio::test]
async fn malformed_provider_and_unsupported_operation_are_terminal_failures() {
    let temp = tempfile::tempdir().unwrap();
    let inference = FakeInference::new([
        Reply::Response {
            body: "not-json".into(),
            delay: Duration::ZERO,
        },
        Reply::Failure("provider rejected sk-superSecret123".into()),
        Reply::Response {
            body: completed_json("delete"),
            delay: Duration::ZERO,
        },
    ]);
    let review = service(&temp, inference);
    for (id, expected) in [
        ("malformed", "malformed structured output"),
        ("provider", "provider failure"),
        ("operation", "unsupported proposed operation"),
    ] {
        review.submit("principal-a", job(id)).await.unwrap();
        let receipt = review
            .wait("principal-a", id, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(receipt.status, ReviewStatus::Failed);
        assert_eq!(receipt.policy_decision, expected);
        if id == "provider" {
            assert_eq!(
                receipt.error.as_deref(),
                Some("inference provider failed: provider rejected [REDACTED]")
            );
        }
    }
}

#[tokio::test]
async fn deadline_output_overflow_and_explicit_cancel_settle_terminally() {
    let temp = tempfile::tempdir().unwrap();
    let inference = FakeInference::new([
        Reply::Response {
            body: completed_json("retain"),
            delay: Duration::ZERO,
        },
        Reply::Response {
            body: completed_json("retain"),
            delay: Duration::from_secs(10),
        },
    ]);
    let review = service(&temp, inference);

    let mut expired = job("expired");
    expired.deadline_unix_ms = now_ms().saturating_sub(1);
    review.submit("principal-a", expired).await.unwrap();
    assert_eq!(
        review
            .wait("principal-a", "expired", Duration::from_secs(2))
            .await
            .unwrap()
            .status,
        ReviewStatus::Expired
    );

    let mut overflow = job("overflow");
    overflow.budget.max_output_bytes = 8;
    review.submit("principal-a", overflow).await.unwrap();
    let overflow = review
        .wait("principal-a", "overflow", Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(overflow.status, ReviewStatus::Failed);
    assert_eq!(overflow.policy_decision, "output budget exceeded");

    review
        .submit("principal-a", job("cancelled"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let cancelled = review.cancel("principal-a", "cancelled").await.unwrap();
    assert_eq!(cancelled.status, ReviewStatus::Cancelled);
    assert_eq!(
        review
            .wait("principal-a", "cancelled", Duration::from_secs(1))
            .await
            .unwrap()
            .status,
        ReviewStatus::Cancelled
    );
}

#[tokio::test]
async fn reconstruction_replays_queued_once_and_exact_idempotency_returns_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(GovernedReviewStore::open(temp.path()).unwrap());
    store.enqueue("principal-a", job("job-1")).await.unwrap();
    drop(store);

    let inference = FakeInference::new([Reply::Response {
        body: completed_json("retain"),
        delay: Duration::ZERO,
    }]);
    let review = service(&temp, inference.clone());
    review.recover().await;
    let terminal = review
        .wait("principal-a", "job-1", Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(terminal.status, ReviewStatus::Completed);

    let replay = review.submit("principal-a", job("job-1")).await.unwrap();
    assert_eq!(replay.receipt, terminal);
    assert_eq!(inference.calls.lock().await.len(), 1);
}

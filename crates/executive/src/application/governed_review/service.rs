use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fabric::governed_review::{
    GovernedReviewJob, GovernedReviewReceipt, ProposedReviewChange, ReviewStatus, ReviewUsage,
    REVIEW_SCHEMA_CURRENT, REVIEW_SCHEMA_PREVIOUS,
};
use fabric::ContentBlock;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::application::inference_port::{CoreInferenceRequest, InferencePort};

use super::prompt;
use super::store::{GovernedReviewStore, ReviewStoreError, StoredReview};

const CAPABILITIES: &[&str] = &[
    "bounded_inference",
    "cancellation",
    "durable_terminal_receipt",
    "evidence_only",
    "idempotent_submission",
    "terminal_wait",
    "zero_tools",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernedReviewLimits {
    pub max_input_bytes: u64,
    pub max_output_bytes: u64,
    pub max_tokens: u64,
    pub max_wall_time_ms: u64,
    pub max_pending_jobs: usize,
    pub max_concurrent_jobs: usize,
}

impl GovernedReviewLimits {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.max_input_bytes == 0
            || self.max_output_bytes == 0
            || self.max_tokens == 0
            || self.max_wall_time_ms == 0
            || self.max_pending_jobs == 0
            || self.max_concurrent_jobs == 0
            || self.max_concurrent_jobs > self.max_pending_jobs
        {
            anyhow::bail!("governed review limits must be positive and concurrency <= pending");
        }
        Ok(())
    }
}

impl Default for GovernedReviewLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 262_144,
            max_output_bytes: 65_536,
            max_tokens: 16_384,
            max_wall_time_ms: 120_000,
            max_pending_jobs: 64,
            max_concurrent_jobs: 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewCapabilities {
    pub protocol_version: u16,
    pub schema_versions: Vec<u16>,
    pub capabilities: Vec<String>,
    pub limits: GovernedReviewLimits,
    pub daemon_instance_id: String,
    pub daemon_started_at_unix_ms: u64,
    pub model_spec: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ReviewServiceError {
    #[error(transparent)]
    Store(#[from] ReviewStoreError),
    #[error("governed review service is at pending capacity")]
    Capacity,
    #[error("job budget exceeds effective service limit: {0}")]
    BudgetExceedsLimit(&'static str),
    #[error("review wait timed out")]
    WaitTimeout,
    #[error("review service configuration is invalid: {0}")]
    Configuration(String),
}

pub struct GovernedReviewService {
    store: Arc<GovernedReviewStore>,
    inference: Arc<dyn InferencePort>,
    model_spec: String,
    limits: GovernedReviewLimits,
    concurrency: Arc<Semaphore>,
    active: Mutex<HashMap<String, CancellationToken>>,
    notifications: Mutex<HashMap<String, Arc<Notify>>>,
    daemon_instance_id: String,
    daemon_started_at_unix_ms: u64,
}

impl GovernedReviewService {
    pub fn new(
        store: Arc<GovernedReviewStore>,
        inference: Arc<dyn InferencePort>,
        model_spec: impl Into<String>,
        limits: GovernedReviewLimits,
        daemon_instance_id: impl Into<String>,
        daemon_started_at_unix_ms: u64,
    ) -> Result<Arc<Self>, ReviewServiceError> {
        limits
            .validate()
            .map_err(|error| ReviewServiceError::Configuration(error.to_string()))?;
        let model_spec = model_spec.into();
        if model_spec.trim().is_empty() {
            return Err(ReviewServiceError::Configuration(
                "model_spec must not be empty".into(),
            ));
        }
        Ok(Arc::new(Self {
            store,
            inference,
            model_spec,
            concurrency: Arc::new(Semaphore::new(limits.max_concurrent_jobs)),
            limits,
            active: Mutex::new(HashMap::new()),
            notifications: Mutex::new(HashMap::new()),
            daemon_instance_id: daemon_instance_id.into(),
            daemon_started_at_unix_ms,
        }))
    }

    pub fn capabilities(&self) -> ReviewCapabilities {
        ReviewCapabilities {
            protocol_version: 1,
            schema_versions: vec![REVIEW_SCHEMA_CURRENT, REVIEW_SCHEMA_PREVIOUS],
            capabilities: CAPABILITIES
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            limits: self.limits.clone(),
            daemon_instance_id: self.daemon_instance_id.clone(),
            daemon_started_at_unix_ms: self.daemon_started_at_unix_ms,
            model_spec: self.model_spec.clone(),
        }
    }

    pub async fn recover(self: &Arc<Self>) {
        for stored in self.store.nonterminal().await {
            self.schedule(stored).await;
        }
    }

    pub async fn submit(
        self: &Arc<Self>,
        principal_id: &str,
        job: GovernedReviewJob,
    ) -> Result<StoredReview, ReviewServiceError> {
        self.validate_effective_budget(&job)?;
        if let Some(existing) = self
            .store
            .find_by_idempotency(principal_id, &job.idempotency_key)
            .await?
        {
            if existing.job.evidence_digest != job.evidence_digest {
                return Err(ReviewStoreError::IdempotencyConflict.into());
            }
            if !existing.receipt.status.is_terminal() {
                self.schedule(existing.clone()).await;
            }
            return Ok(existing);
        }
        if self.store.nonterminal().await.len() >= self.limits.max_pending_jobs {
            return Err(ReviewServiceError::Capacity);
        }
        let stored = self.store.enqueue(principal_id, job).await?;
        self.schedule(stored.clone()).await;
        Ok(stored)
    }

    pub async fn status(
        &self,
        principal_id: &str,
        job_id: &str,
    ) -> Result<GovernedReviewReceipt, ReviewServiceError> {
        Ok(self.store.get(principal_id, job_id).await?.receipt)
    }

    pub async fn wait(
        &self,
        principal_id: &str,
        job_id: &str,
        timeout: Duration,
    ) -> Result<GovernedReviewReceipt, ReviewServiceError> {
        let key = active_key(principal_id, job_id);
        let notify = self.notify_for(&key).await;
        let wait = async {
            loop {
                let receipt = self.status(principal_id, job_id).await?;
                if receipt.status.is_terminal() {
                    return Ok(receipt);
                }
                let notified = notify.notified();
                let receipt = self.status(principal_id, job_id).await?;
                if receipt.status.is_terminal() {
                    return Ok(receipt);
                }
                notified.await;
            }
        };
        tokio::time::timeout(timeout, wait)
            .await
            .map_err(|_| ReviewServiceError::WaitTimeout)?
    }

    pub async fn cancel(
        &self,
        principal_id: &str,
        job_id: &str,
    ) -> Result<GovernedReviewReceipt, ReviewServiceError> {
        let key = active_key(principal_id, job_id);
        if let Some(token) = self.active.lock().await.get(&key).cloned() {
            token.cancel();
        }
        let current = self.store.get(principal_id, job_id).await?;
        if current.receipt.status.is_terminal() {
            return Ok(current.receipt);
        }
        let receipt = terminal_receipt(
            &current,
            ReviewStatus::Cancelled,
            "cancelled by authenticated caller",
            Some("cancelled"),
            ReviewUsage::default(),
            current.receipt.started_at_unix_ms,
        );
        match self
            .store
            .update_receipt(principal_id, job_id, receipt)
            .await
        {
            Ok(stored) => {
                self.notify_for(&key).await.notify_waiters();
                Ok(stored.receipt)
            }
            Err(ReviewStoreError::InvalidTransition { .. }) => {
                Ok(self.store.get(principal_id, job_id).await?.receipt)
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn schedule(self: &Arc<Self>, stored: StoredReview) {
        let key = active_key(&stored.principal_id, &stored.job.job_id);
        let mut active = self.active.lock().await;
        if active.contains_key(&key) || stored.receipt.status.is_terminal() {
            return;
        }
        let cancellation = CancellationToken::new();
        active.insert(key.clone(), cancellation.clone());
        drop(active);
        let service = Arc::clone(self);
        tokio::spawn(async move {
            service.execute(stored, cancellation).await;
            service.active.lock().await.remove(&key);
        });
    }

    async fn execute(&self, mut stored: StoredReview, cancellation: CancellationToken) {
        let principal = stored.principal_id.clone();
        let job_id = stored.job.job_id.clone();
        let key = active_key(&principal, &job_id);
        let started = unix_now_ms();
        if started >= stored.job.deadline_unix_ms {
            self.settle_failure(
                &stored,
                ReviewStatus::Expired,
                "deadline expired",
                None,
                started,
            )
            .await;
            return;
        }
        let permit = tokio::select! {
            _ = cancellation.cancelled() => {
                let _ = self.cancel(&principal, &job_id).await;
                return;
            }
            permit = self.concurrency.clone().acquire_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => {
                    self.settle_failure(&stored, ReviewStatus::Failed, "service shutting down", None, started).await;
                    return;
                }
            }
        };
        let mut running = stored.receipt.clone();
        running.status = ReviewStatus::Running;
        running.started_at_unix_ms = Some(started);
        stored = match self
            .store
            .update_receipt(&principal, &job_id, running)
            .await
        {
            Ok(stored) => stored,
            Err(ReviewStoreError::InvalidTransition { .. }) => return,
            Err(error) => {
                tracing::error!(%error, job_id, "failed to persist running review state");
                return;
            }
        };

        let (messages, input_bytes, estimated_tokens) = match prompt::compile(&stored.job) {
            Ok(compiled) => compiled,
            Err(error) => {
                self.settle_failure(
                    &stored,
                    ReviewStatus::Failed,
                    "prompt serialization failed",
                    Some(&error.to_string()),
                    started,
                )
                .await;
                return;
            }
        };
        if input_bytes as u64 > stored.job.budget.max_input_bytes
            || estimated_tokens > stored.job.budget.max_tokens
        {
            self.settle_failure(
                &stored,
                ReviewStatus::Rejected,
                "input budget exceeded",
                None,
                started,
            )
            .await;
            return;
        }
        let remaining_deadline =
            Duration::from_millis(stored.job.deadline_unix_ms.saturating_sub(unix_now_ms()));
        let timeout = remaining_deadline.min(Duration::from_millis(stored.job.budget.wall_time_ms));
        let before = Instant::now();
        let inference = self.inference.complete(CoreInferenceRequest {
            messages,
            tools: vec![],
            model_spec: self.model_spec.clone(),
        });
        let outcome = tokio::select! {
            _ = cancellation.cancelled() => {
                drop(permit);
                let _ = self.cancel(&principal, &job_id).await;
                return;
            }
            result = tokio::time::timeout(timeout, inference) => result,
        };
        drop(permit);
        let response = match outcome {
            Err(_) => {
                let status = if unix_now_ms() >= stored.job.deadline_unix_ms {
                    ReviewStatus::Expired
                } else {
                    ReviewStatus::Failed
                };
                self.settle_failure(&stored, status, "inference timeout", None, started)
                    .await;
                return;
            }
            Ok(Err(error)) => {
                self.settle_failure(
                    &stored,
                    ReviewStatus::Failed,
                    "provider failure",
                    Some(&error.to_string()),
                    started,
                )
                .await;
                return;
            }
            Ok(Ok(response)) => response,
        };
        let usage = ReviewUsage {
            input_tokens: response.usage.input_tokens.into(),
            output_tokens: response.usage.output_tokens.into(),
            inference_requests: 1,
            elapsed_ms: u64::try_from(before.elapsed().as_millis()).unwrap_or(u64::MAX),
        };
        let mut output = String::new();
        for block in &response.content {
            if let ContentBlock::Text { text } = block {
                if output.len().saturating_add(text.len())
                    > stored.job.budget.max_output_bytes as usize
                {
                    self.settle_failure(
                        &stored,
                        ReviewStatus::Failed,
                        "output budget exceeded",
                        None,
                        started,
                    )
                    .await;
                    return;
                }
                output.push_str(text);
            }
        }
        let result: ModelReviewResult = match serde_json::from_str(output.trim()) {
            Ok(result) => result,
            Err(error) => {
                self.settle_failure(
                    &stored,
                    ReviewStatus::Failed,
                    "malformed structured output",
                    Some(&error.to_string()),
                    started,
                )
                .await;
                return;
            }
        };
        if result
            .proposed_changes
            .iter()
            .any(|change| !stored.job.allowed_operations.contains(&change.operation))
        {
            self.settle_failure(
                &stored,
                ReviewStatus::Failed,
                "unsupported proposed operation",
                None,
                started,
            )
            .await;
            return;
        }
        let receipt = GovernedReviewReceipt {
            schema_version: stored.job.schema_version,
            job_id: job_id.clone(),
            status: ReviewStatus::Completed,
            evidence_digest: stored.job.evidence_digest.clone(),
            evidence_assessed: result.evidence_assessed,
            findings: result.findings,
            proposed_changes: result.proposed_changes,
            confidence_millis: result.confidence_millis,
            unresolved_conflicts: result.unresolved_conflicts,
            policy_decision: result.policy_decision,
            runtime_capabilities: CAPABILITIES
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            usage,
            started_at_unix_ms: Some(started),
            completed_at_unix_ms: Some(unix_now_ms()),
            error: None,
        };
        if let Err(error) = receipt.validate() {
            self.settle_failure(
                &stored,
                ReviewStatus::Failed,
                "invalid structured output",
                Some(&error.to_string()),
                started,
            )
            .await;
            return;
        }
        match self
            .store
            .update_receipt(&principal, &job_id, receipt)
            .await
        {
            Ok(_) => self.notify_for(&key).await.notify_waiters(),
            Err(ReviewStoreError::InvalidTransition { .. }) => {}
            Err(error) => {
                tracing::error!(%error, job_id, "failed to persist terminal review receipt")
            }
        }
    }

    async fn settle_failure(
        &self,
        stored: &StoredReview,
        status: ReviewStatus,
        decision: &str,
        error: Option<&str>,
        started: u64,
    ) {
        let receipt = terminal_receipt(
            stored,
            status,
            decision,
            error,
            ReviewUsage::default(),
            Some(started),
        );
        let key = active_key(&stored.principal_id, &stored.job.job_id);
        match self
            .store
            .update_receipt(&stored.principal_id, &stored.job.job_id, receipt)
            .await
        {
            Ok(_) => self.notify_for(&key).await.notify_waiters(),
            Err(ReviewStoreError::InvalidTransition { .. }) => {}
            Err(error) => {
                tracing::error!(%error, job_id = %stored.job.job_id, "failed to persist failed review receipt")
            }
        }
    }

    async fn notify_for(&self, key: &str) -> Arc<Notify> {
        self.notifications
            .lock()
            .await
            .entry(key.to_owned())
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone()
    }

    fn validate_effective_budget(&self, job: &GovernedReviewJob) -> Result<(), ReviewServiceError> {
        job.validate().map_err(ReviewStoreError::from)?;
        for (field, requested, effective) in [
            (
                "max_input_bytes",
                job.budget.max_input_bytes,
                self.limits.max_input_bytes,
            ),
            (
                "max_output_bytes",
                job.budget.max_output_bytes,
                self.limits.max_output_bytes,
            ),
            ("max_tokens", job.budget.max_tokens, self.limits.max_tokens),
            (
                "wall_time_ms",
                job.budget.wall_time_ms,
                self.limits.max_wall_time_ms,
            ),
        ] {
            if requested > effective {
                return Err(ReviewServiceError::BudgetExceedsLimit(field));
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelReviewResult {
    evidence_assessed: Vec<String>,
    findings: Vec<String>,
    proposed_changes: Vec<ProposedReviewChange>,
    confidence_millis: u16,
    unresolved_conflicts: Vec<String>,
    policy_decision: String,
}

fn terminal_receipt(
    stored: &StoredReview,
    status: ReviewStatus,
    decision: &str,
    error: Option<&str>,
    usage: ReviewUsage,
    started_at_unix_ms: Option<u64>,
) -> GovernedReviewReceipt {
    GovernedReviewReceipt {
        schema_version: stored.job.schema_version,
        job_id: stored.job.job_id.clone(),
        status,
        evidence_digest: stored.job.evidence_digest.clone(),
        evidence_assessed: vec![],
        findings: vec![],
        proposed_changes: vec![],
        confidence_millis: 0,
        unresolved_conflicts: vec![],
        policy_decision: decision.to_owned(),
        runtime_capabilities: CAPABILITIES
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        usage,
        started_at_unix_ms,
        completed_at_unix_ms: Some(unix_now_ms()),
        error: error.map(redacted_error),
    }
}

fn redacted_error(error: &str) -> String {
    let governed = fabric::types::data_governance::scrub_for_projection(
        error,
        fabric::types::data_governance::ContentTrust::ExternalUntrusted,
    );
    let max = fabric::governed_review::MAX_REVIEW_ERROR_BYTES;
    if governed.content.len() <= max {
        return governed.content;
    }
    let mut end = max;
    while !governed.content.is_char_boundary(end) {
        end -= 1;
    }
    governed.content[..end].to_owned()
}

fn active_key(principal_id: &str, job_id: &str) -> String {
    format!("{principal_id}\0{job_id}")
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

//! Narrow consumer port for governed review used by the daemon transport.
//!
//! The concrete `GovernedReviewService` uses `&Arc<Self>` receivers for its
//! scheduler (`submit`, `recover`, `schedule`). A trait object cannot express
//! those receivers, so this module exposes a plain `&self` port plus an adapter
//! that owns the `Arc` and forwards to the concrete service.

use std::sync::Arc;
use std::time::Duration;

use application::governed_review::{GovernedReviewJob, GovernedReviewReceipt};
use async_trait::async_trait;

use super::service::{GovernedReviewService, ReviewCapabilities, ReviewServiceError};
use super::store::StoredReview;

#[async_trait]
pub trait ReviewPort: Send + Sync {
    fn capabilities(&self) -> ReviewCapabilities;
    async fn submit(
        &self,
        principal_id: &str,
        job: GovernedReviewJob,
    ) -> Result<StoredReview, ReviewServiceError>;
    async fn status(
        &self,
        principal_id: &str,
        job_id: &str,
    ) -> Result<GovernedReviewReceipt, ReviewServiceError>;
    async fn wait(
        &self,
        principal_id: &str,
        job_id: &str,
        timeout: Duration,
    ) -> Result<GovernedReviewReceipt, ReviewServiceError>;
    async fn cancel(
        &self,
        principal_id: &str,
        job_id: &str,
    ) -> Result<GovernedReviewReceipt, ReviewServiceError>;
}

pub struct ReviewPortAdapter(pub Arc<GovernedReviewService>);

#[async_trait]
impl ReviewPort for ReviewPortAdapter {
    fn capabilities(&self) -> ReviewCapabilities {
        self.0.capabilities()
    }
    async fn submit(
        &self,
        principal_id: &str,
        job: GovernedReviewJob,
    ) -> Result<StoredReview, ReviewServiceError> {
        self.0.submit(principal_id, job).await
    }
    async fn status(
        &self,
        principal_id: &str,
        job_id: &str,
    ) -> Result<GovernedReviewReceipt, ReviewServiceError> {
        self.0.status(principal_id, job_id).await
    }
    async fn wait(
        &self,
        principal_id: &str,
        job_id: &str,
        timeout: Duration,
    ) -> Result<GovernedReviewReceipt, ReviewServiceError> {
        self.0.wait(principal_id, job_id, timeout).await
    }
    async fn cancel(
        &self,
        principal_id: &str,
        job_id: &str,
    ) -> Result<GovernedReviewReceipt, ReviewServiceError> {
        self.0.cancel(principal_id, job_id).await
    }
}

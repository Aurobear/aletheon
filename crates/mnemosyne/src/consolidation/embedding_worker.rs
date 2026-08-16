use std::sync::Arc;

use ::contracts::{Clock, EmbeddingProvider};

use super::ConsolidationRepository;
use crate::{EmbeddedRecord, VectorIndexWriter};

pub struct MemoryEmbeddingWorker {
    repository: Arc<ConsolidationRepository>,
    provider: Arc<dyn EmbeddingProvider>,
    writer: Arc<dyn VectorIndexWriter>,
    provider_id: String,
    rotation_generation: u64,
    owner: String,
    clock: Arc<dyn Clock>,
    batch_size: usize,
}

impl MemoryEmbeddingWorker {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repository: Arc<ConsolidationRepository>,
        provider: Arc<dyn EmbeddingProvider>,
        writer: Arc<dyn VectorIndexWriter>,
        provider_id: impl Into<String>,
        rotation_generation: u64,
        owner: impl Into<String>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            repository,
            provider,
            writer,
            provider_id: provider_id.into(),
            rotation_generation,
            owner: owner.into(),
            clock,
            batch_size: 32,
        }
    }

    pub async fn run_once(&self) -> anyhow::Result<usize> {
        let now = self.clock.wall_now().0.max(0) as u64;
        self.repository.enqueue_embedding_backfill(
            &self.provider_id,
            self.provider.model_id(),
            self.provider.dimension(),
            self.rotation_generation,
            now,
        )?;
        let jobs = self.repository.claim_embedding_jobs(
            &self.provider_id,
            self.provider.model_id(),
            self.provider.dimension(),
            self.rotation_generation,
            &self.owner,
            now,
            60_000,
            self.batch_size,
        )?;
        if jobs.is_empty() {
            return Ok(0);
        }
        let upserts = jobs
            .iter()
            .filter_map(|job| job.item.clone())
            .collect::<Vec<_>>();
        let texts = upserts
            .iter()
            .map(|item| item.content.clone())
            .collect::<Vec<_>>();
        match self.provider.embed_batch(&texts).await {
            Ok(vectors) if vectors.len() == upserts.len() => {
                let records = upserts
                    .into_iter()
                    .zip(vectors)
                    .map(|(item, embedding)| EmbeddedRecord {
                        item,
                        embedding,
                        provider_id: self.provider_id.clone(),
                        model_id: self.provider.model_id().to_string(),
                        rotation_generation: self.rotation_generation,
                    })
                    .collect::<Vec<_>>();
                self.writer.upsert_batch(&records).await?;
                for job in &jobs {
                    self.finish(job, true, 0, None)?;
                }
            }
            Ok(_) => anyhow::bail!("embedding response cardinality mismatch"),
            Err(error) => {
                let retry_at = now.saturating_add(5_000);
                for job in &jobs {
                    self.finish(job, false, retry_at, Some(&error.to_string()))?;
                }
                return Err(error);
            }
        }
        Ok(jobs.len())
    }

    fn finish(
        &self,
        job: &super::repository::EmbeddingJobRecord,
        success: bool,
        retry_at_ms: u64,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        self.repository.finish_embedding_job(
            &job.record_id,
            &self.provider_id,
            self.provider.model_id(),
            self.provider.dimension(),
            self.rotation_generation,
            &self.owner,
            success,
            retry_at_ms,
            error,
            self.clock.wall_now().0.max(0) as u64,
        )
    }
}

//! Experience ingestion — validates and records experience envelopes.
//!
//! Validates schema version, completion time ordering, evidence references,
//! and idempotency before append.

use async_trait::async_trait;
use thiserror::Error;

use super::model::{ExperienceEnvelope, ExperienceId, ExperienceOutcome, METACOGNITION_SCHEMA_V1};
use crate::evidence::store::{AppendOutcome, EvidenceStore};

#[derive(Debug, Error)]
pub enum ExperienceIngestError {
    #[error("unsupported schema version {0}")]
    UnsupportedSchema(u16),
    #[error("invalid completion time: completed before started")]
    InvalidTimeOrdering,
    #[error("missing evidence references: {0:?}")]
    MissingEvidence(Vec<String>),
    #[error("evidence store error: {0}")]
    EvidenceStore(String),
    #[error("persistence error: {0}")]
    Persistence(String),
}

/// Trait for persisting experience envelopes.
#[async_trait]
pub trait ExperienceStore: Send + Sync {
    async fn append(
        &self,
        envelope: &ExperienceEnvelope,
    ) -> Result<AppendOutcome, ExperienceIngestError>;
    async fn get(
        &self,
        id: &ExperienceId,
    ) -> Result<Option<ExperienceEnvelope>, ExperienceIngestError>;
}

/// In-memory experience store for testing.
pub struct InMemoryExperienceStore {
    items: std::sync::Mutex<Vec<ExperienceEnvelope>>,
}

impl InMemoryExperienceStore {
    pub fn new() -> Self {
        Self {
            items: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl Default for InMemoryExperienceStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExperienceStore for InMemoryExperienceStore {
    async fn append(
        &self,
        envelope: &ExperienceEnvelope,
    ) -> Result<AppendOutcome, ExperienceIngestError> {
        let mut items = self
            .items
            .lock()
            .map_err(|e| ExperienceIngestError::Persistence(format!("lock poisoned: {}", e)))?;
        if items
            .iter()
            .any(|e| e.experience_id == envelope.experience_id)
        {
            return Ok(AppendOutcome::AlreadyPresent);
        }
        items.push(envelope.clone());
        Ok(AppendOutcome::Appended)
    }

    async fn get(
        &self,
        id: &ExperienceId,
    ) -> Result<Option<ExperienceEnvelope>, ExperienceIngestError> {
        let items = self
            .items
            .lock()
            .map_err(|e| ExperienceIngestError::Persistence(format!("lock poisoned: {}", e)))?;
        Ok(items.iter().find(|e| &e.experience_id == id).cloned())
    }
}

/// Ingests and validates experience envelopes.
pub struct ExperienceIngestor<E> {
    evidence: std::sync::Arc<E>,
}

impl<E: EvidenceStore> ExperienceIngestor<E> {
    pub fn new(evidence: std::sync::Arc<E>) -> Self {
        Self { evidence }
    }

    /// Validate and ingest an experience envelope.
    ///
    /// Checks:
    /// 1. Schema version is supported
    /// 2. Completion time is after start time
    /// 3. All referenced evidence IDs exist
    /// 4. Envelope is not a duplicate
    pub async fn ingest(
        &self,
        envelope: &ExperienceEnvelope,
        store: &dyn ExperienceStore,
    ) -> Result<AppendOutcome, ExperienceIngestError> {
        // Schema validation
        if envelope.schema_version != METACOGNITION_SCHEMA_V1 {
            return Err(ExperienceIngestError::UnsupportedSchema(
                envelope.schema_version,
            ));
        }

        // Time ordering
        if let Some(completed) = envelope.completed_at_ms {
            if completed < envelope.started_at_ms {
                return Err(ExperienceIngestError::InvalidTimeOrdering);
            }
        }

        // Evidence references must all exist
        let mut missing = Vec::new();
        for ev_id in &envelope.evidence {
            match self.evidence.get(ev_id).await {
                Ok(None) | Err(_) => missing.push(ev_id.0.clone()),
                Ok(Some(_)) => {}
            }
        }
        if !missing.is_empty() {
            return Err(ExperienceIngestError::MissingEvidence(missing));
        }

        // Persist
        store.append(envelope).await
    }
}

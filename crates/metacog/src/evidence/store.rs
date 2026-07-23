//! Append-only evidence store with integrity-checked persistence.
//!
//! Stores evidence items in a JSONL file. Rejects mismatched digests and
//! duplicate IDs with conflicting payloads.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use thiserror::Error;

use super::integrity;
use super::model::{EvidenceId, EvidenceItem, ExperienceId};

#[derive(Debug, Error)]
pub enum EvidenceStoreError {
    #[error("evidence integrity check failed: {0}")]
    IntegrityViolation(String),
    #[error("duplicate evidence id with conflicting payload")]
    Conflict,
    #[error("evidence persistence failed: {0}")]
    Persistence(String),
}

/// Outcome of appending an evidence item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendOutcome {
    Appended,
    AlreadyPresent,
}

/// Evidence store port — append-only persistence with integrity validation.
#[async_trait]
pub trait EvidenceStore: Send + Sync {
    async fn append(&self, item: EvidenceItem) -> Result<AppendOutcome, EvidenceStoreError>;
    async fn get(&self, id: &EvidenceId) -> Result<Option<EvidenceItem>, EvidenceStoreError>;
    async fn list_for_experience(
        &self,
        id: &ExperienceId,
    ) -> Result<Vec<EvidenceItem>, EvidenceStoreError>;
}

/// JSONL-backed evidence store.
///
/// Each line is one versioned event. The store verifies SHA-256 integrity
/// before accepting any item.
pub struct JsonlEvidenceStore {
    path: Option<PathBuf>,
    items: Mutex<Vec<EvidenceItem>>,
}

impl JsonlEvidenceStore {
    /// Create an in-memory store (for testing).
    pub fn in_memory() -> Self {
        Self {
            path: None,
            items: Mutex::new(Vec::new()),
        }
    }

    /// Open (or create) a JSONL file as the backing store.
    pub fn open(path: PathBuf) -> Result<Self, EvidenceStoreError> {
        let items = if path.exists() {
            let file = std::fs::File::open(&path)
                .map_err(|e| EvidenceStoreError::Persistence(e.to_string()))?;
            let reader = BufReader::new(file);
            let mut items = Vec::new();
            for line in reader.lines() {
                let line = line.map_err(|e| EvidenceStoreError::Persistence(e.to_string()))?;
                if line.trim().is_empty() {
                    continue;
                }
                let item: EvidenceItem = serde_json::from_str(&line)
                    .map_err(|e| EvidenceStoreError::Persistence(e.to_string()))?;
                items.push(item);
            }
            items
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| EvidenceStoreError::Persistence(e.to_string()))?;
            }
            Vec::new()
        };
        Ok(Self {
            path: Some(path),
            items: Mutex::new(items),
        })
    }

    fn persist(&self) -> Result<(), EvidenceStoreError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let temp = path.with_extension(".tmp");
        let items = self
            .items
            .lock()
            .map_err(|e| EvidenceStoreError::Persistence(format!("lock poisoned: {}", e)))?;
        let mut file = std::fs::File::create(&temp)
            .map_err(|e| EvidenceStoreError::Persistence(e.to_string()))?;
        for item in items.iter() {
            let line = serde_json::to_string(item)
                .map_err(|e| EvidenceStoreError::Persistence(e.to_string()))?;
            writeln!(file, "{}", line)
                .map_err(|e| EvidenceStoreError::Persistence(e.to_string()))?;
        }
        file.sync_all()
            .map_err(|e| EvidenceStoreError::Persistence(e.to_string()))?;
        std::fs::rename(&temp, path).map_err(|e| EvidenceStoreError::Persistence(e.to_string()))?;
        Ok(())
    }
}

#[async_trait]
impl EvidenceStore for JsonlEvidenceStore {
    async fn append(&self, item: EvidenceItem) -> Result<AppendOutcome, EvidenceStoreError> {
        // Integrity check — reject mismatched digest
        if !integrity::verify_integrity(&item) {
            return Err(EvidenceStoreError::IntegrityViolation(format!(
                "digest mismatch for evidence {}",
                item.evidence_id.0
            )));
        }

        let mut items = self
            .items
            .lock()
            .map_err(|e| EvidenceStoreError::Persistence(format!("lock poisoned: {}", e)))?;

        // Check for duplicates
        if let Some(existing) = items.iter().find(|i| i.evidence_id == item.evidence_id) {
            if existing.sha256 == item.sha256 {
                return Ok(AppendOutcome::AlreadyPresent);
            }
            return Err(EvidenceStoreError::Conflict);
        }

        items.push(item);
        drop(items);
        self.persist()?;
        Ok(AppendOutcome::Appended)
    }

    async fn get(&self, id: &EvidenceId) -> Result<Option<EvidenceItem>, EvidenceStoreError> {
        let items = self
            .items
            .lock()
            .map_err(|e| EvidenceStoreError::Persistence(format!("lock poisoned: {}", e)))?;
        Ok(items.iter().find(|i| &i.evidence_id == id).cloned())
    }

    async fn list_for_experience(
        &self,
        id: &ExperienceId,
    ) -> Result<Vec<EvidenceItem>, EvidenceStoreError> {
        let items = self
            .items
            .lock()
            .map_err(|e| EvidenceStoreError::Persistence(format!("lock poisoned: {}", e)))?;
        Ok(items
            .iter()
            .filter(|i| &i.experience_id == id)
            .cloned()
            .collect())
    }
}

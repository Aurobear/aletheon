use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use fabric::types::governed_review::{
    GovernedReviewJob, GovernedReviewReceipt, ReviewStatus, ReviewUsage,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

const STORE_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredReview {
    pub store_schema_version: u16,
    pub principal_id: String,
    pub job: GovernedReviewJob,
    pub receipt: GovernedReviewReceipt,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdempotencyRecord {
    store_schema_version: u16,
    principal_id: String,
    idempotency_key: String,
    evidence_digest: String,
    job_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ReviewStoreError {
    #[error("invalid review contract: {0}")]
    InvalidContract(#[from] fabric::types::governed_review::ReviewContractError),
    #[error("review state is corrupt: {0}")]
    Corrupt(String),
    #[error("review job was not found")]
    NotFound,
    #[error("idempotency key is already bound to different evidence")]
    IdempotencyConflict,
    #[error("review job identifier is already bound to different evidence")]
    JobConflict,
    #[error("invalid review state transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: ReviewStatus,
        to: ReviewStatus,
    },
    #[error("review persistence failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("review serialization failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Default)]
struct StoreIndex {
    jobs: HashMap<String, StoredReview>,
    idempotency: HashMap<String, IdempotencyRecord>,
}

pub struct GovernedReviewStore {
    root: PathBuf,
    jobs_dir: PathBuf,
    idempotency_dir: PathBuf,
    index: Mutex<StoreIndex>,
}

impl GovernedReviewStore {
    pub fn open(state_root: impl AsRef<Path>) -> Result<Self, ReviewStoreError> {
        let root = state_root.as_ref().join("governed-review");
        let jobs_dir = root.join("jobs");
        let idempotency_dir = root.join("idempotency");
        private_dir(&root)?;
        private_dir(&jobs_dir)?;
        private_dir(&idempotency_dir)?;

        let mut index = StoreIndex::default();
        for path in authoritative_json_files(&jobs_dir)? {
            let stored: StoredReview = read_json(&path)?;
            validate_stored(&stored)?;
            let key = job_key(&stored.principal_id, &stored.job.job_id);
            if index.jobs.insert(key, stored).is_some() {
                return Err(ReviewStoreError::Corrupt(format!(
                    "duplicate authoritative job record at {}",
                    path.display()
                )));
            }
        }
        for path in authoritative_json_files(&idempotency_dir)? {
            let record: IdempotencyRecord = read_json(&path)?;
            validate_idempotency(&record)?;
            let key = idempotency_key(&record.principal_id, &record.idempotency_key);
            let stored = index
                .jobs
                .get(&job_key(&record.principal_id, &record.job_id));
            if !matches!(stored, Some(stored) if stored.job.evidence_digest == record.evidence_digest)
            {
                return Err(ReviewStoreError::Corrupt(format!(
                    "idempotency record {} does not resolve to matching evidence",
                    path.display()
                )));
            }
            if index.idempotency.insert(key, record).is_some() {
                return Err(ReviewStoreError::Corrupt(format!(
                    "duplicate idempotency record at {}",
                    path.display()
                )));
            }
        }
        Ok(Self {
            root,
            jobs_dir,
            idempotency_dir,
            index: Mutex::new(index),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub async fn enqueue(
        &self,
        principal_id: &str,
        job: GovernedReviewJob,
    ) -> Result<StoredReview, ReviewStoreError> {
        job.validate()?;
        require_identity(principal_id, "principal_id")?;
        let mut index = self.index.lock().await;
        let idem_key = idempotency_key(principal_id, &job.idempotency_key);
        if let Some(record) = index.idempotency.get(&idem_key) {
            if record.evidence_digest != job.evidence_digest {
                return Err(ReviewStoreError::IdempotencyConflict);
            }
            return index
                .jobs
                .get(&job_key(principal_id, &record.job_id))
                .cloned()
                .ok_or_else(|| ReviewStoreError::Corrupt("idempotency target missing".into()));
        }

        let key = job_key(principal_id, &job.job_id);
        if let Some(existing) = index.jobs.get(&key) {
            if existing.job.evidence_digest == job.evidence_digest
                && existing.job.idempotency_key == job.idempotency_key
            {
                return Ok(existing.clone());
            }
            return Err(ReviewStoreError::JobConflict);
        }
        let stored = StoredReview {
            store_schema_version: STORE_SCHEMA_VERSION,
            principal_id: principal_id.to_owned(),
            receipt: GovernedReviewReceipt {
                schema_version: job.schema_version,
                job_id: job.job_id.clone(),
                status: ReviewStatus::Queued,
                evidence_digest: job.evidence_digest.clone(),
                evidence_assessed: vec![],
                findings: vec![],
                proposed_changes: vec![],
                confidence_millis: 0,
                unresolved_conflicts: vec![],
                policy_decision: String::new(),
                runtime_capabilities: vec!["bounded_inference".into(), "terminal_receipt".into()],
                usage: ReviewUsage::default(),
                started_at_unix_ms: None,
                completed_at_unix_ms: None,
                error: None,
            },
            job,
            revision: 1,
        };
        validate_stored(&stored)?;
        let idem = IdempotencyRecord {
            store_schema_version: STORE_SCHEMA_VERSION,
            principal_id: principal_id.to_owned(),
            idempotency_key: stored.job.idempotency_key.clone(),
            evidence_digest: stored.job.evidence_digest.clone(),
            job_id: stored.job.job_id.clone(),
        };

        // The job is written first. If the second write fails, reconstruction
        // sees an unindexed job but never a dangling idempotency authority.
        atomic_json(&self.jobs_dir.join(format!("{key}.json")), &stored)?;
        atomic_json(
            &self.idempotency_dir.join(format!("{idem_key}.json")),
            &idem,
        )?;
        index.jobs.insert(key, stored.clone());
        index.idempotency.insert(idem_key, idem);
        Ok(stored)
    }

    pub async fn find_by_idempotency(
        &self,
        principal_id: &str,
        key: &str,
    ) -> Result<Option<StoredReview>, ReviewStoreError> {
        let index = self.index.lock().await;
        let Some(record) = index.idempotency.get(&idempotency_key(principal_id, key)) else {
            return Ok(None);
        };
        index
            .jobs
            .get(&job_key(principal_id, &record.job_id))
            .cloned()
            .map(Some)
            .ok_or_else(|| ReviewStoreError::Corrupt("idempotency target missing".into()))
    }

    pub async fn get(
        &self,
        principal_id: &str,
        job_id: &str,
    ) -> Result<StoredReview, ReviewStoreError> {
        self.index
            .lock()
            .await
            .jobs
            .get(&job_key(principal_id, job_id))
            .cloned()
            .ok_or(ReviewStoreError::NotFound)
    }

    pub async fn nonterminal(&self) -> Vec<StoredReview> {
        self.index
            .lock()
            .await
            .jobs
            .values()
            .filter(|stored| !stored.receipt.status.is_terminal())
            .cloned()
            .collect()
    }

    pub async fn update_receipt(
        &self,
        principal_id: &str,
        job_id: &str,
        receipt: GovernedReviewReceipt,
    ) -> Result<StoredReview, ReviewStoreError> {
        receipt.validate()?;
        let mut index = self.index.lock().await;
        let key = job_key(principal_id, job_id);
        let current = index.jobs.get(&key).ok_or(ReviewStoreError::NotFound)?;
        if current.job.job_id != receipt.job_id
            || current.job.evidence_digest != receipt.evidence_digest
            || current.job.schema_version != receipt.schema_version
        {
            return Err(ReviewStoreError::Corrupt(
                "receipt identity does not match authoritative job".into(),
            ));
        }
        if current.receipt.status.is_terminal() && current.receipt == receipt {
            return Ok(current.clone());
        }
        if current.receipt.status.is_terminal()
            || !allowed_transition(current.receipt.status, receipt.status)
        {
            return Err(ReviewStoreError::InvalidTransition {
                from: current.receipt.status,
                to: receipt.status,
            });
        }
        let mut next = current.clone();
        next.receipt = receipt;
        next.revision = next.revision.saturating_add(1);
        atomic_json(&self.jobs_dir.join(format!("{key}.json")), &next)?;
        index.jobs.insert(key, next.clone());
        Ok(next)
    }
}

fn allowed_transition(from: ReviewStatus, to: ReviewStatus) -> bool {
    if from == to {
        return true;
    }
    match from {
        ReviewStatus::Queued => to == ReviewStatus::Running || to.is_terminal(),
        ReviewStatus::Running => to.is_terminal(),
        _ => false,
    }
}

fn validate_stored(stored: &StoredReview) -> Result<(), ReviewStoreError> {
    if stored.store_schema_version != STORE_SCHEMA_VERSION || stored.revision == 0 {
        return Err(ReviewStoreError::Corrupt(
            "unsupported store schema or zero revision".into(),
        ));
    }
    require_identity(&stored.principal_id, "principal_id")?;
    stored.job.validate()?;
    stored.receipt.validate()?;
    if stored.job.job_id != stored.receipt.job_id
        || stored.job.evidence_digest != stored.receipt.evidence_digest
        || stored.job.schema_version != stored.receipt.schema_version
    {
        return Err(ReviewStoreError::Corrupt(
            "stored job and receipt identity mismatch".into(),
        ));
    }
    Ok(())
}

fn validate_idempotency(record: &IdempotencyRecord) -> Result<(), ReviewStoreError> {
    if record.store_schema_version != STORE_SCHEMA_VERSION {
        return Err(ReviewStoreError::Corrupt(
            "unsupported idempotency store schema".into(),
        ));
    }
    require_identity(&record.principal_id, "principal_id")?;
    require_identity(&record.idempotency_key, "idempotency_key")?;
    require_identity(&record.job_id, "job_id")?;
    if record.evidence_digest.len() != 64
        || !record
            .evidence_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ReviewStoreError::Corrupt(
            "invalid idempotency evidence digest".into(),
        ));
    }
    Ok(())
}

fn require_identity(value: &str, field: &str) -> Result<(), ReviewStoreError> {
    if value.trim().is_empty() || value.len() > 256 {
        return Err(ReviewStoreError::Corrupt(format!("invalid {field}")));
    }
    Ok(())
}

fn authoritative_json_files(directory: &Path) -> Result<Vec<PathBuf>, ReviewStoreError> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, ReviewStoreError> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| ReviewStoreError::Corrupt(format!("{}: {error}", path.display())))
}

fn private_dir(path: &Path) -> Result<(), std::io::Error> {
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ReviewStoreError> {
    let bytes = serde_json::to_vec(value)?;
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("review path has no parent"))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("review"),
        uuid::Uuid::new_v4()
    ));
    let result = (|| -> Result<(), std::io::Error> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(ReviewStoreError::Io)
}

fn hash(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}

fn job_key(principal_id: &str, job_id: &str) -> String {
    hash(&["job", principal_id, job_id])
}

fn idempotency_key(principal_id: &str, key: &str) -> String {
    hash(&["idempotency", principal_id, key])
}

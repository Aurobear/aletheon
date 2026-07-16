//! Authoritative governed facade for runtime mutation.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fabric::meta::Recommendation;
use fabric::{
    ApprovalCategory, ApprovalId, ApprovalSnapshot, ApprovalStatus, Clock, Evaluation,
    ExecutionPermit, MetaRuntimeOps, MigrationResult, MutationIntent, PermitId, RuntimeCandidate,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

const STATE_SCHEMA_VERSION: u16 = 1;
const APPLY_CAPABILITY: &str = "metacog.apply";
const ROLLBACK_CAPABILITY: &str = "metacog.rollback";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryDisposition {
    Never,
    AfterBackoff,
}

#[derive(Debug, Error)]
pub enum MetacogError {
    #[error("invalid metacog request: {0}")]
    InvalidRequest(String),
    #[error("metacog governance evidence rejected: {0}")]
    Unauthorized(String),
    #[error("metacog mutation conflict: {0}")]
    Conflict(String),
    #[error("metacog mutation was not found")]
    NotFound,
    #[error("metacog verification does not permit adoption: {0}")]
    NotAdoptable(String),
    #[error("metacog runtime stage {stage} failed: {message}")]
    Runtime {
        stage: &'static str,
        message: String,
    },
    #[error("metacog mutation requires reconciliation: {0}")]
    ReconciliationRequired(String),
    #[error("metacog state persistence failed: {0}")]
    Persistence(String),
}

impl MetacogError {
    pub const fn retry_disposition(&self) -> RetryDisposition {
        match self {
            Self::Runtime { .. } | Self::Persistence(_) => RetryDisposition::AfterBackoff,
            Self::InvalidRequest(_)
            | Self::Unauthorized(_)
            | Self::Conflict(_)
            | Self::NotFound
            | Self::NotAdoptable(_)
            | Self::ReconciliationRequired(_) => RetryDisposition::Never,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyMutation {
    pub mutation_id: Uuid,
    pub intent: MutationIntent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationDecision {
    Adopt,
    PartialAdopt,
    Reject,
    NeedsMoreTesting,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationReceipt {
    pub mutation_id: Uuid,
    pub candidate_id: Uuid,
    pub base_version: String,
    pub decision: VerificationDecision,
    pub score: f64,
    pub verification_hash: String,
    pub verified_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernedMutationEvidence {
    pub permit: ExecutionPermit,
    pub approval: ApprovalSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyMutation {
    pub verification: VerificationReceipt,
    pub evidence: GovernedMutationEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackMutation {
    pub mutation_id: Uuid,
    pub applied_receipt_hash: String,
    pub evidence: GovernedMutationEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationOperation {
    Apply,
    Rollback,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutationReceipt {
    pub mutation_id: Uuid,
    pub operation: MutationOperation,
    pub from_version: String,
    pub to_version: String,
    pub verification_hash: String,
    pub permit_id: PermitId,
    pub approval_id: ApprovalId,
    pub receipt_hash: String,
    pub recorded_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationLifecycle {
    Verified,
    Rejected,
    Applying,
    Applied,
    RollbackInProgress,
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationStatus {
    pub mutation_id: Uuid,
    pub lifecycle: MutationLifecycle,
    pub verification: VerificationReceipt,
    pub receipt: Option<MutationReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetacogStatus {
    pub current_version: String,
    pub lineage: Vec<MutationStatus>,
}

#[async_trait]
pub trait MetacogService: Send + Sync {
    async fn verify(&self, request: VerifyMutation) -> Result<VerificationReceipt, MetacogError>;
    async fn apply(&self, request: ApplyMutation) -> Result<MutationReceipt, MetacogError>;
    async fn rollback(&self, request: RollbackMutation) -> Result<MutationReceipt, MetacogError>;
    async fn status(&self) -> Result<MetacogStatus, MetacogError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredMutation {
    request_hash: String,
    verification: VerificationReceipt,
    candidate: RuntimeCandidate,
    evaluation: Evaluation,
    lifecycle: MutationLifecycle,
    apply_receipt: Option<MutationReceipt>,
    rollback_receipt: Option<MutationReceipt>,
}

impl StoredMutation {
    fn status(&self) -> MutationStatus {
        MutationStatus {
            mutation_id: self.verification.mutation_id,
            lifecycle: self.lifecycle,
            verification: self.verification.clone(),
            receipt: self
                .rollback_receipt
                .clone()
                .or_else(|| self.apply_receipt.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedMutationState {
    schema_version: u16,
    mutations: Vec<StoredMutation>,
}

impl Default for PersistedMutationState {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            mutations: Vec::new(),
        }
    }
}

struct MutationStore {
    path: Option<PathBuf>,
    state: PersistedMutationState,
}

impl MutationStore {
    fn in_memory() -> Self {
        Self {
            path: None,
            state: PersistedMutationState::default(),
        }
    }

    fn open(path: PathBuf) -> Result<Self, MetacogError> {
        let state = if path.exists() {
            let bytes = fs::read(&path).map_err(persistence_error)?;
            let state: PersistedMutationState =
                serde_json::from_slice(&bytes).map_err(persistence_error)?;
            if state.schema_version != STATE_SCHEMA_VERSION {
                return Err(MetacogError::Persistence(format!(
                    "unsupported mutation state schema {}",
                    state.schema_version
                )));
            }
            state
        } else {
            PersistedMutationState::default()
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(persistence_error)?;
        }
        Ok(Self {
            path: Some(path),
            state,
        })
    }

    fn persist(&self) -> Result<(), MetacogError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(&self.state).map_err(persistence_error)?;
        let temp = temporary_path(path);
        let write_result = (|| -> Result<(), MetacogError> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)
                .map_err(persistence_error)?;
            file.write_all(&bytes).map_err(persistence_error)?;
            file.sync_all().map_err(persistence_error)?;
            fs::rename(&temp, path).map_err(persistence_error)?;
            if let Some(parent) = path.parent() {
                if let Ok(directory) = OpenOptions::new().read(true).open(parent) {
                    let _ = directory.sync_all();
                }
            }
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        write_result
    }
}

pub struct DefaultMetacogService<M: MetaRuntimeOps> {
    runtime: Arc<M>,
    clock: Arc<dyn Clock>,
    operations: AsyncMutex<()>,
    store: Mutex<MutationStore>,
}

impl<M: MetaRuntimeOps> DefaultMetacogService<M> {
    pub fn in_memory(runtime: Arc<M>, clock: Arc<dyn Clock>) -> Self {
        Self {
            runtime,
            clock,
            operations: AsyncMutex::new(()),
            store: Mutex::new(MutationStore::in_memory()),
        }
    }

    pub fn with_state_path(
        runtime: Arc<M>,
        clock: Arc<dyn Clock>,
        path: PathBuf,
    ) -> Result<Self, MetacogError> {
        Ok(Self {
            runtime,
            clock,
            operations: AsyncMutex::new(()),
            store: Mutex::new(MutationStore::open(path)?),
        })
    }

    fn now_ms(&self) -> i64 {
        self.clock.wall_now().0
    }

    fn validate_evidence(
        &self,
        evidence: &GovernedMutationEvidence,
        mutation_id: Uuid,
        operation: &str,
        binding_name: &str,
        binding_value: &str,
    ) -> Result<(), MetacogError> {
        let expected_capability = match operation {
            "apply" => APPLY_CAPABILITY,
            "rollback" => ROLLBACK_CAPABILITY,
            _ => {
                return Err(MetacogError::InvalidRequest(
                    "unknown mutation operation".into(),
                ))
            }
        };
        if evidence.permit.capability.0 != expected_capability
            || !evidence.permit.is_valid_at(self.clock.mono_now())
        {
            return Err(MetacogError::Unauthorized(
                "missing, expired or mismatched execution permit".into(),
            ));
        }
        evidence
            .approval
            .validate()
            .map_err(|_| MetacogError::Unauthorized("invalid approval snapshot".into()))?;
        if evidence.approval.category != ApprovalCategory::DaseinModification
            || !matches!(
                evidence.approval.status,
                ApprovalStatus::Approved | ApprovalStatus::Consumed
            )
            || self.now_ms() >= evidence.approval.expires_at_ms
        {
            return Err(MetacogError::Unauthorized(
                "approval is not active for runtime mutation".into(),
            ));
        }
        let attributes = &evidence.approval.subject.attributes;
        if attributes.get("mutation_id").map(String::as_str) != Some(&mutation_id.to_string())
            || attributes.get("operation").map(String::as_str) != Some(operation)
            || attributes.get(binding_name).map(String::as_str) != Some(binding_value)
        {
            return Err(MetacogError::Unauthorized(
                "approval subject is not bound to this mutation".into(),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl<M: MetaRuntimeOps + 'static> MetacogService for DefaultMetacogService<M> {
    async fn verify(&self, request: VerifyMutation) -> Result<VerificationReceipt, MetacogError> {
        validate_verify_request(&request)?;
        let _guard = self.operations.lock().await;
        let request_hash = stable_hash(&request)?;
        {
            let store = self.store.lock().map_err(lock_error)?;
            if let Some(existing) = store
                .state
                .mutations
                .iter()
                .find(|item| item.verification.mutation_id == request.mutation_id)
            {
                return if existing.request_hash == request_hash {
                    Ok(existing.verification.clone())
                } else {
                    Err(MetacogError::Conflict(
                        "mutation id already identifies different input".into(),
                    ))
                };
            }
        }

        let base_version = self.runtime.current_version().to_string();
        let candidate = self
            .runtime
            .generate_candidate(&request.intent)
            .await
            .map_err(|error| runtime_error("candidate", error))?;
        let test = match self.runtime.sandbox_test(&candidate).await {
            Ok(test) => test,
            Err(error) => {
                let _ = self.runtime.rollback().await;
                return Err(runtime_error("sandbox", error));
            }
        };
        if !test.passed || test.tests_failed > 0 {
            let _ = self.runtime.rollback().await;
            return Err(MetacogError::NotAdoptable(
                "sandbox verification did not pass".into(),
            ));
        }
        let evaluation = match self.runtime.evaluate(&candidate, &test).await {
            Ok(evaluation) => evaluation,
            Err(error) => {
                let _ = self.runtime.rollback().await;
                return Err(runtime_error("evaluation", error));
            }
        };
        let decision = decision(&evaluation.recommendation);
        let verified_at_ms = self.now_ms();
        let verification_hash = stable_hash(&VerificationHashMaterial {
            mutation_id: request.mutation_id,
            request_hash: &request_hash,
            base_version: &base_version,
            candidate: &candidate,
            evaluation: &evaluation,
            verified_at_ms,
        })?;
        let verification = VerificationReceipt {
            mutation_id: request.mutation_id,
            candidate_id: candidate.id,
            base_version,
            decision: decision.clone(),
            score: evaluation.score,
            verification_hash,
            verified_at_ms,
        };
        let lifecycle = if matches!(
            decision,
            VerificationDecision::Reject | VerificationDecision::NeedsMoreTesting
        ) {
            let _ = self.runtime.rollback().await;
            MutationLifecycle::Rejected
        } else {
            MutationLifecycle::Verified
        };
        let mut store = self.store.lock().map_err(lock_error)?;
        store.state.mutations.push(StoredMutation {
            request_hash,
            verification: verification.clone(),
            candidate,
            evaluation,
            lifecycle,
            apply_receipt: None,
            rollback_receipt: None,
        });
        store.persist()?;
        Ok(verification)
    }

    async fn apply(&self, request: ApplyMutation) -> Result<MutationReceipt, MetacogError> {
        let _guard = self.operations.lock().await;
        let index = {
            let store = self.store.lock().map_err(lock_error)?;
            store
                .state
                .mutations
                .iter()
                .position(|item| item.verification.mutation_id == request.verification.mutation_id)
                .ok_or(MetacogError::NotFound)?
        };
        {
            let store = self.store.lock().map_err(lock_error)?;
            let item = &store.state.mutations[index];
            if item.verification != request.verification {
                return Err(MetacogError::Conflict(
                    "verification receipt does not match durable state".into(),
                ));
            }
            if let Some(receipt) = &item.apply_receipt {
                if receipt.permit_id == request.evidence.permit.id
                    && receipt.approval_id == request.evidence.approval.id
                {
                    return Ok(receipt.clone());
                }
                return Err(MetacogError::Unauthorized(
                    "idempotent apply evidence does not match the durable receipt".into(),
                ));
            }
            if item.lifecycle == MutationLifecycle::Applying {
                return Err(MetacogError::ReconciliationRequired(
                    "a prior apply started without a durable terminal receipt".into(),
                ));
            }
            if item.lifecycle != MutationLifecycle::Verified {
                return Err(MetacogError::NotAdoptable(format!(
                    "mutation is in {:?} state",
                    item.lifecycle
                )));
            }
        }
        self.validate_evidence(
            &request.evidence,
            request.verification.mutation_id,
            "apply",
            "verification_hash",
            &request.verification.verification_hash,
        )?;
        self.ensure_unused_permit(request.evidence.permit.id)?;
        {
            let mut store = self.store.lock().map_err(lock_error)?;
            update_and_persist(&mut store, index, |item| {
                item.lifecycle = MutationLifecycle::Applying;
            })?;
        }
        let candidate = {
            let store = self.store.lock().map_err(lock_error)?;
            store.state.mutations[index].candidate.clone()
        };
        let migration = match self.runtime.migrate(&candidate).await {
            Ok(migration) => migration,
            Err(error) => {
                return Err(MetacogError::ReconciliationRequired(format!(
                    "apply runtime outcome is uncertain: {}",
                    bounded_error(&error.to_string())
                )))
            }
        };
        if !migration.success {
            return Err(MetacogError::ReconciliationRequired(
                "runtime returned a non-successful migration after apply started".into(),
            ));
        }
        let receipt = mutation_receipt(
            request.verification.mutation_id,
            MutationOperation::Apply,
            &migration,
            &request.verification.verification_hash,
            &request.evidence,
            self.now_ms(),
        )?;
        let mut store = self.store.lock().map_err(lock_error)?;
        update_and_persist(&mut store, index, |item| {
            item.lifecycle = MutationLifecycle::Applied;
            item.apply_receipt = Some(receipt.clone());
        })?;
        Ok(receipt)
    }

    async fn rollback(&self, request: RollbackMutation) -> Result<MutationReceipt, MetacogError> {
        let _guard = self.operations.lock().await;
        let index = {
            let store = self.store.lock().map_err(lock_error)?;
            store
                .state
                .mutations
                .iter()
                .position(|item| item.verification.mutation_id == request.mutation_id)
                .ok_or(MetacogError::NotFound)?
        };
        let applied = {
            let store = self.store.lock().map_err(lock_error)?;
            let item = &store.state.mutations[index];
            let applied = item
                .apply_receipt
                .clone()
                .ok_or_else(|| MetacogError::Conflict("mutation has not been applied".into()))?;
            if applied.receipt_hash != request.applied_receipt_hash {
                return Err(MetacogError::Conflict(
                    "applied receipt hash does not match durable state".into(),
                ));
            }
            if let Some(receipt) = &item.rollback_receipt {
                if receipt.permit_id == request.evidence.permit.id
                    && receipt.approval_id == request.evidence.approval.id
                {
                    return Ok(receipt.clone());
                }
                return Err(MetacogError::Unauthorized(
                    "idempotent rollback evidence does not match the durable receipt".into(),
                ));
            }
            if item.lifecycle == MutationLifecycle::RollbackInProgress {
                return Err(MetacogError::ReconciliationRequired(
                    "a prior rollback started without a durable terminal receipt".into(),
                ));
            }
            applied
        };
        self.validate_evidence(
            &request.evidence,
            request.mutation_id,
            "rollback",
            "applied_receipt_hash",
            &request.applied_receipt_hash,
        )?;
        self.ensure_unused_permit(request.evidence.permit.id)?;
        {
            let mut store = self.store.lock().map_err(lock_error)?;
            update_and_persist(&mut store, index, |item| {
                item.lifecycle = MutationLifecycle::RollbackInProgress;
            })?;
        }
        if let Err(error) = self.runtime.rollback().await {
            return Err(MetacogError::ReconciliationRequired(format!(
                "rollback runtime outcome is uncertain: {}",
                bounded_error(&error.to_string())
            )));
        }
        let migration = MigrationResult {
            success: true,
            from_version: applied.to_version.clone(),
            to_version: applied.from_version.clone(),
            memories_migrated: 0,
            identity_preserved: true,
            message: "governed rollback completed".into(),
        };
        let receipt = mutation_receipt(
            request.mutation_id,
            MutationOperation::Rollback,
            &migration,
            &applied.verification_hash,
            &request.evidence,
            self.now_ms(),
        )?;
        let mut store = self.store.lock().map_err(lock_error)?;
        update_and_persist(&mut store, index, |item| {
            item.lifecycle = MutationLifecycle::RolledBack;
            item.rollback_receipt = Some(receipt.clone());
        })?;
        Ok(receipt)
    }

    async fn status(&self) -> Result<MetacogStatus, MetacogError> {
        let store = self.store.lock().map_err(lock_error)?;
        Ok(MetacogStatus {
            current_version: self.runtime.current_version().to_string(),
            lineage: store
                .state
                .mutations
                .iter()
                .map(StoredMutation::status)
                .collect(),
        })
    }
}

impl<M: MetaRuntimeOps> DefaultMetacogService<M> {
    fn ensure_unused_permit(&self, permit_id: PermitId) -> Result<(), MetacogError> {
        let store = self.store.lock().map_err(lock_error)?;
        if store.state.mutations.iter().any(|item| {
            item.apply_receipt
                .iter()
                .chain(item.rollback_receipt.iter())
                .any(|receipt| receipt.permit_id == permit_id)
        }) {
            return Err(MetacogError::Unauthorized(
                "execution permit was already consumed".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct VerificationHashMaterial<'a> {
    mutation_id: Uuid,
    request_hash: &'a str,
    base_version: &'a str,
    candidate: &'a RuntimeCandidate,
    evaluation: &'a Evaluation,
    verified_at_ms: i64,
}

#[derive(Serialize)]
struct ReceiptHashMaterial<'a> {
    mutation_id: Uuid,
    operation: MutationOperation,
    from_version: &'a str,
    to_version: &'a str,
    verification_hash: &'a str,
    permit_id: PermitId,
    approval_id: ApprovalId,
    recorded_at_ms: i64,
}

fn mutation_receipt(
    mutation_id: Uuid,
    operation: MutationOperation,
    migration: &MigrationResult,
    verification_hash: &str,
    evidence: &GovernedMutationEvidence,
    recorded_at_ms: i64,
) -> Result<MutationReceipt, MetacogError> {
    let receipt_hash = stable_hash(&ReceiptHashMaterial {
        mutation_id,
        operation,
        from_version: &migration.from_version,
        to_version: &migration.to_version,
        verification_hash,
        permit_id: evidence.permit.id,
        approval_id: evidence.approval.id,
        recorded_at_ms,
    })?;
    Ok(MutationReceipt {
        mutation_id,
        operation,
        from_version: migration.from_version.clone(),
        to_version: migration.to_version.clone(),
        verification_hash: verification_hash.into(),
        permit_id: evidence.permit.id,
        approval_id: evidence.approval.id,
        receipt_hash,
        recorded_at_ms,
    })
}

fn update_and_persist(
    store: &mut MutationStore,
    index: usize,
    update: impl FnOnce(&mut StoredMutation),
) -> Result<(), MetacogError> {
    let previous = store.state.mutations[index].clone();
    update(&mut store.state.mutations[index]);
    if let Err(error) = store.persist() {
        store.state.mutations[index] = previous;
        return Err(error);
    }
    Ok(())
}

fn validate_verify_request(request: &VerifyMutation) -> Result<(), MetacogError> {
    if request.mutation_id.is_nil()
        || request.intent.target.trim().is_empty()
        || request.intent.reason.trim().is_empty()
        || request.intent.target.len() > 256
        || request.intent.reason.len() > 4 * 1024
    {
        return Err(MetacogError::InvalidRequest(
            "mutation id, target and bounded reason are required".into(),
        ));
    }
    Ok(())
}

fn decision(recommendation: &Recommendation) -> VerificationDecision {
    match recommendation {
        Recommendation::Adopt => VerificationDecision::Adopt,
        Recommendation::PartialAdopt { .. } => VerificationDecision::PartialAdopt,
        Recommendation::Reject => VerificationDecision::Reject,
        Recommendation::NeedsMoreTesting => VerificationDecision::NeedsMoreTesting,
    }
}

fn stable_hash<T: Serialize>(value: &T) -> Result<String, MetacogError> {
    let bytes = serde_json::to_vec(value).map_err(persistence_error)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn temporary_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("metacog-state");
    path.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()))
}

fn runtime_error(stage: &'static str, error: anyhow::Error) -> MetacogError {
    MetacogError::Runtime {
        stage,
        message: bounded_error(&error.to_string()),
    }
}

fn persistence_error(error: impl std::fmt::Display) -> MetacogError {
    MetacogError::Persistence(bounded_error(&error.to_string()))
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> MetacogError {
    MetacogError::Persistence("mutation state lock poisoned".into())
}

fn bounded_error(message: &str) -> String {
    message
        .chars()
        .filter(|character| !character.is_control())
        .take(512)
        .collect()
}

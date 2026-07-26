//! Version-bound practical-work transaction contracts.

use crate::AgentToolContext;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChangeTransactionId(pub Uuid);

impl ChangeTransactionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ChangeTransactionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceVersion {
    pub digest: String,
    pub basis: WorkspaceVersionBasis,
    pub root: String,
    pub head: Option<String>,
    pub changed_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedRange {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceVersionBasis {
    GitWorktree,
    BoundedTree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeTransactionPhase {
    Baseline,
    Applied,
    DiffReviewed,
    Validated,
    Repair,
    Accepted,
    RolledBack,
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeTransactionSnapshot {
    pub transaction_id: ChangeTransactionId,
    pub owner_session_id: String,
    pub owner_agent: Option<AgentToolContext>,
    pub root: String,
    pub baseline: WorkspaceVersion,
    pub current: WorkspaceVersion,
    pub phase: ChangeTransactionPhase,
    pub changed_paths: Vec<String>,
    pub changed_ranges: Vec<ChangedRange>,
    pub diff_artifact_ref: Option<String>,
    pub validation_plan: Vec<ValidationPlanStep>,
    pub validation_omissions: Vec<ValidationPlanOmission>,
    pub validation_impact: ValidationImpact,
    pub validation_risk: ValidationRisk,
    pub validation_receipts: Vec<VersionedValidationReceipt>,
    pub accepted_workspace_version: Option<String>,
    pub active_command: Option<ActiveCommandLease>,
    pub failure: Option<WorkFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveCommandLease {
    pub session_id: String,
    pub purpose: String,
    pub workspace_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationPlanStep {
    pub id: String,
    pub validation_kind: String,
    pub command: String,
    pub reason: String,
    pub source: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationPlanOmission {
    pub validation_kind: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationImpact {
    NonCode,
    PackageLocal,
    WorkspaceDependency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationRisk {
    Low,
    Moderate,
    High,
    DeploymentCritical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionedValidationReceipt {
    pub validation_kind: String,
    pub command: String,
    pub workspace_version: String,
    pub terminal_status: String,
    pub output_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkFailureClass {
    Implementation,
    TestExpectation,
    Environment,
    Dependency,
    Permission,
    Provider,
    Timeout,
    ConcurrentModification,
    InvalidToolRequest,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkFailure {
    pub class: WorkFailureClass,
    pub summary: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkRecoveryDirective {
    RepairImplementation,
    ReviewTestExpectation,
    CollectEnvironmentEvidence,
    InspectDependency,
    RequestAuthority,
    HonorProviderBackpressure,
    AdjustBoundedExecution,
    ReinspectWorkspace,
    CorrectArguments,
    BroadenDiagnosis,
}

impl WorkFailure {
    pub fn recovery(&self) -> WorkRecoveryDirective {
        match self.class {
            WorkFailureClass::Implementation => WorkRecoveryDirective::RepairImplementation,
            WorkFailureClass::TestExpectation => WorkRecoveryDirective::ReviewTestExpectation,
            WorkFailureClass::Environment => WorkRecoveryDirective::CollectEnvironmentEvidence,
            WorkFailureClass::Dependency => WorkRecoveryDirective::InspectDependency,
            WorkFailureClass::Permission => WorkRecoveryDirective::RequestAuthority,
            WorkFailureClass::Provider => WorkRecoveryDirective::HonorProviderBackpressure,
            WorkFailureClass::Timeout => WorkRecoveryDirective::AdjustBoundedExecution,
            WorkFailureClass::ConcurrentModification => WorkRecoveryDirective::ReinspectWorkspace,
            WorkFailureClass::InvalidToolRequest => WorkRecoveryDirective::CorrectArguments,
            WorkFailureClass::Unknown => WorkRecoveryDirective::BroadenDiagnosis,
        }
    }
}

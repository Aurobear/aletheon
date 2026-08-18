//! Request-safe administrative use cases.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use application::turn_control::{CollaborationMode, InterruptReason};
use async_trait::async_trait;
use corpus::security::approval::ApprovalDecision;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{oneshot, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::host::request_use_cases::MemoryAdminUseCases;

const MAX_ADMIN_ITEMS: usize = 200;
const AGENT_PROFILE_SWITCH_EVENT_SCHEMA_V1: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProfileSwitchDecision {
    Accepted,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProfileSwitchEventV1 {
    pub schema_version: u16,
    pub previous_profile: String,
    pub requested_profile: String,
    pub previous_risk_tier: ::contracts::RiskTier,
    pub requested_risk_tier: ::contracts::RiskTier,
    pub decision: AgentProfileSwitchDecision,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModeChange {
    pub old: CollaborationMode,
    pub new: CollaborationMode,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelDescriptor {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelCatalog {
    pub models: Vec<ModelDescriptor>,
    pub current: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct HookDescriptor {
    pub name: String,
    pub source: String,
    pub point: String,
    pub priority: i32,
    pub script_path: Option<std::path::PathBuf>,
}

#[async_trait]
pub trait AgentTimelinePort: Send + Sync {
    async fn read_agent_timeline(
        &self,
        root_agent_id: ::contracts::AgentId,
        agent_id: ::contracts::AgentId,
        limit: usize,
    ) -> Result<Vec<::contracts::protocol::client::AgentTimelineEntry>, AdminServiceError>;
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentProfileDescriptor {
    pub name: String,
    pub risk_tier: String,
    pub tool_count: usize,
    pub max_iterations: usize,
    pub approval_policy: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentProfileSwitchResult {
    pub previous: String,
    pub current: String,
    pub risk_tier: String,
}

#[derive(Clone, Debug)]
pub struct TransientApprovalRequest {
    /// Principal authenticated by the transport adapter.
    pub principal_id: ::contracts::PrincipalId,
    pub connection_id: ::contracts::ConnectionId,
    pub approval_id: String,
    pub decision: ::contracts::protocol::client::TransientApprovalDecision,
    pub scope_hint: Option<::contracts::protocol::client::TransientApprovalScopeHint>,
}

pub use ::contracts::{ApprovalOwner, PendingApprovalKey, ThreadGrantKey};

struct PendingApprovalRecord {
    connection_id: ::contracts::ConnectionId,
    tool: String,
    respond: oneshot::Sender<ApprovalDecision>,
    scope_subject: Option<::contracts::protocol::client::TransientApprovalScopeSubject>,
}

#[derive(Debug, Error)]
pub enum PendingApprovalError {
    #[error("approval is not owned by authenticated principal")]
    WrongOwner,
    #[error("approval is not pending")]
    NotFound,
}

#[derive(Debug)]
pub struct ResolvedPendingApproval {
    pub owner: ApprovalOwner,
    pub tool: String,
    pub delivery: ApprovalDecisionDelivery,
    pub scope_subject: Option<::contracts::protocol::client::TransientApprovalScopeSubject>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalDecisionDelivery {
    Delivered,
    ConsumerGone,
}

#[derive(Clone, Default)]
pub struct PendingApprovals {
    inner: Arc<Mutex<HashMap<PendingApprovalKey, PendingApprovalRecord>>>,
}

/// Narrow consumer port for transient-approval scope checks used by the daemon
/// transport and the typed Gateway.
#[async_trait]
pub trait PendingApprovalsPort: Send + Sync {
    async fn scope_subject_authenticated(
        &self,
        principal_id: &::contracts::PrincipalId,
        connection_id: &::contracts::ConnectionId,
        approval_id: &str,
    ) -> Result<
        Option<::contracts::protocol::client::TransientApprovalScopeSubject>,
        PendingApprovalError,
    >;
    async fn cancel_connection(&self, connection_id: &::contracts::ConnectionId) -> usize;
}

#[async_trait]
impl PendingApprovalsPort for PendingApprovals {
    async fn scope_subject_authenticated(
        &self,
        principal_id: &::contracts::PrincipalId,
        connection_id: &::contracts::ConnectionId,
        approval_id: &str,
    ) -> Result<
        Option<::contracts::protocol::client::TransientApprovalScopeSubject>,
        PendingApprovalError,
    > {
        PendingApprovals::scope_subject_authenticated(
            self,
            principal_id,
            connection_id,
            approval_id,
        )
        .await
    }
    async fn cancel_connection(&self, connection_id: &::contracts::ConnectionId) -> usize {
        PendingApprovals::cancel_connection(self, connection_id).await
    }
}

impl PendingApprovals {
    pub async fn scope_subject_authenticated(
        &self,
        principal_id: &::contracts::PrincipalId,
        connection_id: &::contracts::ConnectionId,
        approval_id: &str,
    ) -> Result<
        Option<::contracts::protocol::client::TransientApprovalScopeSubject>,
        PendingApprovalError,
    > {
        let pending = self.inner.lock().await;
        let (key, record) = pending
            .iter()
            .find(|(key, _)| key.approval_id == approval_id)
            .ok_or(PendingApprovalError::NotFound)?;
        if &key.owner.principal_id != principal_id || &record.connection_id != connection_id {
            return Err(PendingApprovalError::WrongOwner);
        }
        Ok(record.scope_subject.clone())
    }
    pub async fn insert(
        &self,
        owner: ApprovalOwner,
        turn_id: ::contracts::TurnId,
        call_id: String,
        tool: String,
        connection_id: ::contracts::ConnectionId,
        respond: oneshot::Sender<ApprovalDecision>,
    ) -> String {
        self.insert_scoped(owner, turn_id, call_id, tool, connection_id, None, respond)
            .await
    }

    pub async fn insert_scoped(
        &self,
        owner: ApprovalOwner,
        turn_id: ::contracts::TurnId,
        call_id: String,
        tool: String,
        connection_id: ::contracts::ConnectionId,
        scope_subject: Option<::contracts::protocol::client::TransientApprovalScopeSubject>,
        respond: oneshot::Sender<ApprovalDecision>,
    ) -> String {
        let approval_id = uuid::Uuid::new_v4().to_string();
        self.inner.lock().await.insert(
            PendingApprovalKey {
                owner,
                turn_id,
                call_id,
                approval_id: approval_id.clone(),
            },
            PendingApprovalRecord {
                connection_id,
                tool,
                respond,
                scope_subject,
            },
        );
        approval_id
    }

    pub async fn resolve(
        &self,
        owner: &ApprovalOwner,
        approval_id: &str,
        decision: ApprovalDecision,
    ) -> Result<ResolvedPendingApproval, PendingApprovalError> {
        let mut pending = self.inner.lock().await;
        let key = pending
            .keys()
            .find(|key| key.approval_id == approval_id && &key.owner == owner)
            .cloned();
        let Some(key) = key else {
            return if pending.keys().any(|key| key.approval_id == approval_id) {
                Err(PendingApprovalError::WrongOwner)
            } else {
                Err(PendingApprovalError::NotFound)
            };
        };
        let record = pending
            .remove(&key)
            .expect("pending key was selected while holding the same lock");
        let delivery = if record.respond.send(decision).is_ok() {
            ApprovalDecisionDelivery::Delivered
        } else {
            warn!(
                approval_id,
                "approval decision consumer was already gone; pending request closed"
            );
            ApprovalDecisionDelivery::ConsumerGone
        };
        Ok(ResolvedPendingApproval {
            owner: key.owner,
            tool: record.tool,
            delivery,
            scope_subject: record.scope_subject,
        })
    }

    /// Legacy approval responses authenticate a principal but do not carry a
    /// client-authoritative thread. Recover the exact thread only from the
    /// pending key after verifying the authenticated principal.
    pub async fn resolve_authenticated(
        &self,
        principal_id: &::contracts::PrincipalId,
        connection_id: &::contracts::ConnectionId,
        approval_id: &str,
        decision: ApprovalDecision,
    ) -> Result<ResolvedPendingApproval, PendingApprovalError> {
        let owner = {
            let pending = self.inner.lock().await;
            let key = pending
                .keys()
                .find(|key| key.approval_id == approval_id)
                .ok_or(PendingApprovalError::NotFound)?;
            if &key.owner.principal_id != principal_id {
                return Err(PendingApprovalError::WrongOwner);
            }
            let record = pending
                .get(key)
                .expect("pending key and record are read under the same lock");
            if &record.connection_id != connection_id {
                return Err(PendingApprovalError::WrongOwner);
            }
            key.owner.clone()
        };
        self.resolve(&owner, approval_id, decision).await
    }

    /// Fail closed every still-pending request owned by a disconnected
    /// transport connection. Requests belonging to other live connections are
    /// left untouched.
    pub async fn cancel_connection(&self, connection_id: &::contracts::ConnectionId) -> usize {
        let mut pending = self.inner.lock().await;
        let keys = pending
            .iter()
            .filter_map(|(key, record)| {
                (&record.connection_id == connection_id).then_some(key.clone())
            })
            .collect::<Vec<_>>();
        for key in &keys {
            if let Some(record) = pending.remove(key) {
                if record.respond.send(ApprovalDecision::Deny).is_err() {
                    warn!(
                        approval_id = key.approval_id,
                        "disconnected approval consumer was already gone"
                    );
                }
            }
        }
        keys.len()
    }
}

#[derive(Clone)]
pub struct ScopedApprovalCache {
    store: Arc<dyn application::approval::ScopedApprovalGrantStore>,
}

impl ScopedApprovalCache {
    pub fn new(store: Arc<dyn application::approval::ScopedApprovalGrantStore>) -> Self {
        Self { store }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(i64::MAX as u128) as i64
    }

    fn expiry_ms() -> i64 {
        Self::now_ms().saturating_add(24 * 60 * 60 * 1_000)
    }

    pub async fn clear(&self) {
        let _ = self.store.clear();
    }

    pub async fn allow_for_thread(
        &self,
        principal_id: ::contracts::PrincipalId,
        thread_id: ::contracts::ThreadId,
        tool: impl Into<String>,
    ) {
        let _ = self.store.grant(
            &application::approval::ScopedGrantKey {
                principal_id,
                thread_id,
                tool: tool.into(),
                path_root: String::new(),
                subject_version: 0,
                subject_sha256: String::new(),
            },
            Self::expiry_ms(),
        );
    }

    pub async fn is_allowed(
        &self,
        principal_id: &::contracts::PrincipalId,
        thread_id: &::contracts::ThreadId,
        tool: &str,
    ) -> bool {
        self.store
            .has_tool_grant(principal_id, thread_id, tool, Self::now_ms())
            .unwrap_or(false)
    }

    pub async fn allow_path_for_thread(
        &self,
        principal_id: ::contracts::PrincipalId,
        thread_id: ::contracts::ThreadId,
        tool: &str,
        hint: &::contracts::protocol::client::TransientApprovalScopeHint,
    ) -> anyhow::Result<()> {
        self.store.grant(
            &application::approval::ScopedGrantKey {
                principal_id,
                thread_id,
                tool: tool.to_owned(),
                path_root: hint.path_root.to_string_lossy().into_owned(),
                subject_version: hint.subject_version,
                subject_sha256: hint.subject_sha256.clone(),
            },
            Self::expiry_ms(),
        )
    }

    pub async fn is_path_allowed(
        &self,
        principal_id: &::contracts::PrincipalId,
        thread_id: &::contracts::ThreadId,
        tool: &str,
        subject: &::contracts::protocol::client::TransientApprovalScopeSubject,
    ) -> bool {
        let roots = self
            .store
            .path_roots(
                principal_id,
                thread_id,
                tool,
                subject.subject_version,
                &subject.subject_sha256,
                Self::now_ms(),
            )
            .unwrap_or_default();
        roots.iter().any(|root| {
            subject
                .path_candidates
                .iter()
                .any(|candidate| candidate == std::path::Path::new(root))
        })
    }
}

impl Default for ScopedApprovalCache {
    fn default() -> Self {
        Self::new(Arc::new(
            application::approval::InMemoryScopedApprovalGrantStore::default(),
        ))
    }
}

#[derive(Debug, Error)]
pub enum AdminServiceError {
    #[error("admin operation failed: {0}")]
    Operation(String),
}

#[async_trait]
pub trait AdminUseCases: Send + Sync {
    async fn shutdown(&self) -> Result<(), AdminServiceError>;
    async fn reload_skills(&self) -> Result<usize, AdminServiceError>;
    async fn resolve_transient_approval(
        &self,
        request: TransientApprovalRequest,
    ) -> Result<bool, AdminServiceError>;
    async fn interrupt(&self, reason: InterruptReason) -> Result<(), AdminServiceError>;
    async fn switch_mode(&self, mode: CollaborationMode) -> Result<ModeChange, AdminServiceError>;
    async fn model_catalog(&self) -> Result<ModelCatalog, AdminServiceError>;
    async fn switch_model(&self, model: String) -> Result<String, AdminServiceError>;
    async fn tools(&self) -> Result<Vec<::contracts::ToolDefinition>, AdminServiceError>;
    async fn hooks(&self) -> Result<Vec<HookDescriptor>, AdminServiceError>;
    async fn list_skills(&self) -> Vec<SkillDescriptor>;
    async fn sub_agents(
        &self,
    ) -> Result<Vec<::contracts::protocol::client::AgentSessionSnapshot>, AdminServiceError>;
    async fn list_agent_profiles(&self) -> Result<Vec<AgentProfileDescriptor>, AdminServiceError>;
    async fn switch_agent_profile(
        &self,
        profile_name: String,
    ) -> Result<AgentProfileSwitchResult, AdminServiceError>;
    async fn preview_memory_forget(
        &self,
        policy: mnemosyne::ForgetPolicy,
    ) -> Result<mnemosyne::ForgetReceipt, AdminServiceError>;
    async fn forget_memory(
        &self,
        policy: mnemosyne::ForgetPolicy,
    ) -> Result<mnemosyne::ForgetReceipt, AdminServiceError>;
    async fn compact_memory_retention(
        &self,
        owner: &str,
        now_ms: i64,
        policy: mnemosyne::RetentionCompactionPolicy,
    ) -> Result<mnemosyne::RetentionCompactionReport, AdminServiceError>;
    async fn rollback_deployment(
        &self,
        expected_installed_sha: String,
    ) -> Result<platform::DeploymentRollbackReceipt, AdminServiceError>;
}

#[async_trait]
pub trait DeploymentRollbackPort: Send + Sync {
    async fn execute(
        &self,
        expected_installed_sha: String,
    ) -> Result<platform::DeploymentRollbackReceipt, AdminServiceError>;
}

#[async_trait]
impl DeploymentRollbackPort for platform::DeploymentRollbackService {
    async fn execute(
        &self,
        expected_installed_sha: String,
    ) -> Result<platform::DeploymentRollbackReceipt, AdminServiceError> {
        let service = self.clone();
        tokio::task::spawn_blocking(move || service.execute_recommended(&expected_installed_sha))
            .await
            .map_err(|error| {
                AdminServiceError::Operation(format!("rollback worker failed: {error}"))
            })?
            .map_err(|error| AdminServiceError::Operation(error.to_string()))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub extension_id: String,
}

#[async_trait]
pub trait SkillAdminPort: Send + Sync {
    async fn reload(&self) -> Result<usize, AdminServiceError>;
    async fn list(&self) -> Vec<SkillDescriptor>;
}

/// Application-facing projection port for package-provided skills.
///
/// The admin use case consumes this narrow projection instead of depending on
/// a concrete extension runtime snapshot. This lets the Aletheon composition
/// root own extension lifecycle while this use case retains only a typed
/// projection.
#[async_trait]
pub trait ExtensionSkillCatalogPort: Send + Sync {
    async fn list_extension_skills(&self) -> Vec<SkillDescriptor>;
}

#[async_trait]
impl ExtensionSkillCatalogPort for crate::extensions::extension_snapshot::ExtensionRuntimeView {
    async fn list_extension_skills(&self) -> Vec<SkillDescriptor> {
        let snapshot = self.load().await;
        snapshot
            .skills
            .iter()
            .filter_map(|skill| {
                let package_asset = skill.source.strip_prefix("package:")?;
                let (package_id, _) = package_asset.rsplit_once(':')?;
                Some(SkillDescriptor {
                    id: format!("{package_id}:{}", skill.name),
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    enabled: true,
                    extension_id: package_id.to_owned(),
                })
            })
            .collect()
    }
}

#[async_trait]
pub trait ProfileSwitchEventSink: Send + Sync {
    async fn record(&self, event: AgentProfileSwitchEventV1);
}

#[derive(Debug, Default)]
pub struct NoopProfileSwitchEventSink;

#[async_trait]
impl ProfileSwitchEventSink for NoopProfileSwitchEventSink {
    async fn record(&self, _event: AgentProfileSwitchEventV1) {}
}

pub struct SpineProfileSwitchEventSink {
    spine: Arc<dyn runtime::EventSpine>,
}

impl SpineProfileSwitchEventSink {
    pub fn new(spine: Arc<dyn runtime::EventSpine>) -> Self {
        Self { spine }
    }
}

#[async_trait]
impl ProfileSwitchEventSink for SpineProfileSwitchEventSink {
    async fn record(&self, event: AgentProfileSwitchEventV1) {
        let payload = match serde_json::to_value(&event) {
            Ok(payload) => payload,
            Err(error) => {
                warn!(%error, "failed to encode profile switch event");
                return;
            }
        };
        let root = "daemon-admin";
        let envelope = ::contracts::EnvelopeV2::new(
            ::contracts::SchemaId::from(::contracts::SchemaId::EVENT_SANDBOX_PROFILE_APPLIED_V1),
            ::contracts::EnvelopeV2Target("admin:profile".into()),
            ::contracts::EnvelopeV2Target("daemon:admin".into()),
            ::contracts::EnvelopeV2Delivery::Direct,
            ::contracts::NamespaceId("daemon:admin".into()),
            payload.clone(),
        );
        if let Err(error) = self.spine.append(runtime::UnsequencedEvent {
            tree_id: runtime::EventTreeId::for_root_session(root),
            event_id: runtime::EventId::new(),
            parent: None,
            identity: runtime::EventIdentity {
                root_session_id: root.into(),
                session_id: root.into(),
                agent_id: None,
            },
            envelope,
            visibility: runtime::EventVisibility::Control,
            payload: runtime::EventPayload::Inline { value: payload },
        }) {
            warn!(%error, "failed to append profile switch event");
        }
    }
}

pub struct AdminResources {
    pub runtime: Arc<dyn AdminRuntimePort>,
    pub skills: Arc<dyn SkillAdminPort>,
    pub tool_catalog: Arc<dyn Fn() -> ToolCatalogFuture + Send + Sync>,
    pub hook_catalog: Arc<dyn Fn() -> HookCatalogFuture + Send + Sync>,
    pub pending_approvals: PendingApprovals,
    pub session_approvals: ScopedApprovalCache,
    pub daemon_cancel: CancellationToken,
    pub external_sync: Option<Arc<dyn BackgroundWorkerPort>>,
    pub supplemental_memory_worker: Option<Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>>,
    pub goal_worker: Option<Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>>,
    pub runtime_shutdown: Arc<dyn Fn() -> RuntimeShutdownFuture + Send + Sync>,
    pub memory_admin: Option<Arc<dyn MemoryAdminUseCases>>,
    pub agent_runs: Option<Arc<dyn crate::composition::agent_control::AgentRunProjection>>,
    pub agent_timeline: Option<Arc<dyn AgentTimelinePort>>,
    pub agent_profiles: Option<Arc<dyn AgentProfileCatalogPort>>,
    pub current_profile: Option<Arc<tokio::sync::Mutex<String>>>,
    pub profile_switch_events: Arc<dyn ProfileSwitchEventSink>,
    pub deployment_rollback: Option<Arc<dyn DeploymentRollbackPort>>,
}

#[async_trait]
pub trait BackgroundWorkerPort: Send + Sync {
    async fn is_running(&self) -> bool;
    async fn shutdown(&self);
}

pub trait AgentProfileCatalogPort: Send + Sync {
    fn names(&self) -> Vec<String>;
    fn resolve_profile(&self, name: &str) -> Result<::contracts::AgentProfile, AdminServiceError>;
}

#[async_trait]
pub trait AdminRuntimePort: Send + Sync {
    async fn request_interrupt(&self, reason: InterruptReason);
    async fn switch_mode(&self, mode: CollaborationMode) -> ModeChange;
}

pub type ToolCatalogFuture =
    Pin<Box<dyn Future<Output = Vec<::contracts::ToolDefinition>> + Send + 'static>>;
pub type HookCatalogFuture = Pin<Box<dyn Future<Output = Vec<HookDescriptor>> + Send + 'static>>;
pub type RuntimeShutdownFuture =
    Pin<Box<dyn Future<Output = Result<(), AdminServiceError>> + Send + 'static>>;

pub struct AdminService {
    resources: AdminResources,
    extension_runtime: Option<Arc<dyn ExtensionSkillCatalogPort>>,
}

impl AdminService {
    pub fn new(resources: AdminResources) -> Self {
        Self {
            resources,
            extension_runtime: None,
        }
    }

    pub fn with_extension_runtime(
        self,
        runtime: crate::extensions::extension_snapshot::ExtensionRuntimeView,
    ) -> Self {
        self.with_extension_catalog(Arc::new(runtime))
    }

    /// Inject the extension skill projection without coupling the application
    /// service to a concrete extension runtime implementation.
    pub fn with_extension_catalog(mut self, catalog: Arc<dyn ExtensionSkillCatalogPort>) -> Self {
        self.extension_runtime = Some(catalog);
        self
    }
}

fn authorize_agent_profile_switch(
    current: &::contracts::AgentProfile,
    requested: &::contracts::AgentProfile,
) -> Result<(), AdminServiceError> {
    // A foreground profile switch is not child delegation: the operator is
    // replacing the active authority, rather than granting a child a subset of
    // the current profile's tools.  Reusing `allows_child` here rejects a
    // strictly safer profile whenever it exposes a different read-only tool.
    if current.id == requested.id || requested.risk_tier <= current.risk_tier {
        return Ok(());
    }
    Err(AdminServiceError::Operation(format!(
        "profile switch from '{}' ({:?}) to '{}' ({:?}) would escalate authority",
        current.profile_name, current.risk_tier, requested.profile_name, requested.risk_tier
    )))
}

fn profile_switch_event(
    current: &::contracts::AgentProfile,
    requested: &::contracts::AgentProfile,
    decision: AgentProfileSwitchDecision,
    reason: Option<String>,
) -> AgentProfileSwitchEventV1 {
    AgentProfileSwitchEventV1 {
        schema_version: AGENT_PROFILE_SWITCH_EVENT_SCHEMA_V1,
        previous_profile: current.profile_name.clone(),
        requested_profile: requested.profile_name.clone(),
        previous_risk_tier: current.risk_tier,
        requested_risk_tier: requested.risk_tier,
        decision,
        reason,
    }
}

#[async_trait]
impl AdminUseCases for AdminService {
    async fn shutdown(&self) -> Result<(), AdminServiceError> {
        self.resources.daemon_cancel.cancel();
        if let Some(sync) = &self.resources.external_sync {
            sync.shutdown().await;
        }
        for (name, worker) in [
            (
                "Supplemental memory",
                &self.resources.supplemental_memory_worker,
            ),
            ("Goal", &self.resources.goal_worker),
        ] {
            if let Some(worker) = worker {
                if let Some(task) = worker.lock().await.take() {
                    if tokio::time::timeout(Duration::from_secs(5), task)
                        .await
                        .is_err()
                    {
                        warn!(worker = name, "worker did not stop within shutdown bound");
                    }
                }
            }
        }
        (self.resources.runtime_shutdown)().await?;
        Ok(())
    }

    async fn reload_skills(&self) -> Result<usize, AdminServiceError> {
        self.resources.skills.reload().await
    }

    async fn resolve_transient_approval(
        &self,
        request: TransientApprovalRequest,
    ) -> Result<bool, AdminServiceError> {
        let scope_subject = self
            .resources
            .pending_approvals
            .scope_subject_authenticated(
                &request.principal_id,
                &request.connection_id,
                &request.approval_id,
            )
            .await
            .map_err(|error| AdminServiceError::Operation(error.to_string()))?;
        let decision = match request.decision {
            ::contracts::protocol::client::TransientApprovalDecision::Approve => {
                ApprovalDecision::Approve
            }
            ::contracts::protocol::client::TransientApprovalDecision::ApproveForSession => {
                ApprovalDecision::ApproveForSession
            }
            ::contracts::protocol::client::TransientApprovalDecision::Deny => {
                ApprovalDecision::Deny
            }
            ::contracts::protocol::client::TransientApprovalDecision::ApprovePathForSession => {
                let subject = scope_subject.as_ref().ok_or_else(|| {
                    AdminServiceError::Operation("scoped approval is not advertised".into())
                })?;
                let hint = request.scope_hint.as_ref().ok_or_else(|| {
                    AdminServiceError::Operation("scoped approval hint is required".into())
                })?;
                if hint.subject_version != subject.subject_version
                    || hint.subject_sha256 != subject.subject_sha256
                    || !subject.path_candidates.contains(&hint.path_root)
                {
                    return Err(AdminServiceError::Operation(
                        "scoped approval hint does not match pending subject".into(),
                    ));
                }
                ApprovalDecision::ApprovePathForSession
            }
        };
        let resolved = self
            .resources
            .pending_approvals
            .resolve_authenticated(
                &request.principal_id,
                &request.connection_id,
                &request.approval_id,
                decision,
            )
            .await
            .map_err(|error| AdminServiceError::Operation(error.to_string()))?;
        if resolved.delivery == ApprovalDecisionDelivery::ConsumerGone {
            return Ok(false);
        }
        if decision == ApprovalDecision::ApproveForSession {
            self.resources
                .session_approvals
                .allow_for_thread(
                    resolved.owner.principal_id,
                    resolved.owner.thread_id,
                    resolved.tool,
                )
                .await;
        } else if decision == ApprovalDecision::ApprovePathForSession {
            self.resources
                .session_approvals
                .allow_path_for_thread(
                    resolved.owner.principal_id,
                    resolved.owner.thread_id,
                    &resolved.tool,
                    request.scope_hint.as_ref().expect("validated scope hint"),
                )
                .await
                .map_err(|error| AdminServiceError::Operation(error.to_string()))?;
        }
        Ok(true)
    }

    async fn interrupt(&self, reason: InterruptReason) -> Result<(), AdminServiceError> {
        self.resources.runtime.request_interrupt(reason).await;
        Ok(())
    }

    async fn switch_mode(&self, mode: CollaborationMode) -> Result<ModeChange, AdminServiceError> {
        Ok(self.resources.runtime.switch_mode(mode).await)
    }

    async fn model_catalog(&self) -> Result<ModelCatalog, AdminServiceError> {
        Ok(ModelCatalog {
            models: [
                ("default", "Default model from config"),
                ("sonnet", "Claude Sonnet"),
                ("opus", "Claude Opus"),
                ("haiku", "Claude Haiku"),
            ]
            .into_iter()
            .map(|(name, description)| ModelDescriptor {
                name: name.into(),
                description: description.into(),
            })
            .collect(),
            current: "default".into(),
        })
    }

    async fn switch_model(&self, model: String) -> Result<String, AdminServiceError> {
        Ok(model)
    }

    async fn tools(&self) -> Result<Vec<::contracts::ToolDefinition>, AdminServiceError> {
        let mut tools = (self.resources.tool_catalog)().await;
        tools.truncate(MAX_ADMIN_ITEMS);
        Ok(tools)
    }

    async fn hooks(&self) -> Result<Vec<HookDescriptor>, AdminServiceError> {
        let mut hooks = (self.resources.hook_catalog)().await;
        hooks.truncate(MAX_ADMIN_ITEMS);
        Ok(hooks)
    }

    async fn list_skills(&self) -> Vec<SkillDescriptor> {
        let mut skills = self.resources.skills.list().await;
        if let Some(runtime) = &self.extension_runtime {
            skills.extend(runtime.list_extension_skills().await);
        }
        skills.sort_by(|left, right| left.id.cmp(&right.id));
        skills.truncate(MAX_ADMIN_ITEMS);
        skills
    }

    async fn sub_agents(
        &self,
    ) -> Result<Vec<::contracts::protocol::client::AgentSessionSnapshot>, AdminServiceError> {
        let Some(repository) = &self.resources.agent_runs else {
            return Ok(Vec::new());
        };
        let runs = repository
            .list_recent(MAX_ADMIN_ITEMS)
            .await
            .map_err(|error| AdminServiceError::Operation(error.to_string()))?
            .into_iter()
            .take(MAX_ADMIN_ITEMS)
            .collect::<Vec<_>>();
        let mut sessions = Vec::with_capacity(runs.len());
        for run in runs {
            let snapshot = run.snapshot.clone();
            let timeline = match &self.resources.agent_timeline {
                Some(reader) => {
                    reader
                        .read_agent_timeline(
                            snapshot.handle.root_agent_id,
                            snapshot.handle.agent_id,
                            MAX_ADMIN_ITEMS,
                        )
                        .await?
                }
                None => Vec::new(),
            };
            sessions.push(::contracts::protocol::client::AgentSessionSnapshot {
                id: snapshot.handle.agent_id.0.to_string(),
                task: run.request.task.clone(),
                status: format!("{:?}", snapshot.status).to_ascii_lowercase(),
                runtime_id: snapshot.handle.runtime_id.0.clone(),
                profile_id: snapshot.handle.profile_id.0.clone(),
                snapshot,
                timeline,
            });
        }
        Ok(sessions)
    }

    async fn list_agent_profiles(&self) -> Result<Vec<AgentProfileDescriptor>, AdminServiceError> {
        let registry = self
            .resources
            .agent_profiles
            .as_ref()
            .ok_or_else(|| AdminServiceError::Operation("agent profiles unavailable".into()))?;
        let names = registry.names();
        let mut descriptors = Vec::with_capacity(names.len());
        for name in &names {
            let profile = registry.resolve_profile(name)?;
            descriptors.push(AgentProfileDescriptor {
                name: name.clone(),
                risk_tier: format!("{:?}", profile.risk_tier),
                tool_count: profile.allowed_tools.len(),
                max_iterations: profile.max_iterations,
                approval_policy: format!("{:?}", profile.approval_policy),
            });
        }
        Ok(descriptors)
    }

    async fn switch_agent_profile(
        &self,
        profile_name: String,
    ) -> Result<AgentProfileSwitchResult, AdminServiceError> {
        let registry = self
            .resources
            .agent_profiles
            .as_ref()
            .ok_or_else(|| AdminServiceError::Operation("agent profiles unavailable".into()))?;
        let resolved = registry.resolve_profile(&profile_name)?;
        let previous = {
            let mut current = self
                .resources
                .current_profile
                .as_ref()
                .ok_or_else(|| {
                    AdminServiceError::Operation("current profile state unavailable".into())
                })?
                .lock()
                .await;
            let prev = current.clone();
            let current_profile = registry.resolve_profile(&prev)?;
            if let Err(error) = authorize_agent_profile_switch(&current_profile, &resolved) {
                let event = profile_switch_event(
                    &current_profile,
                    &resolved,
                    AgentProfileSwitchDecision::Denied,
                    Some(error.to_string()),
                );
                drop(current);
                self.resources.profile_switch_events.record(event).await;
                return Err(error);
            }
            if prev != profile_name {
                *current = profile_name.clone();
            }
            let event = profile_switch_event(
                &current_profile,
                &resolved,
                AgentProfileSwitchDecision::Accepted,
                None,
            );
            drop(current);
            self.resources.profile_switch_events.record(event).await;
            prev
        };
        Ok(AgentProfileSwitchResult {
            previous,
            current: profile_name,
            risk_tier: format!("{:?}", resolved.risk_tier),
        })
    }

    async fn preview_memory_forget(
        &self,
        policy: mnemosyne::ForgetPolicy,
    ) -> Result<mnemosyne::ForgetReceipt, AdminServiceError> {
        let admin = self.resources.memory_admin.as_ref().ok_or_else(|| {
            AdminServiceError::Operation("memory administration is unavailable".into())
        })?;
        admin
            .preview_forget(policy)
            .await
            .map_err(|error| AdminServiceError::Operation(error.to_string()))
    }

    async fn forget_memory(
        &self,
        policy: mnemosyne::ForgetPolicy,
    ) -> Result<mnemosyne::ForgetReceipt, AdminServiceError> {
        let admin = self.resources.memory_admin.as_ref().ok_or_else(|| {
            AdminServiceError::Operation("memory administration is unavailable".into())
        })?;
        admin
            .tombstone(policy)
            .await
            .map_err(|error| AdminServiceError::Operation(error.to_string()))
    }

    async fn compact_memory_retention(
        &self,
        owner: &str,
        now_ms: i64,
        policy: mnemosyne::RetentionCompactionPolicy,
    ) -> Result<mnemosyne::RetentionCompactionReport, AdminServiceError> {
        let admin = self.resources.memory_admin.as_ref().ok_or_else(|| {
            AdminServiceError::Operation("memory administration is unavailable".into())
        })?;
        admin
            .compact_retention(owner, now_ms, policy)
            .await
            .map_err(|error| AdminServiceError::Operation(error.to_string()))
    }

    async fn rollback_deployment(
        &self,
        expected_installed_sha: String,
    ) -> Result<platform::DeploymentRollbackReceipt, AdminServiceError> {
        let rollback = self.resources.deployment_rollback.as_ref().ok_or_else(|| {
            AdminServiceError::Operation("deployment rollback is unavailable".into())
        })?;
        rollback.execute(expected_installed_sha).await
    }
}

#[cfg(test)]
mod profile_switch_tests {
    use super::*;
    use ::contracts::{
        AgentApprovalPolicy, AgentProfile, AgentProfileId, ParentRestriction, RiskTier,
    };

    fn profile(name: &str, risk_tier: RiskTier, tools: &[&str]) -> AgentProfile {
        AgentProfile {
            id: AgentProfileId(name.into()),
            system_prompt: "test".into(),
            model: "test".into(),
            allowed_tools: tools.iter().map(|tool| (*tool).to_owned()).collect(),
            delegated_tools: tools.iter().map(|tool| (*tool).to_owned()).collect(),
            max_iterations: 1,
            max_input_tokens: 1,
            max_output_tokens: 1,
            max_tool_calls: 1,
            max_elapsed_ms: 1,
            profile_name: name.into(),
            risk_tier,
            approval_policy: AgentApprovalPolicy::PromptUser,
            tool_timeout_ms: 1,
            inheritable: true,
            parent_restriction: ParentRestriction::SameOrSafer,
        }
    }

    #[test]
    fn safe_to_admin_profile_switch_is_denied() {
        let safe = profile("safe", RiskTier::ReadOnly, &["file_read"]);
        let admin = profile("admin", RiskTier::Unrestricted, &["file_read", "bash_exec"]);
        assert!(authorize_agent_profile_switch(&safe, &admin).is_err());
    }

    #[test]
    fn admin_to_safe_profile_switch_is_allowed() {
        let safe = profile("safe", RiskTier::ReadOnly, &["file_read"]);
        let admin = profile("admin", RiskTier::Unrestricted, &["file_read", "bash_exec"]);
        assert!(authorize_agent_profile_switch(&admin, &safe).is_ok());
    }

    #[test]
    fn sandboxed_to_distinct_read_only_profile_is_allowed() {
        let general = profile("general", RiskTier::Sandboxed, &["file_read", "bash_exec"]);
        let reviewer = profile(
            "reviewer",
            RiskTier::ReadOnly,
            &["file_read", "connector_search"],
        );
        assert!(authorize_agent_profile_switch(&general, &reviewer).is_ok());
    }

    #[test]
    fn accepted_profile_switch_event_is_typed_and_secret_free() {
        let safe = profile("safe", RiskTier::ReadOnly, &["file_read"]);
        let admin = profile("admin", RiskTier::Unrestricted, &["file_read", "bash_exec"]);
        let event = profile_switch_event(&admin, &safe, AgentProfileSwitchDecision::Accepted, None);
        assert_eq!(event.decision, AgentProfileSwitchDecision::Accepted);
        assert_eq!(event.previous_profile, "admin");
        assert_eq!(event.requested_profile, "safe");
        let encoded = serde_json::to_value(event).unwrap();
        assert!(encoded.get("prompt").is_none());
        assert!(encoded.get("system_prompt").is_none());
    }

    #[test]
    fn denied_profile_switch_event_records_bounded_reason() {
        let safe = profile("safe", RiskTier::ReadOnly, &["file_read"]);
        let admin = profile("admin", RiskTier::Unrestricted, &["file_read", "bash_exec"]);
        let denial = authorize_agent_profile_switch(&safe, &admin)
            .unwrap_err()
            .to_string();
        let event = profile_switch_event(
            &safe,
            &admin,
            AgentProfileSwitchDecision::Denied,
            Some(denial),
        );
        assert_eq!(event.decision, AgentProfileSwitchDecision::Denied);
        assert!(event.reason.unwrap().contains("escalate authority"));
    }

    #[test]
    fn same_profile_switch_is_idempotently_allowed() {
        let safe = profile("safe", RiskTier::ReadOnly, &["file_read"]);
        assert!(authorize_agent_profile_switch(&safe, &safe).is_ok());
    }
}

/// Read-only projection of canonical Agent events for interactive clients.
#[async_trait]
impl AgentTimelinePort for adapters_sqlite::event_spine::SqliteEventSpine {
    async fn read_agent_timeline(
        &self,
        root_agent_id: ::contracts::AgentId,
        agent_id: ::contracts::AgentId,
        limit: usize,
    ) -> Result<Vec<::contracts::protocol::client::AgentTimelineEntry>, AdminServiceError> {
        let agent_id = agent_id.0.to_string();
        let requested_limit = limit.clamp(1, 1_000);
        let events = self
            .read_tree_tail(
                runtime::EventTreeId::for_root_session(&root_agent_id.0.to_string()),
                adapters_sqlite::event_spine::EventReadFilter {
                    from_sequence: None,
                    through_sequence: None,
                    schema: None,
                    visibility: Some(runtime::EventVisibility::Control),
                    limit: 10_000,
                },
            )
            .map_err(|error| AdminServiceError::Operation(error.to_string()))?;
        let mut timeline = events
            .into_iter()
            .filter(|event| event.identity.agent_id.as_deref() == Some(agent_id.as_str()))
            .filter_map(|event| {
                let runtime::EventPayload::Inline { value } = event.payload else {
                    return None;
                };
                Some(::contracts::protocol::client::AgentTimelineEntry {
                    sequence: event.position.sequence.0,
                    kind: value
                        .get("kind")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("runtime")
                        .to_owned(),
                    detail: value
                        .get("detail")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                })
            })
            .collect::<Vec<_>>();
        if timeline.len() > requested_limit {
            timeline.drain(..timeline.len() - requested_limit);
        }
        Ok(timeline)
    }
}

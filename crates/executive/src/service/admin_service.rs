//! Request-safe administrative use cases.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use corpus::security::approval::ApprovalDecision;
use fabric::ui_event::{CollaborationMode, InterruptReason};
use serde::Serialize;
use thiserror::Error;
use tokio::sync::{oneshot, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::core::orchestrator::AletheonExecutive;
use crate::service::request_use_cases::MemoryAdminUseCases;

const MAX_ADMIN_ITEMS: usize = 200;

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

#[derive(Clone, Debug, Serialize)]
pub struct SubAgentSummary {
    pub id: String,
    pub task: String,
    pub status: String,
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
    pub principal_id: fabric::PrincipalId,
    pub connection_id: fabric::ConnectionId,
    pub approval_id: String,
    pub decision: String,
}

pub use fabric::{ApprovalOwner, PendingApprovalKey, ThreadGrantKey};

struct PendingApprovalRecord {
    connection_id: fabric::ConnectionId,
    tool: String,
    respond: oneshot::Sender<ApprovalDecision>,
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
}

#[derive(Clone, Default)]
pub struct PendingApprovals {
    inner: Arc<Mutex<HashMap<PendingApprovalKey, PendingApprovalRecord>>>,
}

impl PendingApprovals {
    pub async fn insert(
        &self,
        owner: ApprovalOwner,
        turn_id: fabric::TurnId,
        call_id: String,
        tool: String,
        connection_id: fabric::ConnectionId,
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
        let _ = record.respond.send(decision);
        Ok(ResolvedPendingApproval {
            owner: key.owner,
            tool: record.tool,
        })
    }

    /// Legacy approval responses authenticate a principal but do not carry a
    /// client-authoritative thread. Recover the exact thread only from the
    /// pending key after verifying the authenticated principal.
    pub async fn resolve_authenticated(
        &self,
        principal_id: &fabric::PrincipalId,
        connection_id: &fabric::ConnectionId,
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
}

#[derive(Clone, Default)]
pub struct ScopedApprovalCache {
    inner: Arc<Mutex<HashSet<ThreadGrantKey>>>,
}

impl ScopedApprovalCache {
    pub async fn clear(&self) {
        self.inner.lock().await.clear();
    }

    pub async fn allow_for_thread(
        &self,
        principal_id: fabric::PrincipalId,
        thread_id: fabric::ThreadId,
        tool: impl Into<String>,
    ) {
        self.inner.lock().await.insert(ThreadGrantKey {
            owner: ApprovalOwner::new(principal_id, thread_id),
            tool: tool.into(),
        });
    }

    pub async fn is_allowed(
        &self,
        principal_id: &fabric::PrincipalId,
        thread_id: &fabric::ThreadId,
        tool: &str,
    ) -> bool {
        self.inner.lock().await.contains(&ThreadGrantKey {
            owner: ApprovalOwner::new(principal_id.clone(), thread_id.clone()),
            tool: tool.to_owned(),
        })
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
    async fn tools(&self) -> Result<Vec<fabric::ToolDefinition>, AdminServiceError>;
    async fn hooks(&self) -> Result<Vec<HookDescriptor>, AdminServiceError>;
    async fn sub_agents(&self) -> Result<Vec<SubAgentSummary>, AdminServiceError>;
    async fn list_agent_profiles(
        &self,
    ) -> Result<Vec<AgentProfileDescriptor>, AdminServiceError>;
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
}

#[async_trait]
pub trait SkillAdminPort: Send + Sync {
    async fn reload(&self) -> Result<usize, AdminServiceError>;
}

pub struct DefaultSkillAdmin {
    loader: Arc<Mutex<corpus::SkillLoader>>,
    cached_prefix: Arc<Mutex<String>>,
    config_prompt: String,
}

impl DefaultSkillAdmin {
    pub fn new(
        loader: Arc<Mutex<corpus::SkillLoader>>,
        cached_prefix: Arc<Mutex<String>>,
        config_prompt: String,
    ) -> Self {
        Self {
            loader,
            cached_prefix,
            config_prompt,
        }
    }
}

#[async_trait]
impl SkillAdminPort for DefaultSkillAdmin {
    async fn reload(&self) -> Result<usize, AdminServiceError> {
        let count = self.loader.lock().await.reload();
        let new_prefix = {
            let loader = self.loader.lock().await;
            crate::r#impl::daemon::prefix_builder::PrefixBuilder::build(
                &self.config_prompt,
                loader.skills(),
            )
        };
        *self.cached_prefix.lock().await = new_prefix;
        Ok(count)
    }
}

pub struct AdminResources {
    pub orchestrator: Arc<Mutex<AletheonExecutive>>,
    pub skills: Arc<dyn SkillAdminPort>,
    pub tool_catalog: Arc<dyn Fn() -> ToolCatalogFuture + Send + Sync>,
    pub hook_catalog: Arc<dyn Fn() -> HookCatalogFuture + Send + Sync>,
    pub pending_approvals: PendingApprovals,
    pub session_approvals: ScopedApprovalCache,
    pub daemon_cancel: CancellationToken,
    pub google_sync: Option<Arc<Mutex<Option<crate::r#impl::google::GoogleSyncHandle>>>>,
    pub gbrain_worker: Option<Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>>,
    pub goal_worker: Option<Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>>,
    pub runtime_shutdown: Arc<dyn Fn() -> RuntimeShutdownFuture + Send + Sync>,
    pub memory_admin: Option<Arc<dyn MemoryAdminUseCases>>,
    pub agent_runs: Option<Arc<dyn crate::service::agent_control::AgentRunRepository>>,
    pub agent_profiles: Option<Arc<crate::r#impl::runtime::AgentProfileRegistry>>,
    pub current_profile: Option<Arc<tokio::sync::Mutex<String>>>,
}

pub type ToolCatalogFuture =
    Pin<Box<dyn Future<Output = Vec<fabric::ToolDefinition>> + Send + 'static>>;
pub type HookCatalogFuture = Pin<Box<dyn Future<Output = Vec<HookDescriptor>> + Send + 'static>>;
pub type RuntimeShutdownFuture =
    Pin<Box<dyn Future<Output = Result<(), AdminServiceError>> + Send + 'static>>;

pub struct AdminService {
    resources: AdminResources,
}

impl AdminService {
    pub fn new(resources: AdminResources) -> Self {
        Self { resources }
    }
}

#[async_trait]
impl AdminUseCases for AdminService {
    async fn shutdown(&self) -> Result<(), AdminServiceError> {
        self.resources.daemon_cancel.cancel();
        if let Some(sync) = &self.resources.google_sync {
            if let Some(handle) = sync.lock().await.take() {
                handle.shutdown().await;
            }
        }
        for (name, worker) in [
            ("GBrain", &self.resources.gbrain_worker),
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
        let decision = match request.decision.as_str() {
            "once" => ApprovalDecision::Approve,
            "always" => ApprovalDecision::ApproveForSession,
            _ => ApprovalDecision::Deny,
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
        if decision == ApprovalDecision::ApproveForSession {
            self.resources
                .session_approvals
                .allow_for_thread(
                    resolved.owner.principal_id,
                    resolved.owner.thread_id,
                    resolved.tool,
                )
                .await;
        }
        Ok(true)
    }

    async fn interrupt(&self, reason: InterruptReason) -> Result<(), AdminServiceError> {
        self.resources
            .orchestrator
            .lock()
            .await
            .interrupt_flag()
            .request(reason);
        Ok(())
    }

    async fn switch_mode(&self, mode: CollaborationMode) -> Result<ModeChange, AdminServiceError> {
        let mut runtime = self.resources.orchestrator.lock().await;
        let old = runtime.mode_router().current_mode();
        runtime.mode_router_mut().set_mode(mode);
        Ok(ModeChange { old, new: mode })
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

    async fn tools(&self) -> Result<Vec<fabric::ToolDefinition>, AdminServiceError> {
        let mut tools = (self.resources.tool_catalog)().await;
        tools.truncate(MAX_ADMIN_ITEMS);
        Ok(tools)
    }

    async fn hooks(&self) -> Result<Vec<HookDescriptor>, AdminServiceError> {
        let mut hooks = (self.resources.hook_catalog)().await;
        hooks.truncate(MAX_ADMIN_ITEMS);
        Ok(hooks)
    }

    async fn sub_agents(&self) -> Result<Vec<SubAgentSummary>, AdminServiceError> {
        let Some(repository) = &self.resources.agent_runs else {
            return Ok(Vec::new());
        };
        Ok(repository
            .list_recent(MAX_ADMIN_ITEMS)
            .await
            .map_err(|error| AdminServiceError::Operation(error.to_string()))?
            .into_iter()
            .take(MAX_ADMIN_ITEMS)
            .map(|run| SubAgentSummary {
                id: run.agent_id().0.to_string(),
                task: run.request.task.clone(),
                status: format!("{:?}", run.status()),
            })
            .collect())
    }

    async fn list_agent_profiles(
        &self,
    ) -> Result<Vec<AgentProfileDescriptor>, AdminServiceError> {
        let registry = self
            .resources
            .agent_profiles
            .as_ref()
            .ok_or_else(|| AdminServiceError::Operation("agent profiles unavailable".into()))?;
        let names = registry.names();
        let mut descriptors = Vec::with_capacity(names.len());
        for name in &names {
            let resolved = registry.resolve_by_name(name).map_err(|error| {
                AdminServiceError::Operation(error.to_string())
            })?;
            descriptors.push(AgentProfileDescriptor {
                name: name.clone(),
                risk_tier: format!("{:?}", resolved.profile.risk_tier),
                tool_count: resolved.profile.allowed_tools.len(),
                max_iterations: resolved.profile.max_iterations,
                approval_policy: format!("{:?}", resolved.profile.approval_policy),
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
        // Validate the profile exists before switching.
        let resolved = registry
            .resolve_by_name(&profile_name)
            .map_err(|error| AdminServiceError::Operation(error.to_string()))?;
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
            *current = profile_name.clone();
            prev
        };
        Ok(AgentProfileSwitchResult {
            previous,
            current: profile_name,
            risk_tier: format!("{:?}", resolved.profile.risk_tier),
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
}

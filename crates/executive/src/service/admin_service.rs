//! Request-safe administrative use cases.

use std::collections::HashMap;
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

#[derive(Clone, Debug)]
pub struct TransientApprovalRequest {
    pub approval_id: String,
    pub decision: String,
    pub tool_name: String,
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
    pub pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    pub session_approvals: Arc<Mutex<HashMap<String, bool>>>,
    pub daemon_cancel: CancellationToken,
    pub google_sync: Option<Arc<Mutex<Option<crate::r#impl::google::GoogleSyncHandle>>>>,
    pub gbrain_worker: Option<Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>>,
    pub goal_worker: Option<Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>>,
    pub memory_admin: Option<Arc<dyn MemoryAdminUseCases>>,
}

pub type ToolCatalogFuture =
    Pin<Box<dyn Future<Output = Vec<fabric::ToolDefinition>> + Send + 'static>>;
pub type HookCatalogFuture = Pin<Box<dyn Future<Output = Vec<HookDescriptor>> + Send + 'static>>;

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
            "always" => {
                if !request.tool_name.is_empty() {
                    self.resources
                        .session_approvals
                        .lock()
                        .await
                        .insert(request.tool_name, true);
                }
                ApprovalDecision::ApproveForSession
            }
            _ => ApprovalDecision::Deny,
        };
        let sender = self
            .resources
            .pending_approvals
            .lock()
            .await
            .remove(&request.approval_id);
        if let Some(sender) = sender {
            let _ = sender.send(decision);
            Ok(true)
        } else {
            Ok(false)
        }
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
        let runtime = self.resources.orchestrator.lock().await;
        Ok(runtime
            .sub_agent_spawner()
            .list()
            .into_iter()
            .take(MAX_ADMIN_ITEMS)
            .map(|agent| SubAgentSummary {
                id: agent.id.clone(),
                task: agent.task.clone(),
                status: format!("{:?}", agent.status),
            })
            .collect())
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

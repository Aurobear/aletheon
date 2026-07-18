//! Request-facing use cases for daemon protocol adapters.
//!
//! Concrete domain handles are captured once by bootstrap. JSON-RPC handlers
//! receive only these contracts and never acquire domain locks themselves.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use fabric::hook::{HookContext, HookPoint};
use fabric::ui_event::InterruptReason;
use fabric::{
    Clock, EvolutionLogEntry, ExternalIdentityState, ExternalScope, GrantState, OperationId,
    OperationResult, PrincipalContext, PrincipalId, ProcessId, ReflectionEntry, ReflectionTrigger,
    SessionId, ThreadId, LOCAL_OWNER_PRINCIPAL,
};
use serde::Serialize;
use serde_json::json;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::r#impl::daemon::debug_handler::DebugHandler;
use crate::r#impl::external::GoogleIntegration;
use crate::r#impl::health::{ComponentHealth, HealthRegistry, ProductionHealth};
use crate::r#impl::orchestration::digraph::graph::{DiGraph, WorkflowDef};
use crate::r#impl::orchestration::store::WorkflowStore;
use crate::service::session_service::{InterruptOutcome, ResumeResult, SessionService};
use crate::service::DaemonTurnOrchestrator;

#[derive(Clone, Debug, Serialize)]
pub struct RequestStatus {
    pub session_id: String,
    pub iteration: usize,
    pub reflection_count: usize,
    pub evolution_count: usize,
    pub care_weights: Vec<CareWeight>,
    pub boundary_rules: usize,
    pub boundary_immutable: usize,
    pub attention_focus: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CareWeight {
    pub topic: String,
    pub weight: f64,
}

#[derive(Clone, Debug)]
pub struct RuntimeStatus {
    pub session_id: String,
    pub iteration: usize,
}

#[async_trait]
pub trait ExecutiveRuntimePort: Send + Sync {
    async fn status(&self) -> RuntimeStatus;
    async fn request_interrupt(&self, reason: InterruptReason);
}

#[derive(Clone, Debug, Default)]
pub struct ReflectionStats {
    pub reflection_count: usize,
    pub evolution_count: usize,
}

#[async_trait]
pub trait ReflectionMemoryPort: Send + Sync {
    async fn stats(&self) -> ReflectionStats;
    async fn recall_reflections(&self, limit: usize) -> anyhow::Result<Vec<ReflectionEntry>>;
    async fn store_reflection(&self, entry: &ReflectionEntry) -> anyhow::Result<()>;
    async fn recall_evolution_logs(&self, limit: usize) -> anyhow::Result<Vec<EvolutionLogEntry>>;
}

pub trait ReflectionEnginePort: Send + Sync {
    fn reflect_conversation(
        &self,
        conversation: &str,
        trigger: ReflectionTrigger,
        succeeded: bool,
        what_worked: Vec<String>,
        what_failed: Vec<String>,
        learned: Vec<String>,
    ) -> ReflectionEntry;
}

#[derive(Clone, Debug, Default)]
pub struct SelfStatus {
    pub care_weights: Vec<CareWeight>,
    pub boundary_rules: usize,
    pub boundary_immutable: usize,
    pub attention_focus: String,
}

#[async_trait]
pub trait SelfStatusPort: Send + Sync {
    async fn status(&self) -> SelfStatus;
}

#[derive(Clone, Debug, Default)]
pub struct SupplementalMemoryStatus {
    pub enabled: bool,
    pub degraded: bool,
    pub queue_depth: usize,
}

pub trait SupplementalMemoryStatusPort: Send + Sync {
    fn status(&self) -> SupplementalMemoryStatus;
}

#[derive(Clone, Debug, Serialize)]
pub struct RequestHealth {
    #[serde(flatten)]
    pub production: ProductionHealth,
    pub uptime_seconds: u64,
    pub active_connections: usize,
}

#[async_trait]
pub trait HealthUseCases: Send + Sync {
    async fn status(&self) -> anyhow::Result<RequestStatus>;
    async fn health(&self) -> RequestHealth;
}

pub struct ProductionHealthUseCases {
    runtime_port: Arc<dyn ExecutiveRuntimePort>,
    reflections: Arc<dyn ReflectionMemoryPort>,
    self_status: Arc<dyn SelfStatusPort>,
    supplemental: Arc<dyn SupplementalMemoryStatusPort>,
    data_root: PathBuf,
    registry: Arc<HealthRegistry>,
    clock: Arc<dyn Clock>,
    started_at: fabric::MonoTime,
    active_connections: Arc<AtomicUsize>,
    daemon_cancel: CancellationToken,
    telegram_task: Arc<Mutex<Option<Arc<tokio::task::JoinHandle<()>>>>>,
    google_sync: Option<Arc<Mutex<Option<crate::r#impl::google::GoogleSyncHandle>>>>,
    goal_worker: Option<Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>>,
    agent_recovery: crate::service::agent_control::AgentRecoveryReport,
}

pub struct ProductionHealthResources {
    pub runtime_port: Arc<dyn ExecutiveRuntimePort>,
    pub reflections: Arc<dyn ReflectionMemoryPort>,
    pub self_status: Arc<dyn SelfStatusPort>,
    pub supplemental: Arc<dyn SupplementalMemoryStatusPort>,
    pub data_root: PathBuf,
    pub registry: Arc<HealthRegistry>,
    pub clock: Arc<dyn Clock>,
    pub started_at: fabric::MonoTime,
    pub active_connections: Arc<AtomicUsize>,
    pub daemon_cancel: CancellationToken,
    pub telegram_task: Arc<Mutex<Option<Arc<tokio::task::JoinHandle<()>>>>>,
    pub google_sync: Option<Arc<Mutex<Option<crate::r#impl::google::GoogleSyncHandle>>>>,
    pub goal_worker: Option<Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>>,
    pub agent_recovery: crate::service::agent_control::AgentRecoveryReport,
}

impl ProductionHealthUseCases {
    pub fn new(resources: ProductionHealthResources) -> Self {
        Self {
            runtime_port: resources.runtime_port,
            reflections: resources.reflections,
            self_status: resources.self_status,
            supplemental: resources.supplemental,
            data_root: resources.data_root,
            registry: resources.registry,
            clock: resources.clock,
            started_at: resources.started_at,
            active_connections: resources.active_connections,
            daemon_cancel: resources.daemon_cancel,
            telegram_task: resources.telegram_task,
            google_sync: resources.google_sync,
            goal_worker: resources.goal_worker,
            agent_recovery: resources.agent_recovery,
        }
    }
}

#[async_trait]
impl HealthUseCases for ProductionHealthUseCases {
    async fn status(&self) -> anyhow::Result<RequestStatus> {
        let runtime = self.runtime_port.status().await;
        let reflections = self.reflections.stats().await;
        let self_status = self.self_status.status().await;
        Ok(RequestStatus {
            session_id: runtime.session_id,
            iteration: runtime.iteration,
            reflection_count: reflections.reflection_count,
            evolution_count: reflections.evolution_count,
            care_weights: self_status.care_weights,
            boundary_rules: self_status.boundary_rules,
            boundary_immutable: self_status.boundary_immutable,
            attention_focus: self_status.attention_focus,
        })
    }

    async fn health(&self) -> RequestHealth {
        let mut agent_recovery = if self.agent_recovery.ready() {
            ComponentHealth::ready()
        } else {
            ComponentHealth::unready("agent_recovery_incomplete")
        };
        agent_recovery.count = Some(
            self.agent_recovery
                .unreconciled
                .saturating_add(self.agent_recovery.recovery_failed) as u64,
        );
        self.registry.set("agent_recovery", agent_recovery);
        let minimum_free_bytes = env_u64("ALETHEON_MINIMUM_FREE_BYTES", 5 * 1024 * 1024 * 1024);
        let maximum_backup_age_secs = env_u64("ALETHEON_MAXIMUM_BACKUP_AGE_SECS", 36 * 60 * 60);
        let backup_required = std::env::var("ALETHEON_BACKUP_REQUIRED")
            .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
        let data_root = std::env::var_os("ALETHEON_DATA_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.data_root.clone());
        self.registry.refresh_storage(
            &data_root,
            minimum_free_bytes,
            backup_required,
            maximum_backup_age_secs,
        );
        self.registry.set(
            "telegram",
            match self.telegram_task.lock().await.as_ref() {
                Some(task) if task.is_finished() => ComponentHealth::degraded("worker_stopped"),
                Some(_) => ComponentHealth::ready(),
                None => ComponentHealth::disabled(),
            },
        );
        self.registry.set(
            "google_sync",
            match &self.google_sync {
                Some(sync) if sync.lock().await.is_some() => ComponentHealth::ready(),
                Some(_) => ComponentHealth::degraded("worker_stopped"),
                None => ComponentHealth::disabled(),
            },
        );
        self.registry.set(
            "goal_worker",
            match &self.goal_worker {
                Some(worker) => match worker.lock().await.as_ref() {
                    Some(task) if task.is_finished() => ComponentHealth::unready("worker_stopped"),
                    Some(_) => ComponentHealth::ready(),
                    None => ComponentHealth::unready("worker_stopped"),
                },
                None => ComponentHealth::disabled(),
            },
        );
        let supplemental = self.supplemental.status();
        let mut supplemental_health = if !supplemental.enabled {
            ComponentHealth::disabled()
        } else if supplemental.degraded {
            ComponentHealth::degraded("supplemental_memory")
        } else {
            ComponentHealth::ready()
        };
        if supplemental.enabled {
            supplemental_health.count = Some(supplemental.queue_depth as u64);
        }
        self.registry.set("gbrain_spool", supplemental_health);
        if self.daemon_cancel.is_cancelled() {
            self.registry.begin_shutdown();
        }
        RequestHealth {
            production: self.registry.snapshot(),
            uptime_seconds: self.clock.mono_now().0.saturating_sub(self.started_at.0) / 1000,
            active_connections: self.active_connections.load(Ordering::Relaxed),
        }
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[async_trait]
pub trait ReflectionUseCases: Send + Sync {
    async fn list(&self, limit: usize) -> anyhow::Result<Vec<ReflectionEntry>>;
    async fn reflect_now(&self, turn: usize) -> anyhow::Result<ReflectionEntry>;
    async fn genome_yaml(&self) -> anyhow::Result<String>;
    async fn evolution(&self, limit: usize) -> anyhow::Result<Vec<EvolutionLogEntry>>;
}

pub struct ProductionReflectionUseCases {
    runtime_port: Arc<dyn ExecutiveRuntimePort>,
    reflections: Arc<dyn ReflectionMemoryPort>,
    metacog: Arc<dyn metacog::MetacogService>,
    reflector: Arc<dyn ReflectionEnginePort>,
}

impl ProductionReflectionUseCases {
    pub fn new(
        runtime_port: Arc<dyn ExecutiveRuntimePort>,
        reflections: Arc<dyn ReflectionMemoryPort>,
        metacog: Arc<dyn metacog::MetacogService>,
        reflector: Arc<dyn ReflectionEnginePort>,
    ) -> Self {
        Self {
            runtime_port,
            reflections,
            metacog,
            reflector,
        }
    }
}

#[async_trait]
impl ReflectionUseCases for ProductionReflectionUseCases {
    async fn list(&self, limit: usize) -> anyhow::Result<Vec<ReflectionEntry>> {
        self.reflections.recall_reflections(limit).await
    }

    async fn reflect_now(&self, turn: usize) -> anyhow::Result<ReflectionEntry> {
        let runtime = self.runtime_port.status().await;
        let recent = self.reflections.recall_reflections(5).await?;
        let mut what_worked = vec![
            format!("Session is active with {turn} turns"),
            format!("Runtime iteration count: {}", runtime.iteration),
        ];
        let mut what_failed = Vec::new();
        let mut learned = Vec::new();
        if turn == 0 {
            what_failed.push("No chat turns recorded yet".to_string());
        }
        if recent.is_empty() {
            learned.push("No prior reflections available for context".to_string());
        } else {
            learned.push(format!("Reviewed {} recent reflections", recent.len()));
            let failures: usize = recent.iter().map(|entry| entry.what_failed.len()).sum();
            if failures > 0 {
                what_failed.push(format!(
                    "{failures} failure items across recent reflections"
                ));
            }
        }
        let succeeded = what_failed.is_empty() && turn > 0;
        let entry = self.reflector.reflect_conversation(
            &format!(
                "Session {} after {turn} turns (iteration {})",
                runtime.session_id, runtime.iteration
            ),
            ReflectionTrigger::Manual,
            succeeded,
            std::mem::take(&mut what_worked),
            what_failed,
            learned,
        );
        self.reflections.store_reflection(&entry).await?;
        Ok(entry)
    }

    async fn genome_yaml(&self) -> anyhow::Result<String> {
        let genome = self.metacog.genome().await?;
        Ok(serde_yaml::to_string(&genome)?)
    }

    async fn evolution(&self, limit: usize) -> anyhow::Result<Vec<EvolutionLogEntry>> {
        self.reflections.recall_evolution_logs(limit).await
    }
}

#[async_trait]
pub trait SessionLifecycleUseCases: Send + Sync {
    async fn reset_turn_token(&self);
    async fn finish(&self, session_id: String, turn_count: usize) -> anyhow::Result<()>;
    async fn start(&self, session_id: String, clear_approvals: bool) -> anyhow::Result<()>;
}

pub struct ProductionSessionLifecycle {
    corpus: Arc<dyn corpus::CorpusService>,
    approvals: crate::service::admin_service::ScopedApprovalCache,
    cancel_token: Arc<Mutex<Option<CancellationToken>>>,
    lifecycle: Arc<crate::service::lifecycle_contributors::LifecycleRegistry>,
    lifecycle_enabled: bool,
}

impl ProductionSessionLifecycle {
    pub fn new(
        corpus: Arc<dyn corpus::CorpusService>,
        approvals: crate::service::admin_service::ScopedApprovalCache,
        cancel_token: Arc<Mutex<Option<CancellationToken>>>,
        lifecycle: Arc<crate::service::lifecycle_contributors::LifecycleRegistry>,
        lifecycle_enabled: bool,
    ) -> Self {
        Self {
            corpus,
            approvals,
            cancel_token,
            lifecycle,
            lifecycle_enabled,
        }
    }

    async fn dispatch(
        &self,
        phase: crate::service::lifecycle_contributors::LifecyclePhase,
        session_id: &str,
    ) -> anyhow::Result<()> {
        self.lifecycle
            .dispatch_if_enabled(
                self.lifecycle_enabled,
                crate::service::lifecycle_contributors::LifecycleInput {
                    phase,
                    principal_id: fabric::PrincipalId(fabric::LOCAL_OWNER_PRINCIPAL.into()),
                    thread_id: fabric::ThreadId(session_id.into()),
                    turn_id: None,
                    session_id: session_id.into(),
                    detail: serde_json::Value::Null,
                },
            )
            .await
            .map_err(|error| anyhow::anyhow!("{phase:?} lifecycle rejected session: {error}"))?;
        Ok(())
    }
}

#[async_trait]
impl SessionLifecycleUseCases for ProductionSessionLifecycle {
    async fn reset_turn_token(&self) {
        *self.cancel_token.lock().await = None;
    }

    async fn finish(&self, session_id: String, turn_count: usize) -> anyhow::Result<()> {
        self.dispatch(
            crate::service::lifecycle_contributors::LifecyclePhase::BeforeSessionEnd,
            &session_id,
        )
        .await?;
        let context = HookContext {
            point: HookPoint::OnSessionEnd,
            session_id: session_id.clone(),
            turn_count,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: {
                let cwd = std::env::current_dir()
                    .unwrap_or_default()
                    .display()
                    .to_string();
                HashMap::from([("cwd".into(), cwd.clone()), ("workspace_root".into(), cwd)])
            },
        };
        self.corpus.execute_hook(&context).await;
        self.dispatch(
            crate::service::lifecycle_contributors::LifecyclePhase::AfterSessionEnd,
            &session_id,
        )
        .await?;
        Ok(())
    }

    async fn start(&self, session_id: String, clear_approvals: bool) -> anyhow::Result<()> {
        self.dispatch(
            crate::service::lifecycle_contributors::LifecyclePhase::BeforeSessionStart,
            &session_id,
        )
        .await?;
        if clear_approvals {
            self.approvals.clear().await;
        }
        let context = HookContext {
            point: HookPoint::OnSessionStart,
            session_id: session_id.clone(),
            turn_count: 0,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: {
                let cwd = std::env::current_dir()
                    .unwrap_or_default()
                    .display()
                    .to_string();
                HashMap::from([("cwd".into(), cwd.clone()), ("workspace_root".into(), cwd)])
            },
        };
        self.corpus.execute_hook(&context).await;
        self.dispatch(
            crate::service::lifecycle_contributors::LifecyclePhase::AfterSessionStart,
            &session_id,
        )
        .await?;
        Ok(())
    }
}

#[async_trait]
pub trait TurnUseCases: Send + Sync {
    async fn execute(
        &self,
        id: serde_json::Value,
        message: String,
        context: PrincipalContext,
    ) -> serde_json::Value;
    async fn wait(&self, id: OperationId) -> anyhow::Result<OperationResult>;
    async fn cancel(&self, id: OperationId) -> anyhow::Result<()>;
    async fn cancel_by_key(
        &self,
        principal_id: PrincipalId,
        thread_id: String,
        turn_id: fabric::TurnId,
        operation_id: OperationId,
    ) -> anyhow::Result<()>;
    async fn exit(&self, id: ProcessId) -> anyhow::Result<()>;
    async fn rewind_workspace(
        &self,
        principal_id: PrincipalId,
        session_id: String,
        prompt_index: u64,
        workspace: fabric::types::workspace_checkpoint::WorkspaceIdentity,
    ) -> fabric::types::workspace_checkpoint::RestoreOutcome;
    async fn cancel_current(&self);
    async fn cancel_current_with_thread(&self, thread_id: String);
    async fn session_resume(&self, id: SessionId) -> anyhow::Result<ResumeResult>;
    async fn session_fork(
        &self,
        id: SessionId,
        through: u64,
    ) -> anyhow::Result<fabric::SessionRecord>;
    async fn session_interrupt(&self, id: SessionId) -> anyhow::Result<InterruptOutcome>;
    async fn session_replay(
        &self,
        id: SessionId,
        after: Option<u64>,
    ) -> anyhow::Result<Vec<fabric::Message>>;
    async fn set_notify(&self, sender: tokio::sync::mpsc::Sender<String>);
}

pub struct ProductionTurnUseCases {
    orchestrator: Arc<DaemonTurnOrchestrator>,
    runtime_port: Arc<dyn ExecutiveRuntimePort>,
    cancel_token: Arc<Mutex<Option<CancellationToken>>>,
    sessions: Arc<SessionService>,
}

impl ProductionTurnUseCases {
    pub fn new(
        orchestrator: Arc<DaemonTurnOrchestrator>,
        runtime_port: Arc<dyn ExecutiveRuntimePort>,
        cancel_token: Arc<Mutex<Option<CancellationToken>>>,
        sessions: Arc<SessionService>,
    ) -> Self {
        Self {
            orchestrator,
            runtime_port,
            cancel_token,
            sessions,
        }
    }
}

#[async_trait]
impl TurnUseCases for ProductionTurnUseCases {
    async fn execute(
        &self,
        id: serde_json::Value,
        message: String,
        context: PrincipalContext,
    ) -> serde_json::Value {
        self.orchestrator.execute_turn(id, &message, context).await
    }
    async fn wait(&self, id: OperationId) -> anyhow::Result<OperationResult> {
        self.orchestrator.wait_turn(id).await
    }
    async fn cancel(&self, id: OperationId) -> anyhow::Result<()> {
        self.orchestrator.cancel_turn(id).await
    }
    async fn cancel_by_key(
        &self,
        principal_id: PrincipalId,
        thread_id: String,
        turn_id: fabric::TurnId,
        operation_id: OperationId,
    ) -> anyhow::Result<()> {
        let tid = ThreadId(thread_id);
        self.orchestrator
            .cancel_turn_by_key(&principal_id, &tid, turn_id, operation_id)
            .await
    }
    async fn exit(&self, id: ProcessId) -> anyhow::Result<()> {
        self.orchestrator.exit_process(id).await
    }
    async fn rewind_workspace(
        &self,
        principal_id: PrincipalId,
        session_id: String,
        prompt_index: u64,
        workspace: fabric::types::workspace_checkpoint::WorkspaceIdentity,
    ) -> fabric::types::workspace_checkpoint::RestoreOutcome {
        self.orchestrator
            .rewind_workspace(&principal_id, &session_id, prompt_index, &workspace)
            .await
    }
    async fn cancel_current(&self) {
        self.runtime_port
            .request_interrupt(InterruptReason::Timeout)
            .await;
        if let Some(token) = self.cancel_token.lock().await.take() {
            token.cancel();
        }
    }
    async fn cancel_current_with_thread(&self, thread_id: String) {
        self.runtime_port
            .request_interrupt(InterruptReason::Timeout)
            .await;
        if let Some(token) = self.cancel_token.lock().await.take() {
            token.cancel();
        }
        let _ = thread_id;
    }
    async fn session_resume(&self, id: SessionId) -> anyhow::Result<ResumeResult> {
        self.sessions.resume(&id).await
    }
    async fn session_fork(
        &self,
        id: SessionId,
        through: u64,
    ) -> anyhow::Result<fabric::SessionRecord> {
        self.sessions.fork(&id, through).await
    }
    async fn session_interrupt(&self, id: SessionId) -> anyhow::Result<InterruptOutcome> {
        self.sessions.interrupt(&id).await
    }
    async fn session_replay(
        &self,
        id: SessionId,
        after: Option<u64>,
    ) -> anyhow::Result<Vec<fabric::Message>> {
        self.sessions.replay(&id, after).await
    }

    async fn set_notify(&self, sender: tokio::sync::mpsc::Sender<String>) {
        self.orchestrator.set_notify_sender(sender).await;
    }
}

#[derive(Debug, Error)]
pub enum GoogleUseCaseError {
    #[error("google_not_configured")]
    Unavailable,
    #[error("google_account_not_found")]
    NotFound,
    #[error("google_account_revoked_or_scope_denied")]
    Forbidden,
    #[error("google provider operation failed")]
    Provider,
}

#[async_trait]
pub trait GoogleUseCases: Send + Sync {
    async fn authorization_start(&self) -> Result<serde_json::Value, GoogleUseCaseError>;
    async fn authorization_callback(
        &self,
        code: String,
        state: String,
        alias: Option<String>,
    ) -> Result<serde_json::Value, GoogleUseCaseError>;
    async fn accounts(&self) -> Result<Vec<serde_json::Value>, GoogleUseCaseError>;
    async fn revoke(&self, account: String) -> Result<(bool, bool), GoogleUseCaseError>;
    async fn refresh(&self, account: String) -> Result<GoogleRefresh, GoogleUseCaseError>;
}

#[derive(Clone, Debug, Serialize)]
pub struct GoogleRefresh {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

pub struct ProductionGoogleUseCases {
    integration: Option<Arc<GoogleIntegration>>,
    corpus: Arc<dyn corpus::CorpusService>,
    capabilities: Arc<RwLock<Vec<fabric::CapabilityId>>>,
    clock: Arc<dyn Clock>,
}

impl ProductionGoogleUseCases {
    pub fn new(
        integration: Option<Arc<GoogleIntegration>>,
        corpus: Arc<dyn corpus::CorpusService>,
        capabilities: Arc<RwLock<Vec<fabric::CapabilityId>>>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            integration,
            corpus,
            capabilities,
            clock,
        }
    }

    fn context(&self) -> Result<(Arc<GoogleIntegration>, PrincipalId), GoogleUseCaseError> {
        Ok((
            self.integration
                .clone()
                .ok_or(GoogleUseCaseError::Unavailable)?,
            PrincipalId(LOCAL_OWNER_PRINCIPAL.into()),
        ))
    }

    async fn register_read_tools(&self, google: &Arc<GoogleIntegration>) -> anyhow::Result<()> {
        use corpus::tools::google::{
            GoogleApiClient, GoogleApiEndpoints, GoogleCalendarAdapter, GoogleGmailAdapter,
        };
        let repository = google.repository();
        let (gmail, calendar) = {
            let repository = repository.lock().unwrap();
            (
                repository.has_active_scope(ExternalScope::GmailReadonly)?,
                repository.has_active_scope(ExternalScope::CalendarReadonly)?,
            )
        };
        let credentials = Arc::new(
            crate::r#impl::external::ExecutiveGoogleCredentialSource::new(
                repository.clone(),
                google.oauth(),
            ),
        );
        let accounts =
            Arc::new(crate::r#impl::external::ExecutiveGoogleAccountResolver::new(repository));
        let client = GoogleApiClient::new(credentials, GoogleApiEndpoints::default())?;
        if gmail && !self.tool_registered("google_gmail_search").await? {
            let gmail = Arc::new(GoogleGmailAdapter::new(client.clone()));
            self.corpus
                .register_tool(Arc::new(corpus::tools::google::GoogleGmailSearchTool::new(
                    gmail.clone(),
                    accounts.clone(),
                )))
                .await?;
            self.grant_tool("google_gmail_search").await;
            self.corpus
                .register_tool(Arc::new(corpus::tools::google::GoogleGmailReadTool::new(
                    gmail,
                    accounts.clone(),
                )))
                .await?;
            self.grant_tool("google_gmail_read").await;
        }
        if calendar && !self.tool_registered("google_calendar_list").await? {
            self.corpus
                .register_tool(Arc::new(
                    corpus::tools::google::GoogleCalendarListTool::new(
                        Arc::new(GoogleCalendarAdapter::new(client)),
                        accounts,
                    ),
                ))
                .await?;
            self.grant_tool("google_calendar_list").await;
        }
        Ok(())
    }

    async fn tool_registered(&self, name: &str) -> anyhow::Result<bool> {
        let grant = corpus::ExtensionGrant {
            grant_id: format!("google-tool-check:{name}"),
            principal: PrincipalId(LOCAL_OWNER_PRINCIPAL.into()),
            session_id: "google-admin".into(),
            agent_id: None,
            capabilities: vec![fabric::CapabilityId(name.into())],
            resources: fabric::CapabilityScope::default(),
        };
        Ok(!self.corpus.catalog(&grant).await?.entries.is_empty())
    }

    async fn grant_tool(&self, name: &str) {
        let capability = fabric::CapabilityId(name.into());
        let mut capabilities = self.capabilities.write().await;
        if !capabilities.contains(&capability) {
            capabilities.push(capability);
        }
    }
}

#[async_trait]
impl GoogleUseCases for ProductionGoogleUseCases {
    async fn authorization_start(&self) -> Result<serde_json::Value, GoogleUseCaseError> {
        let (google, principal) = self.context()?;
        let start = google
            .start_authorization(&principal)
            .await
            .map_err(|_| GoogleUseCaseError::Provider)?;
        Ok(
            json!({"authorization_url":start.url,"state":start.state,"expires_at_secs":start.expires_at_secs}),
        )
    }
    async fn authorization_callback(
        &self,
        code: String,
        state: String,
        alias: Option<String>,
    ) -> Result<serde_json::Value, GoogleUseCaseError> {
        let (google, principal) = self.context()?;
        let (identity, grant) = google
            .complete_authorization(&principal, &code, &state, alias, self.clock.wall_now().0)
            .await
            .map_err(|_| GoogleUseCaseError::Provider)?;
        if let Err(error) = self.register_read_tools(&google).await {
            tracing::warn!(%error, "Google account bound but tool registration failed");
        }
        Ok(safe_account(&identity, &grant))
    }
    async fn accounts(&self) -> Result<Vec<serde_json::Value>, GoogleUseCaseError> {
        let (google, principal) = self.context()?;
        google
            .repository()
            .lock()
            .unwrap()
            .list(&principal)
            .map(|items| {
                items
                    .iter()
                    .map(|(identity, grant)| safe_account(identity, grant))
                    .collect()
            })
            .map_err(|_| GoogleUseCaseError::Provider)
    }
    async fn revoke(&self, account: String) -> Result<(bool, bool), GoogleUseCaseError> {
        let (google, principal) = self.context()?;
        let repository = google.repository();
        let identity = {
            let repository = repository.lock().unwrap();
            let id = repository
                .resolve_account(&principal, &account)
                .map_err(|_| GoogleUseCaseError::Provider)?
                .ok_or(GoogleUseCaseError::NotFound)?;
            repository
                .get(&principal, id)
                .map_err(|_| GoogleUseCaseError::Provider)?
                .map(|item| item.0)
                .ok_or(GoogleUseCaseError::NotFound)?
        };
        repository
            .lock()
            .unwrap()
            .revoke_local(
                &principal,
                identity.id,
                identity.version,
                self.clock.wall_now().0,
            )
            .map_err(|_| GoogleUseCaseError::Provider)?;
        let provider = google
            .oauth()
            .lock()
            .await
            .revoke(identity.id)
            .await
            .is_ok();
        Ok((true, provider))
    }
    async fn refresh(&self, account: String) -> Result<GoogleRefresh, GoogleUseCaseError> {
        let (google, principal) = self.context()?;
        let account_id = {
            let repository = google.repository();
            let repository = repository.lock().unwrap();
            let id = repository
                .resolve_account(&principal, &account)
                .map_err(|_| GoogleUseCaseError::Provider)?
                .ok_or(GoogleUseCaseError::NotFound)?;
            let (identity, grant) = repository
                .get(&principal, id)
                .map_err(|_| GoogleUseCaseError::Provider)?
                .ok_or(GoogleUseCaseError::NotFound)?;
            let active = identity.state == ExternalIdentityState::Active
                && grant.state == GrantState::Active
                && grant.scopes.iter().any(|scope| {
                    matches!(
                        scope,
                        ExternalScope::GmailReadonly
                            | ExternalScope::CalendarReadonly
                            | ExternalScope::DriveReadonly
                    )
                })
                && grant.scopes.iter().all(|scope| !scope.is_write());
            if !active {
                return Err(GoogleUseCaseError::Forbidden);
            }
            id
        };
        match google.refresh_singleflight(account_id).await {
            Ok(_) => Ok(GoogleRefresh {
                status: "success".into(),
                code: None,
            }),
            Err(corpus::tools::google::GoogleApiError::ReauthorizationRequired) => {
                Ok(GoogleRefresh {
                    status: "reauthorization_required".into(),
                    code: None,
                })
            }
            Err(error) => Ok(GoogleRefresh {
                status: "error".into(),
                code: Some(error.to_string()),
            }),
        }
    }
}

fn safe_account(
    identity: &fabric::ExternalIdentity,
    grant: &fabric::CapabilityGrant,
) -> serde_json::Value {
    json!({
        "id": identity.id, "email": identity.email, "alias": identity.alias,
        "state": identity.state, "scopes": grant.scopes,
        "grant_state": grant.state, "version": identity.version,
    })
}

#[async_trait]
pub trait WorkflowUseCases: Send + Sync {
    async fn save(&self, name: String, definition: WorkflowDef) -> anyhow::Result<()>;
    async fn load(&self, name: String) -> anyhow::Result<WorkflowDef>;
    async fn list(&self) -> anyhow::Result<Vec<String>>;
    async fn delete(&self, name: String) -> anyhow::Result<()>;
}

pub struct ProductionWorkflowUseCases {
    root: PathBuf,
}

impl ProductionWorkflowUseCases {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    fn store(&self) -> anyhow::Result<WorkflowStore> {
        Ok(WorkflowStore::new(self.root.clone())?)
    }
}

#[async_trait]
impl WorkflowUseCases for ProductionWorkflowUseCases {
    async fn save(&self, name: String, definition: WorkflowDef) -> anyhow::Result<()> {
        self.store()?.save(&name, &DiGraph::from_def(&definition))
    }
    async fn load(&self, name: String) -> anyhow::Result<WorkflowDef> {
        Ok(self.store()?.load(&name)?.to_def())
    }
    async fn list(&self) -> anyhow::Result<Vec<String>> {
        self.store()?.list()
    }
    async fn delete(&self, name: String) -> anyhow::Result<()> {
        self.store()?.delete(&name)
    }
}

pub trait DebugUseCases: Send + Sync {
    fn handler(&self) -> Arc<DebugHandler>;
}

pub struct ProductionDebugUseCases(pub Arc<DebugHandler>);
impl DebugUseCases for ProductionDebugUseCases {
    fn handler(&self) -> Arc<DebugHandler> {
        self.0.clone()
    }
}

#[async_trait]
pub trait MemoryAdminUseCases: Send + Sync {
    async fn preview_forget(
        &self,
        policy: mnemosyne::ForgetPolicy,
    ) -> anyhow::Result<mnemosyne::ForgetReceipt>;
    async fn tombstone(
        &self,
        policy: mnemosyne::ForgetPolicy,
    ) -> anyhow::Result<mnemosyne::ForgetReceipt>;
    async fn compact_retention(
        &self,
        owner: &str,
        now_ms: i64,
        policy: mnemosyne::RetentionCompactionPolicy,
    ) -> anyhow::Result<mnemosyne::RetentionCompactionReport>;
}

pub trait RetentionAdminPort: Send + Sync {
    fn compact(
        &self,
        owner: &str,
        now_ms: i64,
        policy: &mnemosyne::RetentionCompactionPolicy,
    ) -> anyhow::Result<mnemosyne::RetentionCompactionReport>;
}

pub struct ProductionMemoryAdminUseCases {
    service: Arc<dyn mnemosyne::MemoryService>,
    retention: Arc<dyn RetentionAdminPort>,
    authenticated_principal: String,
}

impl ProductionMemoryAdminUseCases {
    pub fn new(
        service: Arc<dyn mnemosyne::MemoryService>,
        retention: Arc<dyn RetentionAdminPort>,
        authenticated_principal: impl Into<String>,
    ) -> Self {
        Self {
            service,
            retention,
            authenticated_principal: authenticated_principal.into(),
        }
    }

    fn authenticate(&self, requester: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.authenticated_principal.trim().is_empty()
                && requester == self.authenticated_principal,
            "memory administration requester is not authenticated"
        );
        Ok(())
    }
}

#[async_trait]
impl MemoryAdminUseCases for ProductionMemoryAdminUseCases {
    async fn preview_forget(
        &self,
        policy: mnemosyne::ForgetPolicy,
    ) -> anyhow::Result<mnemosyne::ForgetReceipt> {
        self.authenticate(&policy.requester)?;
        self.service.preview_forget(policy).await
    }

    async fn tombstone(
        &self,
        policy: mnemosyne::ForgetPolicy,
    ) -> anyhow::Result<mnemosyne::ForgetReceipt> {
        self.authenticate(&policy.requester)?;
        self.service.forget(policy).await
    }

    async fn compact_retention(
        &self,
        owner: &str,
        now_ms: i64,
        policy: mnemosyne::RetentionCompactionPolicy,
    ) -> anyhow::Result<mnemosyne::RetentionCompactionReport> {
        anyhow::ensure!(
            owner == self.authenticated_principal,
            "memory compaction owner is not authenticated"
        );
        self.retention.compact(owner, now_ms, &policy)
    }
}

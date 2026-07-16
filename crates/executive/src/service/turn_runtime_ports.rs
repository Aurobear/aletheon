//! Synchronous, turn-blocking runtime ports.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use async_trait::async_trait;
use fabric::hook::{HookContext, HookResult};
use fabric::{
    AdmissionController, Clock, ContentBlock, Context, LlmProvider, Message, Role, Timer,
    ToolDefinition, Verdict,
};
use tokio::sync::{mpsc, Mutex};

use crate::r#impl::daemon::handler::tool_executor::{prepare_corpus, TurnToolExecutor};
use crate::r#impl::daemon::model_router::ModelRouter;
use crate::service::governed_capability::{
    CapabilityExecutionContext, CapabilityRuntimeFactory, RegistryAuthorityProvider,
    TurnCapabilityInvoker,
};

#[async_trait]
pub trait TurnHookPort: Send + Sync {
    async fn run_pre_turn_script(&self, message: &str, session_id: &str) -> String;
    async fn execute(&self, context: HookContext) -> HookResult;
}

#[async_trait]
pub trait StormStatePort: Send + Sync {
    async fn reset(&self);
    async fn failure_count(&self) -> usize;
}

pub trait ModelSelectionPort: Send + Sync {
    fn select(&self, message: &str) -> Arc<dyn LlmProvider>;
}

#[async_trait]
pub trait SelfPolicyPort: Send + Sync {
    async fn review(&self, intent: &fabric::Intent, context: &Context) -> anyhow::Result<Verdict>;
    async fn narrate(&self, event: &str, reason: &str);
    async fn coordinate(&self, turn: usize, output: &str, status: fabric::dasein::OutcomeStatus);
    fn dasein_context_provider(&self) -> Arc<dyn Fn() -> Option<String> + Send + Sync>;
}

#[async_trait]
pub trait TurnSessionStatePort: Send + Sync {
    async fn current(&self) -> anyhow::Result<(String, usize)>;
    async fn begin_user(&self, message: &str) -> anyhow::Result<(String, usize)>;
    async fn finish(
        &self,
        succeeded: bool,
        tool_calls: &[(String, String, serde_json::Value)],
        tool_results: &[(String, String, bool)],
        output: &str,
    ) -> anyhow::Result<usize>;
}

#[async_trait]
pub trait TurnConfigPort: Send + Sync {
    async fn config(&self) -> crate::core::config::ExecutiveConfig;
}

pub trait TurnObservabilityPort: Send + Sync {
    fn record_turn(&self, tokens_in: u64, tokens_out: u64);
}

#[derive(Clone, Debug)]
pub struct ApprovalNotice {
    pub approval_id: String,
    pub tool: String,
    pub action_summary: String,
    pub risk_level: String,
    pub detail: Option<String>,
}

#[async_trait]
pub trait TurnApprovalPort: Send + Sync {
    async fn next(&self) -> Option<ApprovalNotice>;
}

pub struct PreparedCapabilities {
    pub definitions: Vec<ToolDefinition>,
    pub invoker: Arc<dyn TurnCapabilityInvoker>,
}

#[async_trait]
pub trait GovernedTurnCapabilityPort: Send + Sync {
    async fn prepare(
        &self,
        context: CapabilityExecutionContext,
    ) -> anyhow::Result<PreparedCapabilities>;
}

pub struct TurnRuntimePorts {
    pub hooks: Arc<dyn TurnHookPort>,
    pub storm: Arc<dyn StormStatePort>,
    pub models: Arc<dyn ModelSelectionPort>,
    pub self_policy: Arc<dyn SelfPolicyPort>,
    pub approvals: Arc<dyn TurnApprovalPort>,
    pub capabilities: Arc<dyn GovernedTurnCapabilityPort>,
    pub sessions: Arc<dyn TurnSessionStatePort>,
    pub config: Arc<dyn TurnConfigPort>,
    pub observability: Arc<dyn TurnObservabilityPort>,
}

pub(crate) struct TurnRuntimeResources {
    pub(crate) corpus: Arc<dyn corpus::CorpusService>,
    pub(crate) pre_turn_scripts: Vec<String>,
    pub(crate) storm: Arc<Mutex<corpus::security::storm_breaker::StormBreaker>>,
    pub(crate) model_router: Arc<ModelRouter>,
    pub(crate) default_llm: Arc<dyn LlmProvider>,
    pub(crate) self_field: Arc<Mutex<dasein::SelfField>>,
    pub(crate) approval_rx:
        Arc<Mutex<mpsc::Receiver<corpus::security::socket_approval::PendingApproval>>>,
    pub(crate) pending_approvals: Arc<
        Mutex<
            HashMap<
                String,
                tokio::sync::oneshot::Sender<corpus::security::approval::ApprovalDecision>,
            >,
        >,
    >,
    pub(crate) capabilities: crate::r#impl::daemon::handler::tool_executor::CapabilityResources,
    pub(crate) admission: Arc<dyn AdmissionController>,
    pub(crate) sessions: Arc<
        Mutex<HashMap<String, Arc<Mutex<crate::r#impl::daemon::session_manager::SessionManager>>>>,
    >,
    pub(crate) default_session_id: Arc<Mutex<String>>,
    pub(crate) session_created_at: Arc<Mutex<HashMap<String, fabric::MonoTime>>>,
    pub(crate) data_dir: std::path::PathBuf,
    pub(crate) context_window: usize,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) memory: Arc<dyn mnemosyne::MemoryService>,
    pub(crate) executive: Arc<Mutex<crate::core::orchestrator::AletheonExecutive>>,
    pub(crate) performance: Arc<fabric::kernel::debug_bus::PerfCounter>,
}

impl TurnRuntimePorts {
    pub(crate) fn production(resources: TurnRuntimeResources) -> Self {
        let corpus = resources.corpus;
        let execute_hook: Arc<HookExecutionFn> = Arc::new(move |context| {
            let corpus = corpus.clone();
            Box::pin(async move { corpus.execute_hook(&context).await })
        });
        let hooks: Arc<dyn TurnHookPort> = Arc::new(ProductionTurnHooks {
            execute_hook,
            pre_turn_scripts: resources.pre_turn_scripts,
        });
        Self {
            hooks: hooks.clone(),
            storm: Arc::new(ProductionStormState {
                state: resources.storm,
            }),
            models: Arc::new(ProductionModelSelection {
                router: resources.model_router,
                default_llm: resources.default_llm.clone(),
            }),
            self_policy: Arc::new(ProductionSelfPolicy {
                field: resources.self_field,
            }),
            approvals: Arc::new(ProductionTurnApprovals {
                receiver: resources.approval_rx,
                pending: resources.pending_approvals,
            }),
            capabilities: Arc::new(ProductionGovernedCapabilities {
                resources: resources.capabilities,
                admission: resources.admission,
            }),
            sessions: Arc::new(ProductionTurnSessions {
                registry: resources.sessions,
                default_id: resources.default_session_id,
                created_at: resources.session_created_at,
                data_dir: resources.data_dir,
                context_window: resources.context_window,
                clock: resources.clock,
                llm: resources.default_llm,
                memory_service: resources.memory,
                hooks,
            }),
            config: Arc::new(ProductionTurnConfig {
                config_source: resources.executive,
            }),
            observability: Arc::new(ProductionTurnObservability {
                performance: resources.performance,
            }),
        }
    }
}

struct ProductionTurnHooks {
    execute_hook: Arc<HookExecutionFn>,
    pre_turn_scripts: Vec<String>,
}

type HookExecutionFn =
    dyn Fn(HookContext) -> Pin<Box<dyn Future<Output = HookResult> + Send>> + Send + Sync;

#[async_trait]
impl TurnHookPort for ProductionTurnHooks {
    async fn run_pre_turn_script(&self, message: &str, session_id: &str) -> String {
        let input = serde_json::json!({"prompt":message,"session_id":session_id}).to_string();
        let mut output = String::new();
        for script in &self.pre_turn_scripts {
            let path = crate::r#impl::daemon::handler::format::expand_tilde(script);
            if !std::path::Path::new(&path).exists() {
                tracing::warn!(%path, "Hook script not found, skipping");
                continue;
            }
            let spawned = tokio::process::Command::new(&path)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn();
            let Ok(mut child) = spawned else {
                tracing::warn!(%path, "Failed to spawn hook script");
                continue;
            };
            if let Some(mut stdin) = child.stdin.take() {
                let input = input.clone();
                tokio::spawn(async move {
                    use tokio::io::AsyncWriteExt;
                    let _ = stdin.write_all(input.as_bytes()).await;
                });
            }
            let mut stdout = child.stdout.take();
            match aletheon_kernel::chronos::SystemTimer
                .timeout(Duration::from_secs(30), child.wait())
                .await
            {
                Ok(Ok(status)) if status.success() => {
                    if let Some(ref mut pipe) = stdout {
                        use tokio::io::AsyncReadExt;
                        let mut fragment = String::new();
                        if pipe.read_to_string(&mut fragment).await.is_ok() && !fragment.is_empty()
                        {
                            output.push_str("\n[Hook output]\n");
                            output.push_str(&fragment);
                            output.push('\n');
                        }
                    }
                }
                Ok(Ok(status)) => tracing::warn!(%path, code=?status.code(), "Hook script failed"),
                Ok(Err(error)) => tracing::warn!(%path, %error, "Hook script I/O error"),
                Err(_) => {
                    tracing::warn!(%path, "Hook script timed out");
                    let _ = child.kill().await;
                }
            }
        }
        output
    }

    async fn execute(&self, context: HookContext) -> HookResult {
        (self.execute_hook)(context).await
    }
}

struct ProductionStormState {
    state: Arc<Mutex<corpus::security::storm_breaker::StormBreaker>>,
}

#[async_trait]
impl StormStatePort for ProductionStormState {
    async fn reset(&self) {
        self.state.lock().await.reset();
    }

    async fn failure_count(&self) -> usize {
        self.state.lock().await.failure_count()
    }
}

struct ProductionModelSelection {
    router: Arc<ModelRouter>,
    default_llm: Arc<dyn LlmProvider>,
}

impl ModelSelectionPort for ProductionModelSelection {
    fn select(&self, message: &str) -> Arc<dyn LlmProvider> {
        let task = self.router.classify_message(message);
        match self.router.create_provider(task) {
            Ok(provider) => {
                tracing::info!(task=?task, model=provider.name(), "Model selected by router");
                Arc::from(provider)
            }
            Err(error) => {
                tracing::warn!(%error, task=?task, "ModelRouter failed, using default");
                self.default_llm.clone()
            }
        }
    }
}

struct ProductionSelfPolicy {
    field: Arc<Mutex<dasein::SelfField>>,
}

#[async_trait]
impl SelfPolicyPort for ProductionSelfPolicy {
    async fn review(&self, intent: &fabric::Intent, context: &Context) -> anyhow::Result<Verdict> {
        use fabric::SelfFieldOps;
        self.field.lock().await.review(intent, context).await
    }

    async fn narrate(&self, event: &str, reason: &str) {
        use fabric::SelfFieldOps;
        let _ = self.field.lock().await.narrate(event, reason).await;
    }

    async fn coordinate(&self, turn: usize, output: &str, status: fabric::dasein::OutcomeStatus) {
        let field = self.field.lock().await;
        if let Some(dasein) = field.dasein() {
            match dasein.record_outcome(output, status, "turn-pipeline").await {
                Ok(receipt) => tracing::info!(
                    turn,
                    version = receipt.current_version.0,
                    "Dasein outcome accepted"
                ),
                Err(error) => tracing::warn!(turn, %error, "Dasein outcome rejected"),
            }
        }
    }

    fn dasein_context_provider(&self) -> Arc<dyn Fn() -> Option<String> + Send + Sync> {
        let field = self.field.clone();
        Arc::new(move || {
            field
                .try_lock()
                .ok()
                .and_then(|field| field.dasein_prompt_injection())
        })
    }
}

struct ProductionTurnSessions {
    registry: Arc<
        Mutex<HashMap<String, Arc<Mutex<crate::r#impl::daemon::session_manager::SessionManager>>>>,
    >,
    default_id: Arc<Mutex<String>>,
    created_at: Arc<Mutex<HashMap<String, fabric::MonoTime>>>,
    data_dir: std::path::PathBuf,
    context_window: usize,
    clock: Arc<dyn Clock>,
    llm: Arc<dyn LlmProvider>,
    memory_service: Arc<dyn mnemosyne::MemoryService>,
    hooks: Arc<dyn TurnHookPort>,
}

impl ProductionTurnSessions {
    async fn manager(
        &self,
    ) -> anyhow::Result<(
        String,
        Arc<Mutex<crate::r#impl::daemon::session_manager::SessionManager>>,
    )> {
        let session_id = self.default_id.lock().await.clone();
        if let Some(manager) = self.registry.lock().await.get(&session_id).cloned() {
            return Ok((session_id, manager));
        }
        let manager = crate::r#impl::daemon::session_manager::SessionManager::new(
            &self.data_dir,
            session_id.clone(),
            self.context_window,
            self.clock.clone(),
        )
        .await?;
        let manager = Arc::new(Mutex::new(manager));
        self.registry
            .lock()
            .await
            .insert(session_id.clone(), manager.clone());
        self.created_at
            .lock()
            .await
            .insert(session_id.clone(), self.clock.mono_now());
        Ok((session_id, manager))
    }
}

#[async_trait]
impl TurnSessionStatePort for ProductionTurnSessions {
    async fn current(&self) -> anyhow::Result<(String, usize)> {
        let (session_id, manager) = self.manager().await?;
        let turn_count = manager.lock().await.turn_count();
        Ok((session_id, turn_count))
    }

    async fn begin_user(&self, message: &str) -> anyhow::Result<(String, usize)> {
        let (session_id, manager) = self.manager().await?;
        let turn_count = {
            let mut manager = manager.lock().await;
            manager.push_user(message).await;
            manager.turn_count()
        };
        if let Err(error) = self
            .memory_service
            .record(mnemosyne::ExperienceEvent::Message {
                session: session_id.clone(),
                role: "user".into(),
                content: message.to_owned(),
                metadata: mnemosyne::MemoryMetadata::local(
                    format!("message:{session_id}:user:{turn_count}"),
                    format!("{session_id}:user:{turn_count}"),
                    fabric::wall_to_datetime(self.clock.wall_now()),
                ),
            })
            .await
        {
            tracing::warn!(%error, "user memory projection failed");
        }
        self.hooks
            .execute(HookContext {
                point: fabric::hook::HookPoint::OnMemoryStore,
                session_id: session_id.clone(),
                turn_count,
                tool_name: None,
                tool_input: None,
                tool_result: None,
                message: Some(message.to_owned()),
                metadata: HashMap::new(),
            })
            .await;
        Ok((session_id, turn_count))
    }

    async fn finish(
        &self,
        succeeded: bool,
        tool_calls: &[(String, String, serde_json::Value)],
        tool_results: &[(String, String, bool)],
        output: &str,
    ) -> anyhow::Result<usize> {
        let (_, manager) = self.manager().await?;
        let mut manager = manager.lock().await;
        if succeeded {
            if !tool_calls.is_empty() {
                manager
                    .push_message(Message {
                        role: Role::Assistant,
                        content: tool_calls
                            .iter()
                            .map(|(id, name, input)| ContentBlock::ToolUse {
                                id: id.clone(),
                                name: name.clone(),
                                input: input.clone(),
                            })
                            .collect(),
                    })
                    .await;
                manager
                    .push_message(Message {
                        role: Role::User,
                        content: tool_results
                            .iter()
                            .map(|(id, content, is_error)| ContentBlock::ToolResult {
                                tool_use_id: id.clone(),
                                content: content.clone(),
                                is_error: *is_error,
                            })
                            .collect(),
                    })
                    .await;
            }
            manager.push_assistant(output).await;
            let _ = manager.compact_if_needed(&*self.llm).await;
        }
        Ok(manager.turn_count())
    }
}

struct ProductionTurnConfig {
    config_source: Arc<Mutex<crate::core::orchestrator::AletheonExecutive>>,
}

#[async_trait]
impl TurnConfigPort for ProductionTurnConfig {
    async fn config(&self) -> crate::core::config::ExecutiveConfig {
        self.config_source.lock().await.config().clone()
    }
}

struct ProductionTurnObservability {
    performance: Arc<fabric::kernel::debug_bus::PerfCounter>,
}

impl TurnObservabilityPort for ProductionTurnObservability {
    fn record_turn(&self, tokens_in: u64, tokens_out: u64) {
        self.performance.record_turn(tokens_in, tokens_out);
    }
}

struct ProductionTurnApprovals {
    receiver: Arc<Mutex<mpsc::Receiver<corpus::security::socket_approval::PendingApproval>>>,
    pending: Arc<
        Mutex<
            HashMap<
                String,
                tokio::sync::oneshot::Sender<corpus::security::approval::ApprovalDecision>,
            >,
        >,
    >,
}

#[async_trait]
impl TurnApprovalPort for ProductionTurnApprovals {
    async fn next(&self) -> Option<ApprovalNotice> {
        let pending = self.receiver.lock().await.recv().await?;
        let approval_id = uuid::Uuid::new_v4().to_string();
        let notice = ApprovalNotice {
            approval_id: approval_id.clone(),
            tool: pending.request.tool,
            action_summary: pending.request.action_summary,
            risk_level: pending.request.risk_level,
            detail: pending.request.detail,
        };
        self.pending
            .lock()
            .await
            .insert(approval_id, pending.respond);
        Some(notice)
    }
}

struct ProductionGovernedCapabilities {
    resources: crate::r#impl::daemon::handler::tool_executor::CapabilityResources,
    admission: Arc<dyn AdmissionController>,
}

#[async_trait]
impl GovernedTurnCapabilityPort for ProductionGovernedCapabilities {
    async fn prepare(
        &self,
        context: CapabilityExecutionContext,
    ) -> anyhow::Result<PreparedCapabilities> {
        let prepared = prepare_corpus(&self.resources, &context).await?;
        let definitions = prepared
            .snapshot
            .entries
            .iter()
            .filter_map(|entry| entry.tool_definition.clone())
            .collect();
        let executor = Arc::new(TurnToolExecutor::new(
            &self.resources,
            prepared.executor,
            context.session_id.clone(),
            context.turn_count,
            context.working_dir.clone(),
            context.operation_id,
            context.process_id,
        ));
        let authority = Arc::new(
            RegistryAuthorityProvider::new(
                prepared.risk_by_tool,
                context.principal,
                context.session_id,
                context.working_dir,
                context.sandbox,
                context.cancel,
            )
            .with_agent_context(context.agent),
        );
        let invoker = match context.action_loop {
            Some(action_loop) => CapabilityRuntimeFactory::build_with_action_loop(
                self.admission.clone(),
                executor,
                authority,
                action_loop,
            ),
            None => CapabilityRuntimeFactory::build(self.admission.clone(), executor, authority),
        };
        Ok(PreparedCapabilities {
            definitions,
            invoker,
        })
    }
}

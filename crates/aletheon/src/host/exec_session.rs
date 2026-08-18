//! CLI `exec` session builder — shared factory for non-daemon single-turn execution.
//!
//! The CLI keeps its lighter orchestration because it does not own the daemon's
//! long-lived infrastructure. Daemon, CLI, and native Agent turns nevertheless
//! cross the same Cognit `CognitiveSession`/factory boundary. Interactive daemon
//! turns select the streaming session operation; CLI and native Agent turns use
//! the ordinary operation. Concrete harness construction remains in the
//! Aletheon composition adapter.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::info;

use ::contracts::types::admission::RiskLevel;
use ::contracts::{
    CapabilityCall, CapabilityResult, Clock, ItemPayload, LlmProvider, Message, PrincipalId,
    ProcessId, RecallSet, SandboxRequirement, Timer, ToolDefinition, TurnRequest, TurnServices,
};
use cognit::harness::{CognitiveSessionFactory, HarnessConfig, LinearCognitiveSessionFactory};
use kernel::chronos::{SystemClock, SystemTimer};
use kernel::KernelRuntime;
use tokio_util::sync::CancellationToken;

use crate::host::session::exec_turn_service::RecordingTurnServices;
use crate::host::session::exec_turn_service::TurnService;
use adapters_sqlite::session::canonical_store::CanonicalSessionStore;
use application::turn::coordinator::{cancelled_result, TurnCoordinator, TurnExecution};
use application::turn::settings::ResolvedTurnProfile;
use application::turn::{
    TurnEngine, TurnEngineContext, TurnEngineError, TurnEngineRequest, TurnEngineResult,
    TurnEngineStream,
};
use cognit::ports::inference::{InferencePort, PortLlmProvider};
use kernel::capability::governed::{
    CapabilityRuntimeFactory, RegistryAuthorityProvider, TurnCapabilityInvoker,
};
use runtime::post_turn::PostTurnPipeline;
use runtime::pre_turn::PreTurnPipeline;
use runtime::turn_policy::TurnPolicy;
use std::time::Duration;

/// Builder for a CLI `exec` session (non-daemon, single-turn).
pub struct ExecSessionBuilder {
    config_path: Option<PathBuf>,
    model: String,
    max_turns: usize,
    working_dir: PathBuf,
    sandbox: String,
    inference: Option<Arc<dyn InferencePort>>,
    cancellation: CancellationToken,
}

#[derive(Default)]
pub struct ExecSessionFacts {
    approval_unavailable: AtomicBool,
}

impl ExecSessionFacts {
    pub fn approval_unavailable(&self) -> bool {
        self.approval_unavailable.load(Ordering::SeqCst)
    }
}

/// Exec-specific adapter for the authoritative TurnEngine boundary. It uses
/// the same Runtime-owned TurnCoordinator reducer as daemon execution while
/// retaining the lightweight CLI Cognit composition.
struct ExecTurnEngine {
    services: Arc<dyn TurnServices>,
    pre_turn: PreTurnPipeline,
    post_turn: PostTurnPipeline,
    factory: Arc<dyn CognitiveSessionFactory>,
    clock: Arc<dyn Clock>,
    coordinator: Arc<TurnCoordinator>,
    policy: TurnPolicy,
}

impl ExecTurnEngine {
    fn new(
        services: Arc<dyn TurnServices>,
        kernel: Arc<KernelRuntime>,
        coordinator: Arc<TurnCoordinator>,
        harness_config: HarnessConfig,
    ) -> Self {
        Self {
            services,
            pre_turn: PreTurnPipeline,
            post_turn: PostTurnPipeline,
            factory: Arc::new(LinearCognitiveSessionFactory::new(
                harness_config,
                kernel.clock(),
            )),
            clock: kernel.clock(),
            coordinator,
            policy: TurnPolicy::exec(),
        }
    }

    async fn execute_inner(
        &self,
        request: TurnEngineRequest,
        context: TurnEngineContext,
        events: &dyn TurnEngineStream,
    ) -> Result<TurnEngineResult, TurnEngineError> {
        let principal_context = context.require_principal_context()?;
        let services = self.services.clone();
        let pre_turn = self.pre_turn.clone();
        let factory = self.factory.clone();
        let clock = self.clock.clone();
        let policy = self.policy.clone();
        let runner_policy = policy.clone();
        let history_store = self.coordinator.session_port();
        let turn_id = Arc::new(std::sync::Mutex::new(None));
        let captured_turn_id = turn_id.clone();
        let turn_request = TurnRequest {
            operation_id: context.operation_id,
            process_id: context.process_id,
            context: principal_context,
            input: request.input,
            execution_target: request.execution_target,
            model_policy: request
                .model_policy
                .or(context.profile.model_policy.clone()),
            deadline: request.deadline,
            requirements: request.requirements,
            requested_task_kind: request.requested_task_kind,
            evaluation_contract: None,
        };
        let result = self
            .coordinator
            .submit_with(turn_request, &policy, move |request, cancel| async move {
                if let Some(runtime_turn_id) = request.context.turn_id.as_ref() {
                    events.bind_runtime_identity(request.operation_id, runtime_turn_id);
                    *captured_turn_id
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                        Some(runtime_turn_id.clone());
                }
                let session_record = ::contracts::SessionRecord {
                    schema_version: ::contracts::SESSION_SCHEMA_VERSION,
                    id: ::contracts::SessionId(request.context.thread_id.0.clone()),
                    parent: None,
                    created_at_ms: 0,
                    status: ::contracts::SessionStatus::Active,
                };
                let mut history = history_store
                    .load_items(&::contracts::SessionId(request.context.thread_id.0.clone()), None)
                    .await?;
                if history.last().is_some_and(|item| {
                    matches!(&item.payload, ItemPayload::UserMessage { content, .. } if content == &request.input)
                }) {
                    history.pop();
                }
                let canonical_seed = runtime::session_projection::project_messages(&history)?;
                let recording = RecordingTurnServices::new(services, canonical_seed);
                let request = pre_turn.run(request, &recording).await?;
                let mut session = factory
                    .create(&session_record, &runner_policy, cancel.clone())
                    .await?;
                let start = clock.mono_now();
                let run = session.run_turn(request.clone(), &recording, events);
                let mut result = match request.deadline {
                    Some(deadline) => tokio::select! {
                        _ = cancel.cancelled() => cancelled_result(),
                        timeout = SystemTimer.timeout(Duration::from_millis(deadline.0), run) => {
                            match timeout { Ok(result) => result?, Err(_) => cancelled_result() }
                        }
                    },
                    None => tokio::select! {
                        _ = cancel.cancelled() => cancelled_result(),
                        result = run => result?,
                    },
                };
                result.metrics.elapsed_ms = clock.mono_now().0.saturating_sub(start.0);
                Ok(TurnExecution {
                    result,
                    items: recording.take_items().await,
                    projection: None,
                    context_projection: None,
                    evaluation_artifacts: Default::default(),
                })
            })
            .await
            .map_err(TurnEngineError::Internal)?;
        let result = self
            .post_turn
            .run(result)
            .await
            .map_err(TurnEngineError::Internal)?;
        let turn_id = turn_id
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or_else(|| {
                TurnEngineError::Internal(anyhow::anyhow!(
                    "TurnCoordinator completed without assigning a TurnId"
                ))
            })?;
        let execution = TurnExecution {
            result: result.clone(),
            items: Vec::new(),
            projection: None,
            context_projection: None,
            evaluation_artifacts: Default::default(),
        };
        Ok(TurnEngineResult {
            turn_id,
            output: result.output,
            stop: result.stop,
            failure: result.failure,
            tool_calls: result.metrics.tool_calls_made,
            usage: result.usage,
            elapsed_ms: result.metrics.elapsed_ms,
            coordinator_execution: Some(execution),
        })
    }
}

#[async_trait::async_trait]
impl TurnEngine for ExecTurnEngine {
    async fn execute(
        &self,
        request: TurnEngineRequest,
        context: TurnEngineContext,
    ) -> Result<TurnEngineResult, TurnEngineError> {
        self.execute_inner(request, context, &::contracts::NoopTurnEventSink)
            .await
    }

    async fn execute_with_events(
        &self,
        request: TurnEngineRequest,
        context: TurnEngineContext,
        events: &dyn TurnEngineStream,
    ) -> Result<TurnEngineResult, TurnEngineError> {
        self.execute_inner(request, context, events).await
    }
}

impl ExecSessionBuilder {
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            config_path: None,
            model: String::new(),
            max_turns: 20,
            working_dir,
            sandbox: "auto".to_string(),
            inference: None,
            cancellation: CancellationToken::new(),
        }
    }

    pub fn with_config(mut self, path: PathBuf) -> Self {
        self.config_path = Some(path);
        self
    }

    pub fn with_model(mut self, model: String) -> Self {
        self.model = model;
        self
    }

    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns;
        self
    }

    pub fn with_sandbox(mut self, sandbox: String) -> Self {
        self.sandbox = sandbox;
        self
    }

    pub fn with_inference(mut self, inference: Arc<dyn InferencePort>) -> Self {
        self.inference = Some(inference);
        self
    }

    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    /// Wire up the full exec stack and return the turn service, provider view,
    /// risk level, and registered kernel process that owns the turn.
    pub async fn build(
        self,
    ) -> Result<(
        TurnService,
        Arc<dyn LlmProvider>,
        RiskLevel,
        ProcessId,
        Arc<ExecSessionFacts>,
    )> {
        let working_dir = self.working_dir.canonicalize().with_context(|| {
            format!("resolving exec workspace '{}'", self.working_dir.display())
        })?;

        // Load config
        let app_config =
            crate::config::load_for_host(Some(&self.working_dir), self.config_path.as_deref())?
                .value;

        let inference = self.inference.unwrap_or_else(|| {
            let socket = std::env::var_os("ALETHEON_CORE_SOCKET")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/run/aletheon/core.sock"));
            Arc::new(adapters_inference::CoreRpcClient::new(socket))
        });
        let model = if self.model.is_empty() {
            app_config.model_routing.default.clone().unwrap_or_default()
        } else {
            self.model.clone()
        };
        let llm: Arc<dyn LlmProvider> = Arc::new(PortLlmProvider::resolve(inference, model).await?);
        info!(provider = llm.name(), model = %self.model, "LLM provider initialized");

        let user_paths = ::contracts::paths::UserRuntimePaths::resolve(
            &::contracts::paths::ProcessRuntimeEnvironment,
        )?;
        user_paths.prepare()?;
        let audit_path = user_paths.state_root.join("exec-audit.jsonl");
        let clock = Arc::new(SystemClock::new());
        let session_id = uuid::Uuid::new_v4().to_string();
        let corpus_composition = crate::composition::exec_corpus::compose_exec_corpus(
            audit_path,
            &self.sandbox,
            clock.clone(),
            session_id.clone(),
        )
        .await?;
        let corpus = corpus_composition.service;
        let grant = corpus_composition.grant;

        let system_prompt = format!(
            "You are Aletheon, an AI agent executing a task non-interactively. \
             You have access to tools. Complete the user's request and provide a final response. \
             Working directory: {}",
            working_dir.display()
        );

        let event_db = user_paths.state_root.join("exec-events.db");
        let event_spine = Arc::new(adapters_sqlite::event_spine::SqliteEventSpine::open(
            event_db,
        )?);

        let kernel = Arc::new(KernelRuntime::new());
        let process = kernel
            .spawn_process(::contracts::SpawnSpec::default())
            .await?;

        let snapshot = corpus.catalog(&grant).await?;
        let activated = crate::extensions::ExtensionService::new(
            corpus.clone(),
            Arc::new(
                crate::extensions::extension_service::SpineExtensionDecisionSink::new(
                    event_spine.clone(),
                ),
            ),
        )
        .activate(
            grant,
            snapshot
                .entries
                .iter()
                .map(|entry| entry.id.clone())
                .collect(),
            &crate::extensions::SessionExtensionPolicy::default(),
        )
        .await?;
        let snapshot = activated.snapshot;
        let tool_definitions = snapshot
            .entries
            .iter()
            .filter_map(|entry| entry.tool_definition.clone())
            .collect::<Vec<_>>();
        let allowed_tools = tool_definitions
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<std::collections::HashSet<_>>();
        let tool_risks = snapshot
            .entries
            .iter()
            .filter_map(|entry| {
                entry
                    .primary_capability()
                    .map(|capability| (capability.0.clone(), entry.risk))
            })
            .collect();
        info!(tool_count = tool_definitions.len(), "Tools registered");
        let executor = Arc::new(corpus::ActivatedCorpusExecutor::new(
            corpus,
            activated.receipt.id,
        ));
        let cancellation = self.cancellation.clone();
        let authority = Arc::new(RegistryAuthorityProvider::new(
            tool_risks,
            PrincipalId("exec".into()),
            ::contracts::ConnectionId::new(),
            ::contracts::ThreadId(session_id.clone()),
            // Capability authority is constructed before Runtime admission;
            // it receives the canonical turn through the admitted request.
            // Do not mint a client-side core TurnId here.
            ::contracts::TurnId(uuid::Uuid::nil()),
            ::contracts::WorkspacePolicy::from_resolved_roots(working_dir.clone(), vec![])
                .map_err(anyhow::Error::msg)?,
            session_id,
            working_dir,
            SandboxRequirement::NotRequired,
            self.cancellation,
        ));
        let capability = CapabilityRuntimeFactory::build(kernel.admission(), executor, authority);

        let session_db = user_paths.state_root.join("exec-sessions-v1.db");
        if let Some(parent) = session_db.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let event_projections = Arc::new(
            adapters_sqlite::projection_set::DefaultEventProjectionSet::open(
                user_paths.state_root.join("exec-event-projections.db"),
            )?,
        );
        let coordinator = Arc::new(
            crate::composition::turn_coordinator::compose_turn_coordinator(
                kernel.clone(),
                Arc::new(CanonicalSessionStore::open(session_db)?),
                event_spine,
                event_projections,
                crate::config::GrokHardeningConfig::default(),
            ),
        );

        let facts = Arc::new(ExecSessionFacts::default());
        let services = Arc::new(ExecTurnServices {
            llm: llm.clone(),
            tool_definitions,
            system_prompt,
            capability,
            facts: facts.clone(),
        });

        let harness_config = HarnessConfig {
            max_iterations: self.max_turns,
            ..Default::default()
        };
        let profile = ResolvedTurnProfile {
            profile_name: "exec".into(),
            allowed_tools,
            delegated_tools: Default::default(),
            system_prompt: String::new(),
            model_policy: (!self.model.is_empty()).then_some(self.model.clone()),
            max_iterations: self.max_turns,
            max_input_tokens: llm.max_context_length() as u64,
            max_output_tokens: 16_384,
            tool_schema_tokens: 0.into(),
            max_tool_calls: self.max_turns as u32,
            max_elapsed_ms: 600_000,
            approval_policy: ::contracts::AgentApprovalPolicy::AutoApprove,
            tool_timeout_ms: 30_000,
        };
        let engine: Arc<dyn TurnEngine> = Arc::new(ExecTurnEngine::new(
            services,
            kernel,
            coordinator.clone(),
            harness_config,
        ));
        let turn_service = TurnService::from_engine(engine, coordinator, profile, cancellation);

        Ok((turn_service, llm, RiskLevel::ReadOnly, process.id, facts))
    }
}

// ── ExecTurnServices (private helper) ────────────────────────────────────

struct ExecTurnServices {
    llm: Arc<dyn LlmProvider>,
    tool_definitions: Vec<ToolDefinition>,
    system_prompt: String,
    capability: Arc<dyn TurnCapabilityInvoker>,
    facts: Arc<ExecSessionFacts>,
}

#[async_trait::async_trait]
impl TurnServices for ExecTurnServices {
    async fn recall(&self, _req: ::contracts::RecallRequest) -> Result<RecallSet> {
        Ok(RecallSet::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> Result<::contracts::DaseinView> {
        Ok(::contracts::DaseinView::default())
    }

    /// Exec sessions are single-user CLI runs with no shared workspace.
    /// Agora is intentionally absent — this is not a degraded daemon path.
    /// If shared cognitive workspace is ever needed in exec mode, inject
    /// an Aletheon-owned AgoraService handle here.
    async fn agora_view(&self, _session_id: &str) -> Result<::contracts::AgoraView> {
        Ok(::contracts::AgoraView::default())
    }

    async fn invoke(&self, req: CapabilityCall) -> CapabilityResult {
        self.capability.invoke(req).await
    }

    async fn record_capability_receipt(&self, receipt: ::contracts::CapabilityTerminalReceipt) {
        if receipt.error_class == Some(::contracts::CapabilityErrorClass::Permission) {
            self.facts
                .approval_unavailable
                .store(true, Ordering::SeqCst);
        }
    }

    fn llm_provider(&self) -> Option<&dyn LlmProvider> {
        Some(self.llm.as_ref())
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tool_definitions.clone()
    }

    fn seed_messages(&self, _request: &TurnRequest) -> Vec<Message> {
        vec![Message::system(&self.system_prompt)]
    }
}

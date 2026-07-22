//! CLI `exec` session builder — shared factory for non-daemon single-turn execution.
//!
//! The CLI keeps its lighter orchestration because it does not own the daemon's
//! long-lived infrastructure. Daemon, CLI, and native Agent turns nevertheless
//! cross the same Cognit `CognitiveSession`/factory boundary. Interactive daemon
//! turns select the streaming session operation; CLI and native Agent turns use
//! the ordinary operation. Concrete harness construction remains in the
//! Executive composition adapter.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::info;

use cognit::harness::HarnessConfig;
use fabric::types::admission::RiskLevel;
use fabric::{
    CapabilityCall, CapabilityResult, LlmProvider, Message, PrincipalId, ProcessId, RecallSet,
    SandboxRequirement, ToolDefinition, TurnRequest, TurnServices,
};
use kernel::chronos::SystemClock;
use kernel::KernelRuntime;
use tokio_util::sync::CancellationToken;

use crate::application::governed_capability::{
    CapabilityRuntimeFactory, RegistryAuthorityProvider, TurnCapabilityInvoker,
};
use crate::application::inference_port::{InferencePort, PortLlmProvider};
use crate::application::turn_coordinator::TurnCoordinator;
use crate::application::{PostTurnPipeline, PreTurnPipeline, TurnService};
use crate::r#impl::session::canonical_store::CanonicalSessionStore;

/// Builder for a CLI `exec` session (non-daemon, single-turn).
pub struct ExecSessionBuilder {
    config_path: Option<PathBuf>,
    model: String,
    max_turns: usize,
    working_dir: PathBuf,
    sandbox: String,
    inference: Option<Arc<dyn InferencePort>>,
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

    /// Wire up the full exec stack and return the turn service, provider view,
    /// risk level, and registered kernel process that owns the turn.
    pub async fn build(self) -> Result<(TurnService, Arc<dyn LlmProvider>, RiskLevel, ProcessId)> {
        let working_dir = self.working_dir.canonicalize().with_context(|| {
            format!("resolving exec workspace '{}'", self.working_dir.display())
        })?;

        // Load config
        let app_config = crate::composition::config::load_for_host(
            Some(&self.working_dir),
            self.config_path.as_deref(),
        )?
        .value;

        let inference = self.inference.unwrap_or_else(|| {
            let socket = std::env::var_os("ALETHEON_CORE_SOCKET")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/run/aletheon/core.sock"));
            Arc::new(crate::host::core_rpc::CoreRpcClient::new(socket))
        });
        let model = if self.model.is_empty() {
            app_config.model_routing.default.clone().unwrap_or_default()
        } else {
            self.model.clone()
        };
        let llm: Arc<dyn LlmProvider> = Arc::new(PortLlmProvider::new(inference, model));
        info!(provider = llm.name(), model = %self.model, "LLM provider initialized");

        let user_paths =
            fabric::paths::UserRuntimePaths::resolve(&fabric::paths::ProcessRuntimeEnvironment)?;
        user_paths.prepare()?;
        let audit_path = user_paths.state_root.join("exec-audit.jsonl");
        let clock = Arc::new(SystemClock::new());
        let session_id = uuid::Uuid::new_v4().to_string();
        let corpus_composition = crate::r#impl::exec_corpus::compose_exec_corpus(
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
        let event_spine = Arc::new(crate::r#impl::events::SqliteEventSpine::open(event_db)?);

        let kernel = Arc::new(KernelRuntime::new());
        let process = kernel.spawn_process(fabric::SpawnSpec::default()).await?;

        let snapshot = corpus.catalog(&grant).await?;
        let activated = crate::application::ExtensionService::new(
            corpus.clone(),
            Arc::new(
                crate::application::extension_service::SpineExtensionDecisionSink::new(
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
            &crate::application::SessionExtensionPolicy::default(),
        )
        .await?;
        let snapshot = activated.snapshot;
        let tool_definitions = snapshot
            .entries
            .iter()
            .filter_map(|entry| entry.tool_definition.clone())
            .collect::<Vec<_>>();
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
        let authority = Arc::new(RegistryAuthorityProvider::new(
            tool_risks,
            PrincipalId("exec".into()),
            fabric::ConnectionId::new(),
            fabric::ThreadId(session_id.clone()),
            fabric::TurnId::new(),
            fabric::WorkspacePolicy::from_resolved_roots(working_dir.clone(), vec![])
                .map_err(anyhow::Error::msg)?,
            session_id,
            working_dir,
            SandboxRequirement::NotRequired,
            CancellationToken::new(),
        ));
        let capability = CapabilityRuntimeFactory::build(kernel.admission(), executor, authority);

        let session_db = user_paths.state_root.join("exec-sessions-v1.db");
        if let Some(parent) = session_db.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let event_projections = Arc::new(crate::r#impl::events::DefaultEventProjectionSet::open(
            user_paths.state_root.join("exec-event-projections.db"),
        )?);
        let coordinator = Arc::new(
            TurnCoordinator::with_event_spine(
                kernel.clone(),
                Arc::new(CanonicalSessionStore::open(session_db)?),
                event_spine,
            )
            .with_event_projections(event_projections),
        );

        let services = Arc::new(ExecTurnServices {
            llm: llm.clone(),
            tool_definitions,
            system_prompt,
            capability,
        });

        let harness_config = HarnessConfig {
            max_iterations: self.max_turns,
            ..Default::default()
        };
        let turn_service = TurnService::new(services, PreTurnPipeline, PostTurnPipeline, kernel)
            .with_coordinator(coordinator)
            .with_harness_config(harness_config);

        Ok((turn_service, llm, RiskLevel::ReadOnly, process.id))
    }
}

// ── ExecTurnServices (private helper) ────────────────────────────────────

struct ExecTurnServices {
    llm: Arc<dyn LlmProvider>,
    tool_definitions: Vec<ToolDefinition>,
    system_prompt: String,
    capability: Arc<dyn TurnCapabilityInvoker>,
}

#[async_trait::async_trait]
impl TurnServices for ExecTurnServices {
    async fn recall(&self, _req: fabric::RecallRequest) -> Result<RecallSet> {
        Ok(RecallSet::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> Result<fabric::DaseinView> {
        Ok(fabric::DaseinView::default())
    }

    /// Exec sessions are single-user CLI runs with no shared workspace.
    /// Agora is intentionally absent — this is not a degraded daemon path.
    /// If shared cognitive workspace is ever needed in exec mode, inject
    /// an Executive-owned DomainPorts Agora handle here.
    async fn agora_view(&self, _session_id: &str) -> Result<fabric::AgoraView> {
        Ok(fabric::AgoraView::default())
    }

    async fn invoke(&self, req: CapabilityCall) -> CapabilityResult {
        self.capability.invoke(req).await
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

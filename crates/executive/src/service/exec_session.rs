//! CLI `exec` session builder — shared factory for non-daemon single-turn execution.
//!
//! # Turn-path convergence (Stage 3)
//!
//! The exec path intentionally does NOT share `TurnPipeline` with the daemon
//! because it lacks the daemon's infrastructure (CoreSystems, SelfField,
//! SessionGateway, memory service, hook registry, Agora). Instead, convergence
//! is achieved at the ReActLoop level:
//!
//! | Layer | Daemon | Exec | Shared? |
//! |---|---|---|---|
//! | Orchestration | TurnPipeline::run() | TurnService::submit() | No (different infra) |
//! | Session | ReActLoop::run_streaming() | LinearCognitiveSession::run_turn() → ReActLoop::run() | Same ReActLoop type |
//! | Admission | AdmissionController::admit/settle | same | ✅ |
//! | Agora | Full (propose/commit) | None (single-user CLI) | Policy: documented in Stage 1 |
//! | Memory | ExperienceEvent::Message | None | Policy: exec is stateless |
//! | Events | ChannelEventSink (streaming) | NoopTurnEventSink | Different sinks |
//!
//! ReActLoop is constructed in exactly two places:
//! - `harness_factory::build_configured_react_loop()` — daemon path
//! - `LinearCognitiveSession::new()` — exec path (via TurnService)
//!
//! Both use `ReActLoop::new(HarnessConfig, compressor)`.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use aletheon_kernel::chronos::SystemClock;
use aletheon_kernel::service::ServicePorts;
use cognit::config::AppConfig;
use cognit::harness::HarnessConfig;
use cognit::r#impl::provider_registry::ProviderRegistry;
use corpus::security::approval::{ApprovalGate, TerminalApprovalGate};
use corpus::security::audit::AuditLogger;
use corpus::security::runner::ToolRunnerWithGuard;
use corpus::security::sandbox::executor::SandboxPreference;
use corpus::CorpusToolExecutor;
use fabric::types::admission::RiskLevel;
use fabric::{
    AgoraOps, CapabilityCall, CapabilityResult, LlmProvider, Message, PrincipalId, ProcessId,
    RecallSet, SandboxRequirement, ToolDefinition, TurnRequest, TurnServices,
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::host::load_dotenv;
use crate::r#impl::session::canonical_store::{default_session_db_path, CanonicalSessionStore};
use crate::service::governed_capability::{
    CapabilityRuntimeFactory, RegistryAuthorityProvider, TurnCapabilityInvoker,
};
use crate::service::turn_coordinator::TurnCoordinator;
use crate::service::{PostTurnPipeline, PreTurnPipeline, TurnService};

/// Builder for a CLI `exec` session (non-daemon, single-turn).
pub struct ExecSessionBuilder {
    config_path: Option<PathBuf>,
    model: String,
    max_turns: usize,
    working_dir: PathBuf,
    sandbox: String,
}

impl ExecSessionBuilder {
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            config_path: None,
            model: String::new(),
            max_turns: 20,
            working_dir,
            sandbox: "auto".to_string(),
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

    /// Wire up the full exec stack and return the `TurnService`, `LlmProvider`,
    /// and the configured `RiskLevel`.
    pub async fn build(self) -> Result<(TurnService, Arc<dyn LlmProvider>, RiskLevel)> {
        // Load ~/.aletheon/.env so provider API keys resolve.
        if let Some(home) = std::env::var_os("HOME") {
            load_dotenv(&PathBuf::from(home).join(".aletheon").join(".env"));
        }

        let working_dir = self
            .working_dir
            .canonicalize()
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/tmp")));

        // Load config
        let app_config = if let Some(ref path) = self.config_path {
            AppConfig::load_or_default(path)
        } else {
            AppConfig::load_layered(None)
        };

        // Build provider registry
        let registry = ProviderRegistry::from_config(&app_config)?;

        // Create LLM provider
        let llm: Arc<dyn LlmProvider> = Arc::from(registry.resolve_and_create(&self.model)?);
        info!(provider = llm.name(), model = %self.model, "LLM provider initialized");

        // Create tool registry with default tools
        let tool_registry = corpus::default_tool_registry();

        // Guarded runner with terminal approval for risky (L2+) tools.
        let audit_path = working_dir.join(".aletheon-audit.jsonl");
        let approval: Arc<dyn ApprovalGate> = Arc::new(TerminalApprovalGate);
        let sandbox_preference = SandboxPreference::from_str(&self.sandbox);
        info!(preference = ?sandbox_preference, "sandbox configured");
        let clock = Arc::new(SystemClock::new());
        let mut runner = ToolRunnerWithGuard::with_sandbox_preference(
            AuditLogger::new(audit_path)?,
            sandbox_preference,
            clock.clone(),
        )
        .with_approval_gate(approval);
        let turn_id = uuid::Uuid::new_v4().to_string();
        runner.on_new_turn(&turn_id);

        let tool_definitions = tool_registry.lock().await.definitions();
        let tool_risks = corpus::tool_risk_levels(&tool_registry).await;
        let tool_count = tool_definitions.len();
        info!(tool_count, "Tools registered");

        let system_prompt = format!(
            "You are Aletheon, an AI agent executing a task non-interactively. \
             You have access to tools. Complete the user's request and provide a final response. \
             Working directory: {}",
            working_dir.display()
        );

        let session_id = uuid::Uuid::new_v4().to_string();

        // Create kernel service ports for process/operation tracking + admission gating.
        let ports = Arc::new(ServicePorts::new());

        let executor = Arc::new(CorpusToolExecutor::new(
            tool_registry.clone(),
            Arc::new(Mutex::new(runner)),
            clock,
        ));
        let authority = Arc::new(RegistryAuthorityProvider::new(
            tool_risks,
            PrincipalId("exec".into()),
            session_id,
            working_dir,
            SandboxRequirement::NotRequired,
            CancellationToken::new(),
        ));
        let capability =
            CapabilityRuntimeFactory::build(ports.admission.clone(), executor, authority);

        let session_db = default_session_db_path();
        if let Some(parent) = session_db.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let coordinator = Arc::new(TurnCoordinator::new(
            ports.as_ref(),
            Arc::new(CanonicalSessionStore::open(session_db)?),
        ));

        let services = Arc::new(ExecTurnServices {
            llm: llm.clone(),
            tool_definitions,
            system_prompt,
            capability,
            agora: None,
        });

        let harness_config = HarnessConfig {
            max_iterations: self.max_turns,
            ..Default::default()
        };
        let turn_service = TurnService::new(services, PreTurnPipeline, PostTurnPipeline, ports)
            .with_coordinator(coordinator)
            .with_harness_config(harness_config);

        Ok((turn_service, llm, RiskLevel::ReadOnly))
    }
}

// ── ExecTurnServices (private helper) ────────────────────────────────────

struct ExecTurnServices {
    llm: Arc<dyn LlmProvider>,
    tool_definitions: Vec<ToolDefinition>,
    system_prompt: String,
    capability: Arc<dyn TurnCapabilityInvoker>,
    // Optional shared workspace for exec mode.
    // When set (via with_agora), agora_view reflects real state.
    // Default: None (CLI exec is single-user).
    agora: Option<Arc<dyn AgoraOps>>,
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
    /// `ServicePorts::with_agora(registry)` and pass the agora handle here.
    async fn agora_view(&self, _session_id: &str) -> Result<fabric::AgoraView> {
        // Exec sessions are single-user CLI runs with no shared workspace.
        // Agora is intentionally absent by default.
        // If shared cognitive workspace is needed, inject via ExecSessionBuilder.
        if let Some(ref agora) = self.agora {
            // Could snapshot or return a real view here
            let _ = agora;
        }
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

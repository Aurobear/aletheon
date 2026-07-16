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

use anyhow::Result;
use tracing::info;

use aletheon_kernel::chronos::SystemClock;
use aletheon_kernel::KernelRuntime;
use cognit::config::AppConfig;
use cognit::harness::HarnessConfig;
use cognit::r#impl::provider_registry::ProviderRegistry;
use corpus::security::approval::{ApprovalGate, TerminalApprovalGate};
use corpus::security::audit::AuditLogger;
use corpus::security::runner::ToolRunnerWithGuard;
use corpus::security::sandbox::executor::SandboxPreference;
use corpus::CorpusService;
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

        let system_prompt = format!(
            "You are Aletheon, an AI agent executing a task non-interactively. \
             You have access to tools. Complete the user's request and provide a final response. \
             Working directory: {}",
            working_dir.display()
        );

        let session_id = uuid::Uuid::new_v4().to_string();

        let kernel = Arc::new(KernelRuntime::new());

        let raw_executor = Arc::new(corpus::CorpusToolExecutor::new(
            tool_registry.clone(),
            Arc::new(Mutex::new(runner)),
            clock.clone(),
        ));
        let corpus: Arc<dyn CorpusService> = Arc::new(corpus::DefaultCorpusService::from_runtime(
            tool_registry.clone(),
            raw_executor,
            Arc::new(Mutex::new(corpus::HookRegistry::new(clock))),
        ));
        let descriptors = corpus::discover_tool_extensions(&tool_registry).await?;
        let grant = corpus::ExtensionGrant {
            grant_id: uuid::Uuid::new_v4().to_string(),
            principal: PrincipalId("exec".into()),
            session_id: session_id.clone(),
            agent_id: None,
            capabilities: descriptors
                .iter()
                .map(|descriptor| descriptor.capability.clone())
                .collect(),
            resources: Default::default(),
        };
        let snapshot = corpus.catalog(&grant).await?;
        let activation = corpus
            .activate(corpus::ActivationRequest {
                grant,
                extensions: snapshot
                    .entries
                    .iter()
                    .map(|entry| entry.id.clone())
                    .collect(),
            })
            .await?;
        let tool_definitions = snapshot
            .entries
            .iter()
            .filter_map(|entry| entry.tool_definition.clone())
            .collect::<Vec<_>>();
        let tool_risks = snapshot
            .entries
            .iter()
            .map(|entry| (entry.capability.0.clone(), entry.risk))
            .collect();
        info!(tool_count = tool_definitions.len(), "Tools registered");
        let executor = Arc::new(corpus::ActivatedCorpusExecutor::new(corpus, activation.id));
        let authority = Arc::new(RegistryAuthorityProvider::new(
            tool_risks,
            PrincipalId("exec".into()),
            session_id,
            working_dir,
            SandboxRequirement::NotRequired,
            CancellationToken::new(),
        ));
        let capability = CapabilityRuntimeFactory::build(kernel.admission(), executor, authority);

        let session_db = default_session_db_path();
        if let Some(parent) = session_db.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let event_db = crate::r#impl::events::default_event_spine_path();
        let event_projections = Arc::new(crate::r#impl::events::DefaultEventProjectionSet::open(
            crate::r#impl::events::default_event_projection_path(),
        )?);
        let coordinator = Arc::new(
            TurnCoordinator::with_event_spine(
                kernel.clone(),
                Arc::new(CanonicalSessionStore::open(session_db)?),
                Arc::new(crate::r#impl::events::SqliteEventSpine::open(event_db)?),
            )
            .with_event_projections(event_projections),
        );

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
        let turn_service = TurnService::new(services, PreTurnPipeline, PostTurnPipeline, kernel)
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
    /// an Executive-owned DomainPorts Agora handle here.
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

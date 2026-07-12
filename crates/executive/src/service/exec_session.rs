//! CLI `exec` session builder — shared factory for non-daemon single-turn execution.
//!
//! Wraps the manual wiring previously duplicated in `crates/bin/src/main.rs`
//! so the binary crate only depends on `executive + interact`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use aletheon_kernel::service::ServicePorts;
use cognit::config::AppConfig;
use cognit::harness::HarnessConfig;
use cognit::r#impl::provider_registry::ProviderRegistry;
use corpus::security::approval::{ApprovalGate, TerminalApprovalGate};
use corpus::security::audit::AuditLogger;
use corpus::security::runner::ToolRunnerWithGuard;
use corpus::security::sandbox::executor::SandboxPreference;
use corpus::tools::tools::{ToolContext, ToolRegistry};
use fabric::types::admission::RiskLevel;
use fabric::{
    AdmissionController, AdmissionRequest, CapabilityId, CapabilityRequest, CapabilityResult,
    CapabilityScope, LlmProvider, Message, MonoTime, PrincipalId, RecallSet, SandboxRequirement,
    ToolDefinition, TurnRequest, TurnServices, UsageReport, ProcessId,
};

use crate::host::load_dotenv;
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
    pub async fn build(self) -> Result<(
        TurnService,
        Arc<dyn LlmProvider>,
        RiskLevel,
    )> {
        // Load ~/.aletheon/.env so provider API keys resolve.
        if let Some(home) = std::env::var_os("HOME") {
            load_dotenv(&PathBuf::from(home).join(".aletheon").join(".env"));
        }

        let working_dir = self.working_dir.canonicalize().unwrap_or_else(|_| {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/tmp"))
        });

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
        let tool_registry = Arc::new(ToolRegistry::default());

        // Guarded runner with terminal approval for risky (L2+) tools.
        let audit_path = working_dir.join(".aletheon-audit.jsonl");
        let approval: Arc<dyn ApprovalGate> = Arc::new(TerminalApprovalGate);
        let sandbox_preference = SandboxPreference::from_str(&self.sandbox);
        info!(preference = ?sandbox_preference, "sandbox configured");
        let mut runner = ToolRunnerWithGuard::with_sandbox_preference(
            AuditLogger::new(audit_path)?,
            sandbox_preference,
        )
        .with_approval_gate(approval);
        let turn_id = uuid::Uuid::new_v4().to_string();
        runner.on_new_turn(&turn_id);

        let tool_count = tool_registry.definitions().len();
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

        let services = Arc::new(ExecTurnServices {
            llm: llm.clone(),
            tool_registry,
            runner: tokio::sync::Mutex::new(runner),
            tool_ctx: ToolContext {
                working_dir: working_dir.clone(),
                session_id: session_id.clone(),
            },
            turn_id,
            system_prompt,
            admission: ports.admission.clone(),
        });

        let harness_config = HarnessConfig {
            max_iterations: self.max_turns,
            ..Default::default()
        };
        let turn_service = TurnService::new(
            services,
            PreTurnPipeline,
            PostTurnPipeline,
            ports,
        )
        .with_harness_config(harness_config);

        Ok((turn_service, llm, RiskLevel::ReadOnly))
    }
}

// ── ExecTurnServices (private helper) ────────────────────────────────────

struct ExecTurnServices {
    llm: Arc<dyn LlmProvider>,
    tool_registry: Arc<ToolRegistry>,
    runner: tokio::sync::Mutex<ToolRunnerWithGuard>,
    tool_ctx: ToolContext,
    turn_id: String,
    system_prompt: String,
    admission: Arc<dyn AdmissionController>,
}

#[async_trait::async_trait]
impl TurnServices for ExecTurnServices {
    async fn recall(&self, _req: fabric::RecallRequest) -> Result<RecallSet> {
        Ok(RecallSet::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> Result<fabric::DaseinView> {
        Ok(fabric::DaseinView::default())
    }

    async fn agora_view(&self, _session_id: &str) -> Result<fabric::AgoraView> {
        Ok(fabric::AgoraView::default())
    }

    async fn invoke(&self, req: CapabilityRequest) -> CapabilityResult {
        // Route all tool invocations through admission controller.
        let adm_req = AdmissionRequest {
            operation_id: req.operation_id,
            process_id: req.process_id,
            principal: PrincipalId("exec".into()),
            capability: CapabilityId(req.name.clone()),
            action: req.name.clone(),
            input_summary: format!("{:?}", req.input).chars().take(200).collect(),
            risk: RiskLevel::ReadOnly,
            requested_scope: CapabilityScope::default(),
            budget: None,
            lease: None,
            sandbox: SandboxRequirement::NotRequired,
        };

        let permit = match self.admission.admit(adm_req).await {
            Ok(p) => p,
            Err(e) => {
                return CapabilityResult {
                    call_id: req.call_id,
                    output: format!("admission denied: {e}"),
                    is_error: true,
                    usage: UsageReport::default(),
                    audit_id: None,
                };
            }
        };

        if !permit.is_valid_at(MonoTime(0)) {
            return CapabilityResult {
                call_id: req.call_id,
                output: "admission permit invalid".into(),
                is_error: true,
                usage: UsageReport {
                    permit_id: permit.id,
                    ..Default::default()
                },
                audit_id: Some(fabric::AuditEventId::new()),
            };
        }

        let Some(tool) = self.tool_registry.get(&req.name).cloned() else {
            let _ = self
                .admission
                .settle(permit.id, UsageReport::default())
                .await;
            return CapabilityResult {
                call_id: req.call_id,
                output: format!("Error: Unknown tool '{}'", req.name),
                is_error: true,
                usage: UsageReport {
                    permit_id: permit.id,
                    ..Default::default()
                },
                audit_id: Some(fabric::AuditEventId::new()),
            };
        };

        info!(tool = %req.name, "Executing tool (admitted)");
        let result = self
            .runner
            .lock()
            .await
            .run(tool.as_ref(), req.input, &self.tool_ctx, &self.turn_id)
            .await;

        let usage = UsageReport {
            permit_id: permit.id,
            output_bytes: result.content.len() as u64,
            exit_code: if result.is_error { Some(1) } else { Some(0) },
            ..Default::default()
        };
        let _ = self.admission.settle(permit.id, usage.clone()).await;

        if result.is_error {
            tracing::warn!(tool = %req.name, error = %result.content, "Tool failed/denied");
        } else {
            info!(tool = %req.name, "Tool succeeded");
        }
        CapabilityResult {
            call_id: req.call_id,
            output: result.content,
            is_error: result.is_error,
            usage,
            audit_id: Some(fabric::AuditEventId::new()),
        }
    }

    fn llm_provider(&self) -> Option<&dyn LlmProvider> {
        Some(self.llm.as_ref())
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tool_registry.definitions()
    }

    fn seed_messages(&self, _request: &TurnRequest) -> Vec<Message> {
        vec![Message::system(&self.system_prompt)]
    }
}

//! aletheon — unified entry point for Aletheon AI agent.
//!
//! Subcommands:
//!   (none)       TUI client (auto-starts daemon if not running)
//!   daemon       Start daemon (auto-detects systemd/container/foreground)
//!   exec         Non-interactive execution
//!   -m `msg`      Send single message to daemon
//!   version      Print version + git commit

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use executive::host::RuntimeHost;
use tracing::info;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "aletheon", about = "AI agent with sandbox, multi-agent, IPC")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Send a single message to the daemon
    #[arg(short = 'm', long = "message", value_name = "MSG")]
    message: Option<String>,

    /// Socket path (default: /run/aletheon/aletheon.sock)
    #[arg(short, long, default_value = "/run/aletheon/aletheon.sock")]
    socket: PathBuf,
}

#[derive(Subcommand)]
enum Commands {
    /// Start daemon (auto-detects systemd/container/foreground)
    Daemon {
        /// Path to config file
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Path to .env file
        #[arg(long)]
        env: Option<PathBuf>,
        /// Socket path (overrides parent --socket)
        #[arg(short, long)]
        socket: Option<PathBuf>,
        /// Force container mode (docker/podman)
        #[arg(long)]
        container: Option<String>,
        /// Container image name
        #[arg(long, default_value = "aletheon:latest")]
        image: String,
        /// Enable self-evolution loop (HIGH-risk autonomy -- OFF by default)
        #[arg(long, default_value_t = false)]
        enable_evolution: bool,
    },
    /// Non-interactive execution
    Exec {
        /// The prompt/task to execute
        #[arg(short, long)]
        prompt: String,
        /// Model spec
        #[arg(short, long, default_value = "")]
        model: String,
        /// Maximum agentic turns
        #[arg(short = 'n', long, default_value_t = 20)]
        max_turns: usize,
        /// Sandbox preference: auto, require, or forbid
        #[arg(long, default_value = "auto")]
        sandbox: String,
        /// Working directory for tool execution
        #[arg(short = 'd', long, default_value = ".")]
        working_dir: PathBuf,
        /// Path to config file
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Output format: text or json
        #[arg(long, default_value = "text")]
        output: String,
    },
    /// Print version
    Version,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match (&cli.command, &cli.message) {
        // Subcommand-driven paths
        (
            Some(Commands::Daemon {
                config,
                env,
                socket,
                container,
                image,
                enable_evolution,
            }),
            _,
        ) => {
            init_tracing(
                "aletheon::daemon",
                Some(Path::new("/var/lib/aletheon/aletheon.log")),
            );
            let socket_path = socket.clone().unwrap_or(cli.socket);
            let daemon_mode = detect_daemon_mode(container);

            match daemon_mode {
                DaemonMode::Systemd => {
                    let mut host = executive::host::systemd::SystemdHost::new(
                        config.clone(),
                        env.clone(),
                        socket_path,
                        *enable_evolution,
                    );
                    host.init().await?;
                    Box::new(host).serve().await
                }
                DaemonMode::Container { runtime_name } => {
                    let mut host = executive::host::container::ContainerHost::new(
                        config.clone(),
                        env.clone(),
                        runtime_name,
                        image.clone(),
                        *enable_evolution,
                    );
                    host.init().await?;
                    Box::new(host).serve().await
                }
                DaemonMode::Foreground => {
                    let mut host = executive::host::DaemonHost::new(
                        config.clone(),
                        env.clone(),
                        socket_path,
                        *enable_evolution,
                    );
                    host.init().await?;
                    Box::new(host).serve().await
                }
            }
        }
        (
            Some(Commands::Exec {
                prompt,
                model,
                max_turns,
                sandbox,
                working_dir,
                config,
                output,
            }),
            _,
        ) => {
            init_tracing("aletheon::exec", None);
            run_exec(
                prompt,
                model,
                *max_turns,
                sandbox,
                working_dir,
                config,
                output,
            )
            .await
        }
        (Some(Commands::Version), _) => {
            println!("aletheon {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        // -m flag: single message to daemon
        (None, Some(msg)) => interact::cli::single_message(&cli.socket, msg).await,
        // No subcommand, no -m: TUI mode (interact crate handles arg parsing)
        (None, None) => interact::cli::run().await,
    }
}

// ── Daemon mode detection ──────────────────────────────────────────────────

enum DaemonMode {
    Systemd,
    Container { runtime_name: String },
    Foreground,
}

fn detect_daemon_mode(container_override: &Option<String>) -> DaemonMode {
    if let Some(rt) = container_override {
        return DaemonMode::Container {
            runtime_name: rt.clone(),
        };
    }
    if std::env::var("NOTIFY_SOCKET").is_ok() {
        return DaemonMode::Systemd;
    }
    if std::env::var("CONTAINER").is_ok() || std::path::Path::new("/.dockerenv").exists() {
        return DaemonMode::Container {
            runtime_name: "docker".to_string(),
        };
    }
    DaemonMode::Foreground
}

// ── Tracing ─────────────────────────────────────────────────────────────────

fn init_tracing(target: &str, log_file: Option<&Path>) {
    use std::fs;
    let env_filter = if std::env::var("RUST_LOG").is_ok() {
        EnvFilter::from_default_env()
    } else {
        // Capture info-level logs from aletheon + key runtime subsystems
        EnvFilter::new(format!(
            "{}=info,runtime=info,cognit=info,corpus=info",
            target
        ))
    };

    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer);

    if let Some(path) = log_file {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let log_path = path.to_path_buf();
        // Test that the file is writable before adding the layer
        match fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            Ok(_) => {
                let file_layer = tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(move || {
                        fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&log_path)
                            .expect("failed to open log file for append")
                    });
                subscriber.with(file_layer).init();
                return;
            }
            Err(e) => {
                eprintln!(
                    "Warning: could not open log file {}: {}",
                    log_path.display(),
                    e
                );
            }
        }
    }

    subscriber.init();
}

// ── Exec ────────────────────────────────────────────────────────────────────

use aletheon_kernel::service::ServicePorts;
use cognit::r#impl::provider_registry::ProviderRegistry;
use corpus::security::sandbox::executor::SandboxPreference;
use corpus::security::approval::{ApprovalGate, TerminalApprovalGate};
use corpus::security::audit::AuditLogger;
use corpus::security::runner::ToolRunnerWithGuard;
use corpus::tools::tools::{ToolContext, ToolRegistry};
use fabric::types::admission::RiskLevel;
use fabric::{
    AdmissionController, AdmissionRequest, CapabilityId, CapabilityRequest, CapabilityResult,
    CapabilityScope, LlmProvider, Message, NoopTurnEventSink, OperationId, PrincipalId, ProcessId,
    RecallSet, SandboxRequirement, ToolDefinition, TurnRequest, TurnServices, UsageReport,
};

/// Non-interactive exec logic — mirrors the old aletheon-exec binary.
async fn run_exec(
    prompt: &str,
    model: &str,
    max_turns: usize,
    sandbox: &str,
    working_dir: &Path,
    config: &Option<PathBuf>,
    output: &str,
) -> Result<()> {
    // Load ~/.aletheon/.env so provider API keys resolve.
    if let Some(home) = std::env::var_os("HOME") {
        executive::host::load_dotenv(&PathBuf::from(home).join(".aletheon").join(".env"));
    }

    let working_dir = working_dir
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/tmp")));

    // Load config
    let app_config = if let Some(ref path) = config {
        cognit::config::AppConfig::load_or_default(path)
    } else {
        cognit::config::AppConfig::load_layered(None)
    };

    // Build provider registry
    let registry = ProviderRegistry::from_config(&app_config)?;

    // Create LLM provider
    let llm: Arc<dyn LlmProvider> = Arc::from(registry.resolve_and_create(model)?);
    info!(provider = llm.name(), model = %model, "LLM provider initialized");

    // Create tool registry with default tools
    let tool_registry = Arc::new(ToolRegistry::default());

    // Guarded runner with terminal approval for risky (L2+) tools.
    let audit_path = working_dir.join(".aletheon-audit.jsonl");
    let approval: Arc<dyn ApprovalGate> = Arc::new(TerminalApprovalGate);
    let sandbox_preference = SandboxPreference::from_str(sandbox);
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
        llm,
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

    let harness_config = cognit::harness::HarnessConfig {
        max_iterations: max_turns,
        ..Default::default()
    };
    let turn_service = executive::service::TurnService::new(
        services,
        executive::service::PreTurnPipeline,
        executive::service::PostTurnPipeline,
        ports,
    )
    .with_harness_config(harness_config);

    let result = turn_service
        .submit(
            TurnRequest {
                operation_id: OperationId::new(),
                process_id: ProcessId::new(),
                session_id,
                input: prompt.to_string(),
                working_dir,
                model_policy: if model.is_empty() {
                    None
                } else {
                    Some(model.to_string())
                },
                deadline: None,
            },
            &NoopTurnEventSink,
        )
        .await?;

    let success = result.metrics.completed_normally;
    info!(
        iterations = result.metrics.iterations,
        tool_calls = result.metrics.tool_calls_made,
        tool_errors = result.metrics.tool_errors,
        success = success,
        "Execution complete"
    );

    if output == "json" {
        let response = serde_json::json!({
            "success": success,
            "response": result.output,
            "iterations": result.metrics.iterations,
            "tool_calls_made": result.metrics.tool_calls_made,
            "tool_errors": result.metrics.tool_errors,
            "elapsed_ms": result.metrics.elapsed_ms,
        });
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!("{}", result.output);
    }

    if !success {
        std::process::exit(1);
    }
    Ok(())
}

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
    async fn recall(&self, _req: fabric::RecallRequest) -> anyhow::Result<RecallSet> {
        Ok(RecallSet::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> anyhow::Result<fabric::DaseinView> {
        Ok(fabric::DaseinView::default())
    }

    async fn agora_view(&self, _session_id: &str) -> anyhow::Result<fabric::AgoraView> {
        Ok(fabric::AgoraView::default())
    }

    async fn invoke(&self, req: CapabilityRequest) -> CapabilityResult {
        // PR-2: route all tool invocations through admission controller.
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

        if !permit.is_valid_at(fabric::MonoTime(0)) {
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

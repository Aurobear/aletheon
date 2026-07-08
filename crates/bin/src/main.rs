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
use runtime::host::RuntimeHost;
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
                    let mut host = runtime::host::systemd::SystemdHost::new(
                        config.clone(),
                        env.clone(),
                        socket_path,
                        *enable_evolution,
                    );
                    host.init().await?;
                    Box::new(host).serve().await
                }
                DaemonMode::Container { runtime_name } => {
                    let mut host = runtime::host::container::ContainerHost::new(
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
                    let mut host = runtime::host::DaemonHost::new(
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

use base::{ContentBlock, Message, Role};
use cognit::r#impl::llm::LlmProvider;
use cognit::r#impl::llm::StopReason;
use cognit::r#impl::provider_registry::ProviderRegistry;
use corpus::security::sandbox::executor::SandboxPreference;
use corpus::security::security::approval::{ApprovalGate, TerminalApprovalGate};
use corpus::security::security::audit::AuditLogger;
use corpus::security::security::runner::ToolRunnerWithGuard;
use corpus::tools::tools::{ToolContext, ToolRegistry};

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
        runtime::host::load_dotenv(&PathBuf::from(home).join(".aletheon").join(".env"));
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
    let tool_registry = ToolRegistry::default();

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

    // Build tool definitions for LLM
    let tool_defs = tool_registry.definitions();
    info!(tool_count = tool_defs.len(), "Tools registered");

    // Build system prompt
    let system_prompt = format!(
        "You are Aletheon, an AI agent executing a task non-interactively. \
         You have access to tools. Complete the user's request and provide a final response. \
         Working directory: {}",
        working_dir.display()
    );

    // Initialize conversation
    let mut messages: Vec<Message> = vec![Message::system(&system_prompt), Message::user(prompt)];

    // Tool execution context
    let tool_ctx = ToolContext {
        working_dir: working_dir.clone(),
        session_id: uuid::Uuid::new_v4().to_string(),
    };

    // Agent loop
    let mut turns_used = 0;
    let mut total_input_tokens = 0u32;
    let mut total_output_tokens = 0u32;
    let final_response;

    loop {
        if turns_used >= max_turns {
            tracing::warn!(max_turns, "Max turns reached");
            final_response = format!(
                "Max turns ({}) reached without completing the task.",
                max_turns
            );
            break;
        }

        info!(turn = turns_used + 1, "Starting turn");

        // Call LLM
        let response = llm.complete(&messages, &tool_defs).await?;
        total_input_tokens += response.usage.input_tokens;
        total_output_tokens += response.usage.output_tokens;

        match response.stop_reason {
            StopReason::EndTurn | StopReason::MaxTokens => {
                // LLM is done, extract final text response
                final_response = response
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");

                // Add assistant message to history
                messages.push(Message {
                    role: Role::Assistant,
                    content: response.content.clone(),
                });
                break;
            }
            StopReason::ToolUse => {
                // LLM wants to call tools
                let assistant_content = response.content.clone();
                let mut tool_results = Vec::new();

                for block in &response.content {
                    if let ContentBlock::ToolUse { id, name, input } = block {
                        info!(tool = %name, "Executing tool");
                        let tool_result = if let Some(tool) = tool_registry.get(name) {
                            let result = runner
                                .run(tool.as_ref(), input.clone(), &tool_ctx, &turn_id)
                                .await;
                            if result.is_error {
                                tracing::warn!(tool = %name, error = %result.content, "Tool failed/denied");
                            } else {
                                info!(tool = %name, "Tool succeeded");
                            }
                            ContentBlock::ToolResult {
                                tool_use_id: id.clone(),
                                content: result.content,
                                is_error: result.is_error,
                            }
                        } else {
                            tracing::warn!(tool = %name, "Unknown tool");
                            ContentBlock::ToolResult {
                                tool_use_id: id.clone(),
                                content: format!("Error: Unknown tool '{}'", name),
                                is_error: true,
                            }
                        };
                        tool_results.push(tool_result);
                    }
                }

                // Add assistant message (with tool_use) and tool results to history
                messages.push(Message {
                    role: Role::Assistant,
                    content: assistant_content,
                });
                messages.push(Message {
                    role: Role::User,
                    content: tool_results,
                });

                turns_used += 1;
            }
        }
    }

    let success = turns_used < max_turns;
    info!(
        turns = turns_used,
        input_tokens = total_input_tokens,
        output_tokens = total_output_tokens,
        success = success,
        "Execution complete"
    );

    if output == "json" {
        let result = serde_json::json!({
            "success": success,
            "response": final_response,
            "turns_used": turns_used,
            "total_input_tokens": total_input_tokens,
            "total_output_tokens": total_output_tokens,
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("{}", final_response);
    }

    if !success {
        std::process::exit(1);
    }
    Ok(())
}

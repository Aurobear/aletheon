//! aletheon — unified entry point for Aletheon AI agent.
//!
//! Subcommands:
//!   (none)       TUI client (auto-starts daemon if not running)
//!   daemon       Start daemon (auto-detects systemd/container/foreground)
//!   exec         Non-interactive execution
//!   config       Inspect effective configuration or layers
//!   doctor       Run diagnostics and print a health report
//!   -m `msg`      Send single message to daemon
//!   version      Print version + git commit

use aletheon_bin::workspace::WorkspaceArgs;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
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

    /// Socket path (default: $XDG_RUNTIME_DIR/aletheon/aletheon.sock)
    #[arg(short, long)]
    socket: Option<PathBuf>,

    #[command(flatten)]
    workspace: WorkspaceArgs,

    /// Path to write TUI frame snapshots (test instrumentation)
    #[arg(long, hide = true)]
    record_frames: Option<PathBuf>,

    /// Path to write daemon-to-TUI events (test instrumentation)
    #[arg(long, hide = true)]
    record_events: Option<PathBuf>,

    /// Path containing one TUI input per line (test instrumentation)
    #[arg(long, hide = true)]
    test_input: Option<PathBuf>,

    /// Automatically submit test input lines
    #[arg(long, hide = true)]
    auto_submit: bool,

    /// TUI test timeout in seconds
    #[arg(long, default_value_t = 120, hide = true)]
    test_timeout: u64,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the machine-scoped inference core
    Core {
        /// Path to machine configuration
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Group-authorized internal inference socket
        #[arg(long, default_value = "/run/aletheon/core.sock")]
        socket: PathBuf,
    },
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
        /// Path to config file
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Output format: text or json
        #[arg(long, default_value = "text")]
        output: String,
    },
    /// Print version
    Version,
    /// Restore terminal modes after an interrupted TUI session
    RestoreTerminal,
    /// Inspect effective configuration (merged layers)
    Config {
        #[command(subcommand)]
        sub: ConfigSub,
    },
    /// Run diagnostics and print a health report
    Doctor {
        /// Output as JSON schema-stable report
        #[arg(long)]
        json: bool,
        /// Path to a specific config file to validate
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Project directory for layered config discovery
        #[arg(short = 'd', long)]
        project_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ConfigSub {
    /// Print the fully merged effective configuration (secrets redacted)
    Effective {
        /// Path to a specific config file
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Project directory for layered config discovery
        #[arg(short = 'd', long)]
        project_dir: Option<PathBuf>,
    },
    /// Show each config layer source and its overrides
    Layers {
        /// Path to a specific config file
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Project directory for layered config discovery
        #[arg(short = 'd', long)]
        project_dir: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if matches!(&cli.command, Some(Commands::Core { .. }))
        && (cli.workspace.cwd.is_some() || !cli.workspace.add_dirs.is_empty())
    {
        anyhow::bail!("aletheon core does not accept workspace authority");
    }
    match (&cli.command, &cli.message) {
        // Subcommand-driven paths
        (Some(Commands::Core { config, socket }), _) => {
            init_tracing("aletheon::core");
            executive::host::launcher::run_core(executive::host::launcher::CoreLaunch {
                config: config.clone(),
                socket: socket.clone(),
            })
            .await
        }
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
            init_tracing("aletheon::daemon");
            executive::host::launcher::run_daemon(executive::host::launcher::DaemonLaunch {
                config: config.clone(),
                env: env.clone(),
                command_socket: socket.clone(),
                parent_socket: cli.socket.clone(),
                container: container.clone(),
                image: image.clone(),
                enable_evolution: *enable_evolution,
            })
            .await
        }
        (
            Some(Commands::Exec {
                prompt,
                model,
                max_turns,
                sandbox,
                config,
                output,
            }),
            _,
        ) => {
            init_tracing("aletheon::exec");
            let outcome =
                executive::host::launcher::run_exec(executive::host::launcher::ExecLaunch {
                    prompt: prompt.clone(),
                    model: model.clone(),
                    max_turns: *max_turns,
                    sandbox: sandbox.clone(),
                    workspace: cli.workspace.executive_launch(),
                    config: config.clone(),
                    json: output == "json",
                })
                .await?;
            println!("{}", outcome.rendered);
            if outcome.success {
                Ok(())
            } else {
                Err(anyhow::anyhow!("exec host failed"))
            }
        }
        (Some(Commands::Version), _) => {
            println!("aletheon {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        (Some(Commands::Config { sub }), _) => {
            init_tracing("aletheon::config");
            handle_config(sub).await
        }
        (
            Some(Commands::Doctor {
                json,
                config,
                project_dir,
            }),
            _,
        ) => {
            init_tracing("aletheon::doctor");
            handle_doctor(*json, config.as_deref(), project_dir.as_deref()).await
        }
        (Some(Commands::RestoreTerminal), _) => {
            interact::tui::restore_terminal();
            println!("Terminal restored to normal state.");
            Ok(())
        }
        // -m flag: single message to daemon
        (None, Some(msg)) => {
            interact::host::run_single_message(interact::host::MessageLaunch {
                socket: cli.socket.clone(),
                workspace: cli.workspace.interact_launch(),
                message: msg.clone(),
            })
            .await
        }
        // No subcommand, no -m: TUI mode. The unified binary owns argument
        // parsing, so pass instrumentation through instead of parsing twice.
        (None, None) => {
            let config = interact::tui::TestConfig {
                test_input: cli.test_input,
                record_frames: cli.record_frames,
                record_events: cli.record_events,
                auto_submit: cli.auto_submit,
                test_timeout: cli.test_timeout,
            };
            interact::host::run_tui(
                interact::host::TuiLaunch {
                    socket: cli.socket.clone(),
                    workspace: cli.workspace.interact_launch(),
                },
                config,
            )
            .await
        }
    }
}

// ── Config & Doctor handlers ────────────────────────────────────────────────

async fn handle_config(sub: &ConfigSub) -> Result<()> {
    use executive::core::config;
    match sub {
        ConfigSub::Effective {
            config,
            project_dir,
        } => {
            let loaded = if let Some(path) = config {
                let txt = std::fs::read_to_string(path)?;
                let layer = config::ConfigLayer::from_toml(
                    config::ConfigSource::new(
                        config::ConfigSourceKind::Cli,
                        path.display().to_string(),
                    ),
                    &txt,
                )?;
                config::merge_layers([layer])?
            } else {
                config::diagnostics::load_config_diagnostics(project_dir.as_deref())?
            };
            let view = loaded.effective_view();
            println!("{}", serde_json::to_string_pretty(&view.config)?);
        }
        ConfigSub::Layers {
            config,
            project_dir,
        } => {
            let loaded = if let Some(path) = config {
                let txt = std::fs::read_to_string(path)?;
                let layer = config::ConfigLayer::from_toml(
                    config::ConfigSource::new(
                        config::ConfigSourceKind::Cli,
                        path.display().to_string(),
                    ),
                    &txt,
                )?;
                config::merge_layers([layer])?
            } else {
                config::diagnostics::load_config_diagnostics(project_dir.as_deref())?
            };
            let view = loaded.layers_view();
            println!("{}", serde_json::to_string_pretty(&view)?);
        }
    }
    Ok(())
}

async fn handle_doctor(
    json: bool,
    config_path: Option<&PathBuf>,
    project_dir: Option<&PathBuf>,
) -> Result<()> {
    use executive::core::config;
    use executive::r#impl::doctor::DoctorReport;
    let loaded = if let Some(path) = config_path {
        let txt = std::fs::read_to_string(path)?;
        let layer = config::ConfigLayer::from_toml(
            config::ConfigSource::new(config::ConfigSourceKind::Cli, path.display().to_string()),
            &txt,
        )?;
        config::merge_layers([layer])?
    } else {
        config::diagnostics::load_config_diagnostics(project_dir)?
    };
    let report = DoctorReport::standalone(&loaded);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("aletheon doctor — v{}", report.daemon_version);
        println!("  status:    {}", report.status);
        println!(
            "  config:    {} ({} leaves)",
            report.config.validity, report.config.leaf_count
        );
        println!(
            "  deploy:    sha={} (core_compat={})",
            report.deployment.installed_sha,
            report
                .deployment
                .runtime_versions_compatible
                .map_or("unknown".to_string(), |c| c.to_string())
        );
        println!(
            "  MCP:       {} servers configured",
            report.mcp_servers.len()
        );
        println!("  sandbox:   {}", report.sandbox.status);
        println!("  writer:    {}", report.writer_health.status);
        for warning in &report.warnings {
            println!("  WARNING:   {warning}");
        }
    }
    Ok(())
}

// ── Tracing ─────────────────────────────────────────────────────────────────

fn init_tracing(target: &str) {
    let env_filter = if std::env::var("RUST_LOG").is_ok() {
        EnvFilter::from_default_env()
    } else {
        // Capture info-level logs from aletheon + key runtime subsystems
        EnvFilter::new(format!(
            "{}=info,runtime=info,cognit=info,corpus=info",
            target
        ))
    };

    let stderr_layer = tracing_subscriber::fmt::layer()
        .json()
        .flatten_event(true)
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(std::io::stderr);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer)
        .init();
}

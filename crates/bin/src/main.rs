//! aletheon — unified entry point for Aletheon AI agent.
//!
//! Subcommands:
//!   (none)       TUI client (auto-starts daemon if not running)
//!   daemon       Start daemon (auto-detects systemd/container/foreground)
//!   exec         Non-interactive execution
//!   -m `msg`      Send single message to daemon
//!   version      Print version + git commit

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

    /// Socket path (default: /run/aletheon/aletheon.sock)
    #[arg(short, long, default_value = "/run/aletheon/aletheon.sock")]
    socket: PathBuf,

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
    /// Restore terminal modes after an interrupted TUI session
    RestoreTerminal,
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
            init_tracing("aletheon::daemon");
            executive::host::launcher::run_daemon(executive::host::launcher::DaemonLaunch {
                config: config.clone(),
                env: env.clone(),
                socket: socket.clone().unwrap_or(cli.socket),
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
                working_dir,
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
                    working_dir: working_dir.clone(),
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
        (Some(Commands::RestoreTerminal), _) => {
            interact::tui::restore_terminal();
            println!("Terminal restored to normal state.");
            Ok(())
        }
        // -m flag: single message to daemon
        (None, Some(msg)) => interact::cli::single_message(&cli.socket, msg).await,
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
            interact::tui::run_with_config(cli.socket.to_string_lossy().as_ref(), config).await
        }
    }
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

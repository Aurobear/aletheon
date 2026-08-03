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

use aletheon::workspace::WorkspaceArgs;
use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use interact::cli::TaskKindArg;
use std::path::PathBuf;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

#[cfg(feature = "acp")]
mod acp;
mod extension_cli;
mod memory_agent;
mod memory_cli;

#[derive(Parser)]
#[command(name = "aletheon", about = "AI agent with sandbox, multi-agent, IPC")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Permission profile: safe, dev, or full.
    #[arg(
        short = 'P',
        long = "permission-mode",
        global = true,
        value_enum,
        default_value = "safe"
    )]
    permission_mode: PermissionModeArg,

    /// Shortcut for `-P full`.
    #[arg(long, global = true, conflicts_with = "permission_mode")]
    full: bool,

    /// Run the feature-gated ACP stdio gateway.
    #[cfg(feature = "acp")]
    #[arg(long)]
    acp: bool,

    /// Send a single message to the daemon
    #[arg(short = 'm', long = "message", value_name = "MSG")]
    message: Option<String>,

    /// Socket path (default: $XDG_RUNTIME_DIR/aletheon/aletheon.sock)
    #[arg(short, long)]
    socket: Option<PathBuf>,

    /// Require this Agent runtime to be spawned and reach an authoritative
    /// terminal receipt during every submitted chat turn (repeatable).
    #[arg(long = "require-agent-runtime", value_name = "RUNTIME")]
    required_agent_runtimes: Vec<String>,

    /// Explicitly classify submitted chat turns for host-owned evaluation.
    #[arg(long = "task-kind", value_enum)]
    task_kind: Option<TaskKindArg>,

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
enum PermissionModeArg {
    #[default]
    #[value(alias = "restricted")]
    Safe,
    #[value(alias = "developer")]
    Dev,
    #[value(alias = "unrestricted")]
    Full,
}

impl PermissionModeArg {
    fn effective(self, full: bool) -> &'static str {
        if full {
            return "full";
        }
        match self {
            Self::Safe => "safe",
            Self::Dev => "dev",
            Self::Full => "full",
        }
    }
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
        /// Enable the isolated execd tool backend (OFF by default)
        #[arg(long = "execd", default_value_t = false)]
        execd: bool,
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
    /// Manage extension packages.
    Extension {
        #[command(subcommand)]
        sub: extension_cli::ExtensionCmd,
    },
    /// Run the Aletheon-managed memory maintenance client.
    MemoryAgent {
        #[command(subcommand)]
        sub: MemoryAgentCommand,
    },
    /// Use the governed Memory Gateway through the official user socket.
    Memory {
        #[command(subcommand)]
        sub: MemoryCommand,
    },
}

#[derive(Subcommand)]
enum MemoryAgentCommand {
    /// Continuously maintain memory through the official user socket.
    Serve {
        /// Required marker for the supervised, authenticated service form.
        #[arg(long, required = true)]
        official_user_socket: bool,
    },
    /// Run one bounded maintenance cycle through an authenticated child.
    Run {
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=64))]
        max_items: u16,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum MemoryCommand {
    /// Submit a bounded observation to the governed intake journal.
    Observe {
        /// Stable client idempotency key; generated when omitted.
        #[arg(long)]
        observation_id: Option<String>,
        #[arg(long, default_value = ".")]
        working_dir: PathBuf,
        #[arg(long, default_value = "explicit-note")]
        kind: MemoryObservationKindArg,
        /// Content value; when omitted, UTF-8 content is read from stdin.
        #[arg(long)]
        content: Option<String>,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        turn_id: Option<String>,
        #[arg(long)]
        explicit_user_action: bool,
        #[arg(long, default_value = "internal")]
        sensitivity: MemorySensitivityArg,
        #[arg(long = "source-ref")]
        source_refs: Vec<String>,
    },
    /// Recall governed local and bound supplemental memory.
    Recall {
        query: String,
        #[arg(long, default_value = ".")]
        working_dir: PathBuf,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long, default_value_t = 20)]
        max_items: usize,
        #[arg(long, default_value_t = 65536)]
        max_content_bytes: usize,
        #[arg(long)]
        include_historical: bool,
    },
    /// Read the authoritative lifecycle receipt for an observation.
    Receipt { durable_intake_id: String },
    /// Administer the current workspace's supplemental-memory binding.
    Workspace {
        #[command(subcommand)]
        sub: MemoryWorkspaceCommand,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum MemoryObservationKindArg {
    UserMessage,
    AssistantMessage,
    ToolOutcome,
    TaskOutcome,
    ExplicitNote,
    Correction,
    Feedback,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum MemorySensitivityArg {
    Public,
    Internal,
    Confidential,
    Restricted,
}

#[derive(clap::Args)]
struct MemoryBindingArgs {
    #[arg(long, default_value = ".")]
    working_dir: PathBuf,
    #[arg(long, default_value = "supplemental/gbrain")]
    backend: String,
    #[arg(long, default_value = "gbrain")]
    write_handle: String,
    #[arg(long = "read-handle")]
    read_handles: Vec<String>,
    #[arg(long)]
    write_source: String,
    #[arg(long = "read-source", required = true)]
    read_sources: Vec<String>,
    #[arg(long)]
    credential_ref: Option<String>,
}

#[derive(Subcommand)]
enum MemoryWorkspaceCommand {
    /// Negotiate backend grants without changing durable authority.
    PreviewBind {
        #[command(flatten)]
        binding: MemoryBindingArgs,
    },
    /// Repeat negotiation and activate a previewed binding.
    Bind {
        #[command(flatten)]
        binding: MemoryBindingArgs,
        #[arg(long)]
        expected_capability_digest: String,
    },
    /// Revoke remote authority for this workspace.
    Unbind {
        #[arg(long, default_value = ".")]
        working_dir: PathBuf,
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
    let permission_mode = cli.permission_mode.effective(cli.full);
    std::env::set_var("ALETHEON_PERMISSION_MODE", permission_mode);
    #[cfg(feature = "acp")]
    if cli.acp {
        anyhow::ensure!(
            cli.command.is_none() && cli.message.is_none(),
            "--acp cannot be combined with a subcommand or --message"
        );
        init_tracing("aletheon::acp");
        return acp::run(cli.workspace.executive_launch()).await;
    }
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
                execd,
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
                enable_execd: *execd,
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
        (Some(Commands::Extension { sub }), _) => {
            init_tracing("aletheon::extension");
            extension_cli::run(sub, cli.socket.clone()).await
        }
        (Some(Commands::MemoryAgent { sub }), _) => {
            init_tracing("aletheon::memory_agent");
            match sub {
                MemoryAgentCommand::Serve {
                    official_user_socket,
                } => {
                    anyhow::ensure!(*official_user_socket, "--official-user-socket is required");
                    memory_agent::serve_official_user_socket().await
                }
                MemoryAgentCommand::Run { max_items, dry_run } => {
                    memory_agent::run_once(*max_items, *dry_run).await
                }
            }
        }
        (Some(Commands::Memory { sub }), _) => {
            init_tracing("aletheon::memory");
            memory_cli::run(sub, cli.socket.clone()).await
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
                required_agent_runtimes: cli.required_agent_runtimes.clone(),
                task_kind: cli.task_kind.map(Into::into),
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
                    required_agent_runtimes: cli.required_agent_runtimes.clone(),
                    task_kind: cli.task_kind.map(Into::into),
                },
                config,
            )
            .await
        }
    }
}

// ── Config & Doctor handlers ────────────────────────────────────────────────

async fn handle_config(sub: &ConfigSub) -> Result<()> {
    use executive::composition::config;
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
    config_path: Option<&std::path::Path>,
    project_dir: Option<&std::path::Path>,
) -> Result<()> {
    use executive::composition::config;
    use executive::host::doctor::DoctorReport;
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
        println!(
            "  recovery:  {} sessions / {} turns / {} recovered",
            report.turn_recovery.sessions_scanned,
            report.turn_recovery.turns_scanned,
            report.turn_recovery.incomplete_turns_recovered
        );
        println!(
            "  profiles:  {} quarantined{}",
            report.quarantined_profiles.count,
            if report.quarantined_profiles.names.is_empty() {
                String::new()
            } else {
                format!(" ({})", report.quarantined_profiles.names.join(", "))
            }
        );
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
            "{target}=info,runtime=info,cognit=info,corpus=info"
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

#[cfg(all(test, feature = "acp"))]
mod acp_cli_tests {
    use super::*;

    #[test]
    fn acp_flag_is_exposed_only_in_acp_feature_build() {
        let cli = Cli::try_parse_from(["aletheon", "--acp"]).unwrap();
        assert!(cli.acp);
    }
}

#[cfg(test)]
mod daemon_cli_tests {
    use super::*;

    #[test]
    fn execd_flag_defaults_off_and_enables_additively() {
        let default_cli = Cli::try_parse_from(["aletheon", "daemon"]).unwrap();
        assert!(matches!(
            default_cli.command,
            Some(Commands::Daemon { execd: false, .. })
        ));

        let enabled_cli = Cli::try_parse_from(["aletheon", "daemon", "--execd"]).unwrap();
        assert!(matches!(
            enabled_cli.command,
            Some(Commands::Daemon { execd: true, .. })
        ));
    }

    #[test]
    fn required_agent_runtime_is_repeatable_on_the_installed_entrypoint() {
        let cli = Cli::try_parse_from([
            "aletheon",
            "--require-agent-runtime",
            "pi-rpc",
            "--require-agent-runtime",
            "native-cognit",
        ])
        .unwrap();
        assert_eq!(cli.required_agent_runtimes, vec!["pi-rpc", "native-cognit"]);
    }

    #[test]
    fn parses_coding_task_kind_for_message_and_tui() {
        let message =
            Cli::try_parse_from(["aletheon", "--task-kind", "coding", "--message", "hello"])
                .unwrap();
        assert_eq!(message.task_kind, Some(TaskKindArg::Coding));

        let tui = Cli::try_parse_from(["aletheon", "--task-kind", "coding"]).unwrap();
        assert_eq!(tui.task_kind, Some(TaskKindArg::Coding));
    }

    #[test]
    fn parses_short_and_compatible_permission_modes() {
        let short = Cli::try_parse_from(["aletheon", "-P", "full"]).unwrap();
        assert_eq!(short.permission_mode.effective(short.full), "full");

        let compatible =
            Cli::try_parse_from(["aletheon", "--permission-mode", "unrestricted"]).unwrap();
        assert_eq!(
            compatible.permission_mode.effective(compatible.full),
            "full"
        );
    }

    #[test]
    fn full_flag_is_a_shortcut_for_unrestricted_permissions() {
        let cli = Cli::try_parse_from(["aletheon", "--full"]).unwrap();
        assert_eq!(cli.permission_mode.effective(cli.full), "full");
    }

    #[test]
    fn governed_memory_client_commands_parse_on_installed_entrypoint() {
        let observe = Cli::try_parse_from([
            "aletheon",
            "memory",
            "observe",
            "--kind",
            "task-outcome",
            "--content",
            "bounded result",
            "--session-id",
            "session-a",
        ])
        .unwrap();
        assert!(matches!(
            observe.command,
            Some(Commands::Memory {
                sub: MemoryCommand::Observe { .. }
            })
        ));

        let recall = Cli::try_parse_from([
            "aletheon",
            "memory",
            "recall",
            "workspace architecture",
            "--max-items",
            "8",
        ])
        .unwrap();
        assert!(matches!(
            recall.command,
            Some(Commands::Memory {
                sub: MemoryCommand::Recall { max_items: 8, .. }
            })
        ));
    }
}

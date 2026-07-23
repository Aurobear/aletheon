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
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

#[cfg(feature = "acp")]
mod acp;

#[derive(Parser)]
#[command(name = "aletheon", about = "AI agent with sandbox, multi-agent, IPC")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

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
        sub: ExtensionCmd,
    },
}

#[derive(Subcommand)]
enum ExtensionCmd {
    /// Inspect an extension package archive.
    Inspect { path: PathBuf },
    /// Validate an extension package without installing.
    Validate { path: PathBuf },
    /// Install an extension package.
    Install {
        path: PathBuf,
        /// Explicitly trust an archive located under `.aletheon/extensions`.
        #[arg(long)]
        trust_workspace: bool,
    },
    /// List installed extensions.
    List,
    /// Show details for an installed extension.
    Show { id: String },
    /// Enable an installed extension.
    Enable {
        id: String,
        /// Explicitly approve newly requested assets and permissions.
        #[arg(long)]
        approve_permissions: bool,
    },
    /// Disable an active extension.
    Disable { id: String },
    /// Upgrade an extension to a newer package.
    Upgrade {
        /// Path to the new package archive.
        path: PathBuf,
        /// Explicitly approve permission or capability additions.
        #[arg(long)]
        approve_permissions: bool,
        /// Explicitly trust an archive located under `.aletheon/extensions`.
        #[arg(long)]
        trust_workspace: bool,
    },
    /// Rollback to the previous known-good version.
    Rollback { id: String },
    /// Remove an extension (deactivate but keep package).
    Remove { id: String },
    /// Purge an extension (remove package and all state).
    Purge { id: String },
    /// Run diagnostics on an extension.
    Doctor { id: String },
    /// Import legacy filesystem extensions into the package store.
    ImportLegacy,
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

// ── Extension handler ────────────────────────────────────────────────────

async fn handle_extension(cmd: &ExtensionCmd) -> anyhow::Result<()> {
    use executive::application::extension_install::ExtensionInstallService;
    use executive::application::extension_manage::ExtensionManageService;
    let store_root = corpus::extension::store::PackageStore::configured_user_root();
    let install_svc = ExtensionInstallService::new(&store_root)?;
    let explicit_approval = matches!(
        cmd,
        ExtensionCmd::Enable {
            approve_permissions: true,
            ..
        } | ExtensionCmd::Upgrade {
            approve_permissions: true,
            ..
        }
    );
    let manage_svc = if explicit_approval {
        let actor = std::env::var("USER").unwrap_or_else(|_| "local-operator".into());
        ExtensionManageService::new(&store_root)?.with_approval_port(std::sync::Arc::new(
            executive::application::extension_manage::ExplicitOperatorApproval::new(actor),
        ))
    } else {
        ExtensionManageService::new(&store_root)?
    };

    match cmd {
        ExtensionCmd::Inspect { path } => {
            let result = install_svc.inspect(path)?;
            println!("Package: {}", result.manifest.package.id.0);
            println!("Version: {}", result.manifest.package.version.0);
            println!("Hash: {}", result.package_hash);
            println!("Files: {}", result.file_count);
            println!("Total size: {} bytes", result.total_size);
            println!("Assets:");
            for asset in &result.manifest.assets {
                println!("  - {} ({})", asset.id, serde_json::to_string(&asset.kind)?);
            }
        }
        ExtensionCmd::Validate { path } => {
            install_svc.inspect(path)?;
            println!("Package is valid.");
        }
        ExtensionCmd::Install {
            path,
            trust_workspace,
        } => {
            let actor = trust_workspace
                .then(|| std::env::var("USER").unwrap_or_else(|_| "local-operator".into()));
            let hash = install_svc.install_with_workspace_trust(path, actor.as_deref())?;
            println!("Installed package with hash: {hash}");
        }
        ExtensionCmd::List => {
            let packages = install_svc.list()?;
            if packages.is_empty() {
                println!("No extensions installed.");
            } else {
                for pkg in packages {
                    println!("{}\t{}\t{}", pkg.id, pkg.version, pkg.hash);
                }
            }
        }
        ExtensionCmd::Show { id } => {
            println!("{}", serde_json::to_string_pretty(&install_svc.show(id)?)?);
        }
        ExtensionCmd::Enable { id, .. } => {
            manage_svc.enable(id)?;
            println!("Extension '{id}' enabled.");
        }
        ExtensionCmd::Disable { id } => {
            manage_svc.disable(id)?;
            println!("Extension '{id}' disabled.");
        }
        ExtensionCmd::Upgrade {
            path,
            trust_workspace,
            ..
        } => {
            let actor = trust_workspace
                .then(|| std::env::var("USER").unwrap_or_else(|_| "local-operator".into()));
            manage_svc.upgrade_with_workspace_trust(path, actor.as_deref())?;
            println!("Extension upgraded from '{}'.", path.display());
        }
        ExtensionCmd::Rollback { id } => {
            manage_svc.rollback(id)?;
            println!("Extension '{id}' rolled back.");
        }
        ExtensionCmd::Remove { id } => {
            manage_svc.remove(id)?;
            println!("Extension '{id}' removed.");
        }
        ExtensionCmd::Purge { id } => {
            manage_svc.purge(id)?;
            println!("Extension '{id}' purged.");
        }
        ExtensionCmd::Doctor { id } => {
            let result = manage_svc.doctor(id)?;
            println!(
                "Extension '{}': healthy={}, issues={:?}, legacy_reads={:?}, remaining_legacy={}",
                result.id,
                result.healthy,
                result.issues,
                result.migration_report.compatibility_reads,
                result.migration_report.remaining_candidates,
            );
        }
        ExtensionCmd::ImportLegacy => {
            let imported = manage_svc.import_legacy(&store_root.join("legacy"))?;
            println!(
                "Imported {} legacy extensions: {imported:?}",
                imported.len()
            );
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
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
        (Some(Commands::Extension { sub }), _) => handle_extension(sub).await,
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
}

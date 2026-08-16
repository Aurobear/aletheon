//! Extension package CLI. Only package inspection is local; authoritative
//! lifecycle and runtime state are owned by the installed user daemon.

use std::path::PathBuf;

use ::contracts::paths::{ProcessRuntimeEnvironment, RuntimeEnvironment, UserRuntimePaths};
use clap::Subcommand;
use gateway::client::{CommandOutcome, GatewayClient, UnixSocketTransport};
use gateway::protocol::{Command, ExtensionRequest};

#[derive(Subcommand)]
pub(crate) enum ExtensionCmd {
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
    /// Purge an inactive extension and its stored package state.
    Purge { id: String },
    /// Run diagnostics on an extension.
    Doctor { id: String },
}

pub(crate) async fn run(
    command: &ExtensionCmd,
    explicit_socket: Option<PathBuf>,
) -> anyhow::Result<()> {
    match command {
        ExtensionCmd::Inspect { path } => inspect(path, false),
        ExtensionCmd::Validate { path } => inspect(path, true),
        _ => {
            let request = request(command)?;
            let mut client = ExtensionRpcClient::connect(explicit_socket).await?;
            let value = client.request(Command::ManageExtension(request)).await?;
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
    }
}

fn inspect(path: &std::path::Path, validate_only: bool) -> anyhow::Result<()> {
    let result = aletheon::extension::inspect_archive(path)?;
    if validate_only {
        println!("Package is valid.");
        return Ok(());
    }
    println!("Package: {}", result.manifest.package.id.0);
    println!("Version: {}", result.manifest.package.version.0);
    println!("Hash: {}", result.package_hash);
    println!("Files: {}", result.file_count);
    println!("Total size: {} bytes", result.total_size);
    println!("Assets:");
    for asset in &result.manifest.assets {
        println!("  - {} ({})", asset.id, serde_json::to_string(&asset.kind)?);
    }
    Ok(())
}

fn request(command: &ExtensionCmd) -> anyhow::Result<ExtensionRequest> {
    Ok(match command {
        ExtensionCmd::Install {
            path,
            trust_workspace,
        } => ExtensionRequest::Install {
            path: canonical_package_path(path)?.to_string_lossy().into_owned(),
            trust_workspace: *trust_workspace,
            approve_permissions: false,
        },
        ExtensionCmd::List => ExtensionRequest::List,
        ExtensionCmd::Show { id } => ExtensionRequest::Show { id: id.clone() },
        ExtensionCmd::Enable {
            id,
            approve_permissions,
        } => ExtensionRequest::Enable {
            id: id.clone(),
            approve_permissions: *approve_permissions,
        },
        ExtensionCmd::Disable { id } => ExtensionRequest::Disable { id: id.clone() },
        ExtensionCmd::Upgrade {
            path,
            approve_permissions,
            trust_workspace,
        } => ExtensionRequest::Upgrade {
            path: canonical_package_path(path)?.to_string_lossy().into_owned(),
            trust_workspace: *trust_workspace,
            approve_permissions: *approve_permissions,
        },
        ExtensionCmd::Rollback { id } => ExtensionRequest::Rollback { id: id.clone() },
        ExtensionCmd::Remove { id } => ExtensionRequest::Remove { id: id.clone() },
        ExtensionCmd::Purge { id } => ExtensionRequest::Purge { id: id.clone() },
        ExtensionCmd::Doctor { id } => ExtensionRequest::Doctor { id: id.clone() },
        ExtensionCmd::Inspect { .. } | ExtensionCmd::Validate { .. } => {
            anyhow::bail!("offline package inspection does not create an RPC request")
        }
    })
}

fn canonical_package_path(path: &std::path::Path) -> anyhow::Result<PathBuf> {
    std::fs::canonicalize(path)
        .map_err(|error| anyhow::anyhow!("resolving package {}: {error}", path.display()))
}

struct ExtensionRpcClient {
    client: GatewayClient<UnixSocketTransport>,
}

impl ExtensionRpcClient {
    async fn connect(explicit_socket: Option<PathBuf>) -> anyhow::Result<Self> {
        let socket = resolve_socket(explicit_socket)?;
        let transport = UnixSocketTransport::connect(&socket)
            .await
            .map_err(|error| anyhow::anyhow!("connecting {}: {error}", socket.display()))?;
        Ok(Self {
            client: GatewayClient::new(transport),
        })
    }

    async fn request(&mut self, request: Command) -> anyhow::Result<serde_json::Value> {
        match self
            .client
            .send(request)
            .await
            .map_err(|error| anyhow::anyhow!("daemon extension request failed: {error}"))?
        {
            CommandOutcome::ExtensionResult { result } => Ok(result),
            other => anyhow::bail!("daemon extension returned unexpected receipt: {other:?}"),
        }
    }
}

fn resolve_socket(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(socket) = explicit {
        return Ok(socket);
    }
    let environment = ProcessRuntimeEnvironment;
    if let Some(socket) = environment
        .var_os("ALETHEON_SOCKET")
        .filter(|value| !value.is_empty())
    {
        return Ok(PathBuf::from(socket));
    }
    Ok(UserRuntimePaths::resolve(&environment)?.socket_path())
}

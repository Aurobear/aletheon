//! Extension package CLI. Only package inspection is local; authoritative
//! lifecycle and runtime state are owned by the installed user daemon.

use std::path::PathBuf;

use clap::Subcommand;
use fabric::paths::{ProcessRuntimeEnvironment, RuntimeEnvironment, UserRuntimePaths};
use fabric::protocol::client::{
    ClientCapabilities, ClientRequest, ClientRpcRequest, InitializeParams, CLIENT_PROTOCOL_VERSION,
};
use fabric::protocol::extension::{
    ExtensionEnableRequestV1, ExtensionPackageIdRequestV1, ExtensionPackagePathRequestV1,
    EXTENSION_PROTOCOL_SCHEMA_V1,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufStream};
use tokio::net::UnixStream;

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
            let value = client.request(request).await?;
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
    }
}

fn inspect(path: &std::path::Path, validate_only: bool) -> anyhow::Result<()> {
    let result =
        executive::application::extension_install::ExtensionInstallService::inspect_archive(path)?;
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

fn request(command: &ExtensionCmd) -> anyhow::Result<ClientRpcRequest> {
    let schema_version = EXTENSION_PROTOCOL_SCHEMA_V1;
    Ok(match command {
        ExtensionCmd::Install {
            path,
            trust_workspace,
        } => ClientRpcRequest::ExtensionInstall(ExtensionPackagePathRequestV1 {
            schema_version,
            path: canonical_package_path(path)?,
            trust_workspace: *trust_workspace,
            approve_permissions: false,
        }),
        ExtensionCmd::List => ClientRpcRequest::ExtensionList,
        ExtensionCmd::Show { id } => ClientRpcRequest::ExtensionShow(package_id(id)),
        ExtensionCmd::Enable {
            id,
            approve_permissions,
        } => ClientRpcRequest::ExtensionEnable(ExtensionEnableRequestV1 {
            schema_version,
            package_id: id.clone(),
            approve_permissions: *approve_permissions,
        }),
        ExtensionCmd::Disable { id } => ClientRpcRequest::ExtensionDisable(package_id(id)),
        ExtensionCmd::Upgrade {
            path,
            approve_permissions,
            trust_workspace,
        } => ClientRpcRequest::ExtensionUpgrade(ExtensionPackagePathRequestV1 {
            schema_version,
            path: canonical_package_path(path)?,
            trust_workspace: *trust_workspace,
            approve_permissions: *approve_permissions,
        }),
        ExtensionCmd::Rollback { id } => ClientRpcRequest::ExtensionRollback(package_id(id)),
        ExtensionCmd::Remove { id } => ClientRpcRequest::ExtensionRemove(package_id(id)),
        ExtensionCmd::Purge { id } => ClientRpcRequest::ExtensionPurge(package_id(id)),
        ExtensionCmd::Doctor { id } => ClientRpcRequest::ExtensionDoctor(package_id(id)),
        ExtensionCmd::Inspect { .. } | ExtensionCmd::Validate { .. } => {
            anyhow::bail!("offline package inspection does not create an RPC request")
        }
    })
}

fn package_id(id: &str) -> ExtensionPackageIdRequestV1 {
    ExtensionPackageIdRequestV1 {
        schema_version: EXTENSION_PROTOCOL_SCHEMA_V1,
        package_id: id.to_owned(),
    }
}

fn canonical_package_path(path: &std::path::Path) -> anyhow::Result<PathBuf> {
    std::fs::canonicalize(path)
        .map_err(|error| anyhow::anyhow!("resolving package {}: {error}", path.display()))
}

struct ExtensionRpcClient {
    stream: BufStream<UnixStream>,
    next_id: u64,
}

impl ExtensionRpcClient {
    async fn connect(explicit_socket: Option<PathBuf>) -> anyhow::Result<Self> {
        let socket = resolve_socket(explicit_socket)?;
        let stream = UnixStream::connect(&socket)
            .await
            .map_err(|error| anyhow::anyhow!("connecting {}: {error}", socket.display()))?;
        let mut client = Self {
            stream: BufStream::new(stream),
            next_id: 1,
        };
        client
            .versioned_round_trip(ClientRequest::Initialize(InitializeParams {
                client_version: env!("CARGO_PKG_VERSION").into(),
                protocol_versions: vec![CLIENT_PROTOCOL_VERSION],
                capabilities: ClientCapabilities {
                    item_events: false,
                    cursors: false,
                    memory_gateway_v1: false,
                    memory_maintenance_v1: false,
                    memory_admin_v1: false,
                },
            }))
            .await?;
        client
            .versioned_round_trip(ClientRequest::Initialized)
            .await?;
        Ok(client)
    }

    async fn request(&mut self, request: ClientRpcRequest) -> anyhow::Result<serde_json::Value> {
        let id = self.allocate_id();
        let response = self
            .round_trip_value(id, request.to_json_rpc(Some(id))?)
            .await?;
        response
            .get("result")
            .cloned()
            .ok_or_else(|| response_error(&response))
    }

    async fn versioned_round_trip(
        &mut self,
        request: ClientRequest,
    ) -> anyhow::Result<serde_json::Value> {
        let id = self.allocate_id();
        self.round_trip_value(id, request.to_json_rpc(id)?).await
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    async fn round_trip_value(
        &mut self,
        id: u64,
        value: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        self.stream.write_all(value.to_string().as_bytes()).await?;
        self.stream.write_all(b"\n").await?;
        self.stream.flush().await?;
        for _ in 0..32 {
            let mut line = String::new();
            anyhow::ensure!(
                self.stream.read_line(&mut line).await? > 0,
                "daemon closed the extension control connection"
            );
            let response: serde_json::Value = serde_json::from_str(line.trim())?;
            if response.get("id").and_then(serde_json::Value::as_u64) == Some(id) {
                if response.get("error").is_some() {
                    return Err(response_error(&response));
                }
                return Ok(response);
            }
        }
        anyhow::bail!("too many unrelated daemon messages on extension control connection")
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

fn response_error(response: &serde_json::Value) -> anyhow::Error {
    anyhow::anyhow!(
        "daemon extension request failed: {}",
        response["error"]["message"]
            .as_str()
            .unwrap_or("missing result")
    )
}

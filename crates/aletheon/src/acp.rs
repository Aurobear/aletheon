//! ACP executable composition root.
//!
//! ACP is a presentation adapter. It talks to the already-running official
//! user daemon through the typed Gateway client; it does not bootstrap a
//! second Aletheon/Runtime graph or open a session store of its own.

use std::path::Path;

use ::contracts::{
    ApprovalPolicy, ConnectionId, LocalOsPrincipal, PermissionProfileId, ThreadId, WorkspacePolicy,
};
use anyhow::{Context, Result};
use interact::acp::{
    run_transport_loop, AcpAdapter, AuthenticatedAcpConnection, GatewayAcpBackend,
};
use interact::host::WorkspaceLaunch;
use tokio::io::BufReader;

pub async fn run(workspace: WorkspaceLaunch) -> Result<()> {
    let policy = resolve_workspace(workspace)?;
    let socket = crate::ensure_user_socket(None).await?;
    let os_principal = authenticated_process_principal()?;
    let connection_id = ConnectionId::new();
    let principal = interact::acp::establish_principal(
        os_principal,
        connection_id,
        ThreadId("acp-stdio".into()),
        policy,
        PermissionProfileId::workspace_write(),
        ApprovalPolicy::OnRequest,
    );
    let authenticated = AuthenticatedAcpConnection::new(principal);
    let (backend, mut events) = GatewayAcpBackend::connect(&socket).await?;
    let mut adapter = AcpAdapter::default();

    tracing::info!(
        event = "acp.gateway.started",
        socket = %socket.display(),
        uid = os_principal.uid,
        "ACP stdio Gateway client started"
    );
    let stdin = BufReader::new(tokio::io::stdin());
    let stdout = tokio::io::stdout();
    let mut transport = interact::acp::transport::AcpTransport::new(stdin, stdout);
    run_transport_loop(
        &mut adapter,
        &authenticated,
        &backend,
        &mut events,
        &mut transport,
    )
    .await
    .context("ACP stdio transport")
}

fn authenticated_process_principal() -> Result<LocalOsPrincipal> {
    // In stdio mode there is no peer socket. Kernel process credentials are the
    // only authenticated identity source; there is deliberately no env/client fallback.
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    anyhow::ensure!(
        uid != u32::MAX && gid != u32::MAX,
        "kernel credentials unavailable"
    );
    Ok(LocalOsPrincipal { uid, gid })
}

fn resolve_workspace(workspace: WorkspaceLaunch) -> Result<WorkspacePolicy> {
    let cwd = std::fs::canonicalize(workspace.cwd.unwrap_or(std::env::current_dir()?))?;
    anyhow::ensure!(
        cwd.is_dir() && cwd != Path::new("/"),
        "ACP workspace must be a non-root directory"
    );
    let extra = workspace
        .add_dirs
        .into_iter()
        .map(std::fs::canonicalize)
        .collect::<std::io::Result<Vec<_>>>()?;
    WorkspacePolicy::from_resolved_roots(cwd, extra).map_err(anyhow::Error::msg)
}

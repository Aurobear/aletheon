use std::ffi::CString;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use aletheon_kernel::chronos::Timer;
use anyhow::Result;
use fabric::debug::DebugEvent;
use fabric::Clock;
use nix::unistd::{Gid, Uid, User};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::handler::RequestHandler;

pub struct UnixServer {
    listener: UnixListener,
    handler: RequestHandler,
    cancel_token: CancellationToken,
    /// Tracks spawned connection tasks for graceful shutdown drain.
    connections: JoinSet<()>,
    /// UID of the daemon process — allowed to connect.
    owner_uid: u32,
    /// GID of the aletheon group — users in this group may also connect.
    group_gid: u32,
    clock: Arc<dyn Clock>,
}

impl UnixServer {
    pub async fn new(
        socket_path: &Path,
        handler: RequestHandler,
        cancel_token: CancellationToken,
        owner_uid: u32,
        group_gid: u32,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        // Remove stale socket
        if socket_path.exists() {
            tokio::fs::remove_file(socket_path).await?;
        }

        let listener = UnixListener::bind(socket_path)?;
        // Restrict socket to owner and group only (rw-rw----).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o660))?;
        }
        info!(path = %socket_path.display(), owner_uid, group_gid, "Unix socket listening");

        Ok(Self {
            listener,
            handler,
            cancel_token,
            connections: JoinSet::new(),
            owner_uid,
            group_gid,
            clock,
        })
    }

    /// Return a reference to the handler so the host can interact with it
    /// after the accept loop finishes (e.g., for graceful shutdown).
    pub fn handler(&self) -> &RequestHandler {
        &self.handler
    }

    pub async fn run(&mut self) -> Result<()> {
        loop {
            tokio::select! {
                accept_result = self.listener.accept() => {
                    let (stream, _addr) = accept_result?;
                    // Verify peer credentials before accepting the connection.
                    if let Err(e) = Self::check_peer_cred(&stream, self.owner_uid, self.group_gid) {
                        warn!(error = %e, "Connection rejected by peer credential check");
                        continue;
                    }
                    let mut handler = self.handler.clone();

                    // Create a per-connection notify channel so each client receives
                    // its own events independently (shared channels would cause events
                    // to be consumed by whichever connection reads first).
                    let (notify_tx, notify_rx) = mpsc::channel::<String>(64);
                    handler.set_notify_channel(notify_tx);
                    handler.increment_connections();

                    self.connections.spawn(async move {
                        if let Err(e) = Self::handle_connection(stream, handler, notify_rx).await {
                            error!(error = %e, "Connection error");
                        }
                    });
                }
                _ = self.cancel_token.cancelled() => {
                    info!("Shutdown signal received, stopping accept loop");
                    break;
                }
            }
        }

        // Drain in-flight connections with a 5-second timeout per task.
        info!(
            remaining = self.connections.len(),
            "Draining in-flight connections..."
        );
        loop {
            match Timer::timeout(
                &*self.clock,
                Duration::from_secs(5),
                self.connections.join_next(),
            )
            .await
            {
                Ok(Some(Ok(()))) => {
                    // Connection completed normally.
                }
                Ok(Some(Err(e))) => {
                    error!(error = %e, "Connection task panicked during drain");
                }
                Ok(None) => {
                    info!("All connections drained");
                    break;
                }
                Err(_elapsed) => {
                    info!(
                        remaining = self.connections.len(),
                        "Drain timeout expired, aborting remaining connections"
                    );
                    self.connections.abort_all();
                    break;
                }
            }
        }

        Ok(())
    }

    /// Verify that the connecting peer is either the daemon owner or a member
    /// of the aletheon group. Root (uid 0) is always allowed.
    fn check_peer_cred(
        stream: &tokio::net::UnixStream,
        owner_uid: u32,
        group_gid: u32,
    ) -> anyhow::Result<()> {
        let cred = stream.peer_cred()?;
        let peer_uid = cred.uid();

        // Allow root and the daemon owner.
        if peer_uid == 0 || peer_uid == owner_uid {
            return Ok(());
        }

        // Check if the peer belongs to the aletheon group.
        // First check primary group (fast path, no allocation).
        if cred.gid() == group_gid {
            return Ok(());
        }
        // Then check supplementary groups via nix.
        if let Some(user) = User::from_uid(Uid::from_raw(peer_uid))? {
            let c_name = CString::new(user.name)?;
            let groups = nix::unistd::getgrouplist(&c_name, Gid::from_raw(cred.gid()))?;
            if groups.contains(&Gid::from_raw(group_gid)) {
                return Ok(());
            }
        }

        anyhow::bail!("Access denied: uid {} not in aletheon group", peer_uid)
    }

    /// Handle a single client connection. Reads JSON-RPC requests from the
    /// client and also writes out-of-band notifications (e.g. approval_request)
    /// from the handler's notification channel, and debug subscriber events.
    async fn handle_connection(
        stream: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
        handler: RequestHandler,
        mut notify_rx: mpsc::Receiver<String>,
    ) -> Result<()> {
        let (reader, mut writer) = tokio::io::split(stream);
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        // Debug subscriber receiver — populated when the client sends debug.subscribe.
        let mut debug_subscriber_rx: Option<mpsc::Receiver<DebugEvent>> = None;

        // Channel for receiving handler responses from background tasks.
        // This allows the select! loop to continue forwarding notifications
        // while the handler is processing a long-running request (e.g. LLM API call).
        let (resp_tx, mut resp_rx) = mpsc::channel::<String>(1);

        loop {
            tokio::select! {
                // Read incoming requests from the client.
                // Dispatch to a background task so the select loop continues
                // forwarding notifications while the handler processes.
                read_result = reader.read_line(&mut line) => {
                    let n = read_result?;
                    if n == 0 {
                        break; // Connection closed
                    }

                    let trimmed = line.trim().to_string();
                    line.clear();
                    if trimmed.is_empty() {
                        continue;
                    }

                    // Parse JSON request and spawn handler in background
                    let request: serde_json::Value = serde_json::from_str(&trimmed)?;
                    let handler = handler.clone();
                    let resp_tx = resp_tx.clone();
                    tokio::spawn(async move {
                        let response = handler.handle(request).await;
                        let response_json = serde_json::to_string(&response)
                            .unwrap_or_default();
                        let _ = resp_tx.send(response_json).await;
                    });
                }
                // Receive handler response from background task.
                response_json = resp_rx.recv() => {
                    if let Some(json) = response_json {
                        writer.write_all(json.as_bytes()).await?;
                        writer.write_all(b"\n").await?;
                        writer.flush().await?;

                        // Check if the debug handler has a pending subscriber rx
                        // (populated when debug.subscribe was just processed).
                        if let Some(rx) = handler.debug_handler().take_pending_subscriber_rx().await {
                            debug_subscriber_rx = Some(rx);
                            info!("Debug subscriber channel attached to client connection");
                        }
                    }
                }
                // Forward out-of-band notifications from the handler to the client.
                notification = notify_rx.recv() => {
                    match notification {
                        Some(msg) => {
                            writer.write_all(msg.as_bytes()).await?;
                            writer.write_all(b"\n").await?;
                            writer.flush().await?;
                        }
                        None => {
                            // Notification channel closed — handler dropped.
                            // Continue reading requests normally.
                        }
                    }
                }
                // Forward debug subscriber events to the client.
                debug_event = async {
                    match &mut debug_subscriber_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Some(event) = debug_event {
                        let json = serde_json::to_string(&event)?;
                        writer.write_all(json.as_bytes()).await?;
                        writer.write_all(b"\n").await?;
                        writer.flush().await?;
                    } else {
                        // Subscriber channel closed — clear it.
                        debug_subscriber_rx = None;
                    }
                }
            }
        }

        handler.decrement_connections();
        Ok(())
    }
}

use std::path::Path;

use base::debug::DebugEvent;
use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{error, info};

use super::handler::RequestHandler;

pub struct UnixServer {
    listener: UnixListener,
    handler: RequestHandler,
}

impl UnixServer {
    pub async fn new(socket_path: &Path, handler: RequestHandler) -> Result<Self> {
        // Remove stale socket
        if socket_path.exists() {
            tokio::fs::remove_file(socket_path).await?;
        }

        let listener = UnixListener::bind(socket_path)?;
        info!(path = %socket_path.display(), "Unix socket listening");

        Ok(Self { listener, handler })
    }

    pub async fn run(&mut self) -> Result<()> {
        loop {
            let (stream, _addr) = self.listener.accept().await?;
            let mut handler = self.handler.clone();

            // Create a per-connection notify channel so each client receives
            // its own events independently (shared channels would cause events
            // to be consumed by whichever connection reads first).
            let (notify_tx, notify_rx) = mpsc::channel::<String>(64);
            handler.set_notify_channel(notify_tx);

            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream, handler, notify_rx).await {
                    error!(error = %e, "Connection error");
                }
            });
        }
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

        Ok(())
    }
}

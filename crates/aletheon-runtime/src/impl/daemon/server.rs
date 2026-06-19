use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, Mutex};
use tracing::{error, info};

use super::handler::RequestHandler;

pub struct UnixServer {
    listener: UnixListener,
    handler: RequestHandler,
    /// Receiver for out-of-band notifications from the handler (e.g. approval_request).
    notify_rx: mpsc::Receiver<String>,
}

impl UnixServer {
    pub async fn new(socket_path: &Path, mut handler: RequestHandler) -> Result<Self> {
        // Remove stale socket
        if socket_path.exists() {
            tokio::fs::remove_file(socket_path).await?;
        }

        let listener = UnixListener::bind(socket_path)?;
        info!(path = %socket_path.display(), "Unix socket listening");

        // Create the notification channel: handler writes, server reads.
        let notify_rx = handler.create_notify_channel();

        Ok(Self {
            listener,
            handler,
            notify_rx,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        // Take ownership of notify_rx so we can pass it to the connection handler.
        // Only one connection at a time is expected to receive notifications.
        let notify_rx = std::mem::replace(
            &mut self.notify_rx,
            mpsc::channel(1).1, // placeholder
        );
        let notify_rx = Arc::new(Mutex::new(notify_rx));

        loop {
            let (stream, _addr) = self.listener.accept().await?;
            let handler = self.handler.clone();
            let notify_rx = notify_rx.clone();
            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream, handler, notify_rx).await {
                    error!(error = %e, "Connection error");
                }
            });
        }
    }

    /// Handle a single client connection. Reads JSON-RPC requests from the
    /// client and also writes out-of-band notifications (e.g. approval_request)
    /// from the handler's notification channel.
    async fn handle_connection(
        stream: UnixStream,
        handler: RequestHandler,
        notify_rx: Arc<Mutex<mpsc::Receiver<String>>>,
    ) -> Result<()> {
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            tokio::select! {
                // Read incoming requests from the client.
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

                    // Parse JSON request
                    let request: serde_json::Value = serde_json::from_str(&trimmed)?;
                    let response = handler.handle(request).await;
                    let response_json = serde_json::to_string(&response)?;
                    writer.write_all(response_json.as_bytes()).await?;
                    writer.write_all(b"\n").await?;
                    writer.flush().await?;
                }
                // Forward out-of-band notifications from the handler to the client.
                notification = async {
                    let mut rx = notify_rx.lock().await;
                    rx.recv().await
                } => {
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
            }
        }

        Ok(())
    }
}

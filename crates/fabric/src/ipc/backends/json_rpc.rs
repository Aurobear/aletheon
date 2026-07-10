use std::path::Path;

use anyhow::Result;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{error, info};

/// Adapter that bridges JSON-RPC (line-delimited JSON) to the IPC layer.
/// This replaces agentd's UnixServer while keeping CLI compatibility.
pub struct JsonRpcAdapter {
    socket_path: String,
    handler_tx: mpsc::Sender<(Value, mpsc::Sender<Value>)>,
}

impl JsonRpcAdapter {
    pub fn new(
        socket_path: impl Into<String>,
    ) -> (Self, mpsc::Receiver<(Value, mpsc::Sender<Value>)>) {
        let (tx, rx) = mpsc::channel(64);
        (
            Self {
                socket_path: socket_path.into(),
                handler_tx: tx,
            },
            rx,
        )
    }

    /// Start listening for JSON-RPC connections.
    pub async fn run(&self) -> Result<()> {
        let path = Path::new(&self.socket_path);
        if path.exists() {
            tokio::fs::remove_file(path).await?;
        }

        let listener = UnixListener::bind(path)?;
        info!(path = %path.display(), "JSON-RPC adapter listening");

        loop {
            let (stream, _) = listener.accept().await?;
            let handler_tx = self.handler_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream, handler_tx).await {
                    error!(error = %e, "Connection error");
                }
            });
        }
    }

    async fn handle_connection(
        stream: UnixStream,
        handler_tx: mpsc::Sender<(Value, mpsc::Sender<Value>)>,
    ) -> Result<()> {
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                break;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let request: Value = serde_json::from_str(trimmed)?;

            // Create response channel
            let (resp_tx, mut resp_rx) = mpsc::channel(1);

            // Send to handler
            if handler_tx.send((request, resp_tx)).await.is_err() {
                break;
            }

            // Wait for response
            if let Some(response) = resp_rx.recv().await {
                let response_json = serde_json::to_string(&response)?;
                writer.write_all(response_json.as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
            }
        }

        Ok(())
    }
}

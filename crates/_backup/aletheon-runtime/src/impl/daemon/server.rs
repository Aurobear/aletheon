use std::path::Path;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
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

    pub async fn run(&self) -> Result<()> {
        loop {
            let (stream, _addr) = self.listener.accept().await?;
            let handler = self.handler.clone();
            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream, handler).await {
                    error!(error = %e, "Connection error");
                }
            });
        }
    }

    async fn handle_connection(stream: UnixStream, handler: RequestHandler) -> Result<()> {
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                break; // Connection closed
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Parse JSON request
            let request: serde_json::Value = serde_json::from_str(trimmed)?;
            let response = handler.handle(request).await;
            let response_json = serde_json::to_string(&response)?;
            writer.write_all(response_json.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }

        Ok(())
    }
}

//! Shared JSON-RPC client helper — sends a request over a Unix socket and returns the response.

use std::path::Path;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Send a single JSON-RPC request over the Unix socket and return the response.
pub(crate) async fn send_rpc(
    socket: &Path,
    request: &serde_json::Value,
) -> Result<serde_json::Value> {
    let mut stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("Cannot connect to daemon socket: {}", socket.display()))?;

    let req_str = serde_json::to_string(request)?;
    stream.write_all(req_str.as_bytes()).await?;
    stream.write_all(b"\n").await?;

    let (reader, _) = stream.split();
    let mut reader = BufReader::new(reader);
    let mut response = String::new();
    reader.read_line(&mut response).await?;

    let resp: serde_json::Value =
        serde_json::from_str(&response).context("Failed to parse daemon response")?;

    Ok(resp)
}

/// Send a versioned typed request and reject incompatible daemon responses.
pub async fn send_typed_rpc<T, R>(
    socket: &Path,
    method: &str,
    id: u64,
    request: fabric::protocol::client::ClientMessage<T>,
) -> Result<R>
where
    T: serde::Serialize,
    R: serde::de::DeserializeOwned,
{
    let response = send_rpc(
        socket,
        &serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": request,
        }),
    )
    .await?;
    let result = response
        .get("result")
        .cloned()
        .context("typed RPC response omitted result")?;
    let message: fabric::protocol::client::ClientMessage<R> =
        serde_json::from_value(result).context("decode typed RPC response")?;
    message.into_v1().map_err(anyhow::Error::new)
}

//! Compatibility adapters for the pre-CGP-02 JSON-line protocol.
//!
//! The production Gateway session/turn path must use the typed
//! [`crate::GatewayClient`] surface. These adapters remain isolated here for
//! the Memory Agent and explicitly bounded rollback/debug routes; they are not
//! part of the typed Gateway client API.

use ::contracts::protocol::client::{
    ClientCapabilities, ClientEvent, ClientMessage, ClientRequest, InitializeParams,
    InitializedResult, CLIENT_PROTOCOL_VERSION,
};
use crate::protocol::ProtocolError;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Bounded compatibility transport for legacy extension/debug routes.
///
/// Business methods stay outside this crate; this type only owns the old
/// JSON-line framing so presentation crates do not open sockets or implement
/// request/response correlation themselves. New Session/Turn/Approval traffic
/// must use [`GatewayClient`] and the versioned typed envelope above.
pub struct LegacyJsonRpcClient {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    /// Bytes received by the non-blocking presentation poller. Keeping this
    /// buffer in the Gateway compatibility adapter prevents presentation code
    /// from reimplementing JSON-line framing or partial-read handling.
    pending_bytes: Vec<u8>,
}

/// Compatibility event stream returned by a legacy debug subscription. The
/// presentation crate consumes decoded JSON values but never owns line
/// framing or a Unix socket.
pub struct LegacyJsonRpcEventStream {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
}

/// Typed adapter for the versioned Fabric client protocol used by the
/// Memory Agent and other non-Gateway extension routes.  The presentation
/// crates must not construct JSON-RPC frames or correlate numeric ids; this
/// adapter owns that compatibility framing while exposing typed requests and
/// events to its callers.
pub struct LegacyProtocolClient {
    transport: LegacyJsonRpcClient,
    next_id: u64,
}

impl LegacyProtocolClient {
    pub async fn connect(path: impl AsRef<std::path::Path>) -> Result<Self, ProtocolError> {
        Ok(Self {
            transport: LegacyJsonRpcClient::connect(path).await?,
            next_id: 1,
        })
    }

    pub async fn request(&mut self, request: ClientRequest) -> Result<ClientEvent, ProtocolError> {
        let id = self.allocate_id();
        let value = request
            .to_json_rpc(id)
            .map_err(|_| ProtocolError::UnknownSchema)?;
        let response = self.request_value(&value).await?;
        let result = response
            .get("result")
            .cloned()
            .ok_or_else(|| response_error(&response))?;
        let message: ClientMessage<ClientEvent> =
            serde_json::from_value(result).map_err(|_| ProtocolError::UnknownSchema)?;
        message.into_v1().map_err(|_| ProtocolError::UnknownSchema)
    }

    /// Perform the versioned initialize/initialized handshake once per
    /// connection.  The second acknowledgement is an intentionally typed
    /// readiness check rather than a best-effort notification.
    pub async fn initialize(
        &mut self,
        client_version: String,
        capabilities: ClientCapabilities,
    ) -> Result<InitializedResult, ProtocolError> {
        let event = self
            .request(ClientRequest::Initialize(InitializeParams {
                client_version,
                protocol_versions: vec![CLIENT_PROTOCOL_VERSION],
                capabilities,
            }))
            .await?;
        let initialized = match event {
            ClientEvent::InitializeResponse(value) => value,
            _ => return Err(ProtocolError::UnknownSchema),
        };

        let id = self.allocate_id();
        let value = ClientRequest::Initialized
            .to_json_rpc(id)
            .map_err(|_| ProtocolError::UnknownSchema)?;
        let response = self.request_value(&value).await?;
        if response["result"]["status"] != "ready" {
            return Err(ProtocolError::Server(
                "daemon did not acknowledge initialized".into(),
            ));
        }
        Ok(initialized)
    }

    async fn request_value(
        &mut self,
        request: &serde_json::Value,
    ) -> Result<serde_json::Value, ProtocolError> {
        self.transport.request(request).await
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }
}

fn response_error(response: &serde_json::Value) -> ProtocolError {
    ProtocolError::Server(
        response["error"]["message"]
            .as_str()
            .unwrap_or("missing result")
            .to_owned(),
    )
}

impl LegacyJsonRpcEventStream {
    pub async fn next(&mut self) -> Result<Option<serde_json::Value>, ProtocolError> {
        let mut line = Vec::new();
        let read = self
            .reader
            .read_until(b'\n', &mut line)
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?;
        if read == 0 {
            return Ok(None);
        }
        let value = serde_json::from_slice(&line).map_err(|_| ProtocolError::UnknownSchema)?;
        Ok(Some(value))
    }
}

impl LegacyJsonRpcClient {
    pub async fn connect(path: impl AsRef<std::path::Path>) -> Result<Self, ProtocolError> {
        let stream = UnixStream::connect(path)
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(reader),
            writer,
            pending_bytes: Vec::new(),
        })
    }

    /// Construct a compatibility client around an already-connected stream.
    /// This is intentionally used only by deterministic in-memory TUI tests;
    /// production composition should call [`Self::connect`].
    pub fn from_stream(stream: UnixStream) -> Self {
        let (reader, writer) = stream.into_split();
        Self {
            reader: BufReader::new(reader),
            writer,
            pending_bytes: Vec::new(),
        }
    }

    /// Write one legacy JSON-RPC frame. The adapter owns newline framing;
    /// callers only pass a structured JSON value.
    pub async fn send(&mut self, request: &serde_json::Value) -> Result<(), ProtocolError> {
        let line = serde_json::to_vec(request).map_err(|_| ProtocolError::UnknownSchema)?;
        self.writer
            .write_all(&line)
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?;
        self.writer
            .write_all(b"\n")
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?;
        self.writer
            .flush()
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)
    }

    /// Wait until the underlying compatibility socket is readable.
    pub async fn readable(&mut self) -> Result<(), ProtocolError> {
        self.reader
            .get_mut()
            .readable()
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)
    }

    /// Poll one complete JSON frame without blocking. Partial UTF-8/JSON
    /// frames remain buffered until the next poll.
    pub fn try_next(&mut self) -> Result<Option<serde_json::Value>, ProtocolError> {
        loop {
            if let Some(index) = self.pending_bytes.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.pending_bytes.drain(..=index).collect();
                let line = line.strip_suffix(b"\n").unwrap_or(&line);
                if line.is_empty() {
                    continue;
                }
                return serde_json::from_slice(line)
                    .map(Some)
                    .map_err(|_| ProtocolError::UnknownSchema);
            }
            let mut chunk = [0_u8; 8192];
            match self.reader.get_mut().try_read(&mut chunk) {
                Ok(0) => {
                    return Err(ProtocolError::ConnectionClosed);
                }
                Ok(read) => self.pending_bytes.extend_from_slice(&chunk[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
                Err(_) => return Err(ProtocolError::ConnectionClosed),
            }
        }
    }

    pub async fn request(
        &mut self,
        request: &serde_json::Value,
    ) -> Result<serde_json::Value, ProtocolError> {
        self.send(request).await?;
        loop {
            let mut line = Vec::new();
            let read = self
                .reader
                .read_until(b'\n', &mut line)
                .await
                .map_err(|_| ProtocolError::ConnectionClosed)?;
            if read == 0 {
                return Err(ProtocolError::ConnectionClosed);
            }
            let response: serde_json::Value =
                serde_json::from_slice(&line).map_err(|_| ProtocolError::UnknownSchema)?;
            if response.get("id") == request.get("id") {
                return Ok(response);
            }
            // Legacy event/notification frames are not authority. Ignore
            // unrelated lines until the request-correlated response arrives.
        }
    }

    /// Send a legacy subscription request and return its correlated response
    /// together with the same connection as an event stream.
    pub async fn subscribe(
        mut self,
        request: &serde_json::Value,
    ) -> Result<(serde_json::Value, LegacyJsonRpcEventStream), ProtocolError> {
        self.send(request).await?;
        loop {
            let mut line = Vec::new();
            let read = self
                .reader
                .read_until(b'\n', &mut line)
                .await
                .map_err(|_| ProtocolError::ConnectionClosed)?;
            if read == 0 {
                return Err(ProtocolError::ConnectionClosed);
            }
            let response: serde_json::Value =
                serde_json::from_slice(&line).map_err(|_| ProtocolError::UnknownSchema)?;
            if response.get("id") == request.get("id") {
                let stream = LegacyJsonRpcEventStream {
                    reader: self.reader,
                };
                return Ok((response, stream));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn transport_owns_framing_and_ignores_unrelated_notifications() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("legacy.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut request = Vec::new();
            reader.read_until(b'\n', &mut request).await.unwrap();
            writer
                .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"event\",\"params\":{}}\n")
                .await
                .unwrap();
            let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
            let response = serde_json::json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": {"status": "ok"}
            });
            writer
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
        });
        let mut client = LegacyJsonRpcClient::connect(&path).await.unwrap();
        let response = client
            .request(&serde_json::json!({"jsonrpc":"2.0","id":7,"method":"health"}))
            .await
            .unwrap();
        assert_eq!(response["result"]["status"], "ok");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn subscription_returns_correlated_response_and_event_stream() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("subscription.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut request = Vec::new();
            reader.read_until(b'\n', &mut request).await.unwrap();
            let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
            let response = serde_json::json!({
                "jsonrpc":"2.0",
                "id":request["id"],
                "result":{"subscription_id":"sub-1"}
            });
            writer
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
            writer
                .write_all(b"{\"method\":\"event\",\"params\":{\"ts\":1}}\n")
                .await
                .unwrap();
        });
        let client = LegacyJsonRpcClient::connect(&path).await.unwrap();
        let (response, mut events) = client
            .subscribe(&serde_json::json!({
                "jsonrpc":"2.0","id":9,"method":"debug.subscribe"
            }))
            .await
            .unwrap();
        assert_eq!(response["result"]["subscription_id"], "sub-1");
        let event = events.next().await.unwrap().unwrap();
        assert_eq!(event["params"]["ts"], 1);
        server.await.unwrap();
    }
}

//! Transport for the initialized, versioned client protocol.
//!
//! The Memory Agent uses a negotiated typed contract over JSON-RPC framing.
//! Unversioned JSON-RPC routes are retired; raw framing is not public API.

use crate::protocol::ProtocolError;
use ::contracts::protocol::client::{
    ClientCapabilities, ClientEvent, ClientMessage, ClientRequest, InitializeParams,
    InitializedResult, CLIENT_PROTOCOL_VERSION,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Typed adapter for the negotiated client protocol used by the Memory Agent.
pub struct VersionedProtocolClient {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    next_id: u64,
}

impl VersionedProtocolClient {
    pub async fn connect(path: impl AsRef<std::path::Path>) -> Result<Self, ProtocolError> {
        let stream = UnixStream::connect(path)
            .await
            .map_err(|_| ProtocolError::ConnectionClosed)?;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(reader),
            writer,
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

    /// Perform the initialize/initialized handshake once per connection.
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
            .map_err(|_| ProtocolError::ConnectionClosed)?;

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
            // Notifications are not request authority. Ignore unrelated frames
            // until the correlated typed response arrives.
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn versioned_transport_correlates_responses() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("versioned.sock");
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
        let mut client = VersionedProtocolClient::connect(&path).await.unwrap();
        let response = client
            .request_value(&serde_json::json!({"jsonrpc":"2.0","id":7,"method":"health"}))
            .await
            .unwrap();
        assert_eq!(response["result"]["status"], "ok");
        server.await.unwrap();
    }
}

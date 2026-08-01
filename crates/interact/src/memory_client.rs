//! Versioned client used by the independent Aletheon Memory Agent process.

use std::path::PathBuf;

use fabric::protocol::client::{
    ClientCapabilities, ClientEvent, ClientMessage, ClientRequest, InitializeParams,
    CLIENT_PROTOCOL_VERSION,
};
use fabric::protocol::memory_maintenance::{
    MemoryMaintenancePhaseV1, MemoryMaintenanceRunReceiptV1, MemoryMaintenanceRunRequestV1,
    MemoryMaintenanceStatusRequestV1, MemoryMaintenanceStatusV1,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufStream};
use tokio::net::UnixStream;

pub struct MemoryAgentClient {
    stream: BufStream<UnixStream>,
    next_id: u64,
}

impl MemoryAgentClient {
    pub async fn connect_official(explicit_socket: Option<PathBuf>) -> anyhow::Result<Self> {
        let socket = crate::host::resolve_user_socket(explicit_socket)?;
        let stream = UnixStream::connect(&socket)
            .await
            .map_err(|error| anyhow::anyhow!("connecting {}: {error}", socket.display()))?;
        let mut client = Self {
            stream: BufStream::new(stream),
            next_id: 1,
        };
        client.initialize().await?;
        Ok(client)
    }

    pub async fn status(
        &mut self,
        request_id: impl Into<String>,
    ) -> anyhow::Result<MemoryMaintenanceStatusV1> {
        let event = self
            .request(ClientRequest::MemoryMaintenanceStatus(
                MemoryMaintenanceStatusRequestV1 {
                    request_id: request_id.into(),
                },
            ))
            .await?;
        match event {
            ClientEvent::MemoryMaintenanceStatus(status) => Ok(status),
            other => anyhow::bail!("unexpected maintenance status response: {other:?}"),
        }
    }

    pub async fn run(
        &mut self,
        request_id: impl Into<String>,
        max_items: u16,
        dry_run: bool,
    ) -> anyhow::Result<MemoryMaintenanceRunReceiptV1> {
        let request = MemoryMaintenanceRunRequestV1 {
            request_id: request_id.into(),
            phase: MemoryMaintenancePhaseV1::IntakeEvaluation,
            max_items,
            dry_run,
        };
        request.validate()?;
        let event = self
            .request(ClientRequest::MemoryMaintenanceRun(request))
            .await?;
        match event {
            ClientEvent::MemoryMaintenanceRunReceipt(receipt) => Ok(receipt),
            other => anyhow::bail!("unexpected maintenance run response: {other:?}"),
        }
    }

    async fn initialize(&mut self) -> anyhow::Result<()> {
        let event = self
            .request(ClientRequest::Initialize(InitializeParams {
                client_version: env!("CARGO_PKG_VERSION").into(),
                protocol_versions: vec![CLIENT_PROTOCOL_VERSION],
                capabilities: ClientCapabilities {
                    item_events: false,
                    cursors: false,
                    memory_gateway_v1: false,
                    memory_maintenance_v1: true,
                },
            }))
            .await?;
        let initialized = match event {
            ClientEvent::InitializeResponse(initialized) => initialized,
            other => anyhow::bail!("unexpected initialize response: {other:?}"),
        };
        anyhow::ensure!(
            initialized.protocol_version == CLIENT_PROTOCOL_VERSION
                && initialized.server_capabilities.memory_maintenance_v1,
            "daemon did not negotiate memory_maintenance_v1"
        );
        let request_id = self.allocate_id();
        let value = ClientRequest::Initialized.to_json_rpc(request_id)?;
        self.write(&value).await?;
        let response = self.read_response(request_id).await?;
        anyhow::ensure!(
            response["result"]["status"] == "ready",
            "daemon did not acknowledge initialized"
        );
        Ok(())
    }

    async fn request(&mut self, request: ClientRequest) -> anyhow::Result<ClientEvent> {
        let request_id = self.allocate_id();
        let value = request.to_json_rpc(request_id)?;
        self.write(&value).await?;
        let response = self.read_response(request_id).await?;
        let result = response
            .get("result")
            .cloned()
            .ok_or_else(|| response_error(&response))?;
        let message: ClientMessage<ClientEvent> = serde_json::from_value(result)?;
        Ok(message.into_v1()?)
    }

    async fn write(&mut self, value: &serde_json::Value) -> anyhow::Result<()> {
        self.stream.write_all(value.to_string().as_bytes()).await?;
        self.stream.write_all(b"\n").await?;
        self.stream.flush().await?;
        Ok(())
    }

    async fn read_response(&mut self, request_id: u64) -> anyhow::Result<serde_json::Value> {
        for _ in 0..32 {
            let mut line = String::new();
            anyhow::ensure!(
                self.stream.read_line(&mut line).await? > 0,
                "daemon closed the Memory Agent connection"
            );
            let value: serde_json::Value = serde_json::from_str(line.trim())?;
            if value.get("id").and_then(serde_json::Value::as_u64) == Some(request_id) {
                return Ok(value);
            }
            // The maintenance connection never subscribes. Ignore bounded
            // unrelated notifications rather than interpreting them as a
            // terminal response for this request.
        }
        anyhow::bail!("too many unrelated daemon messages on Memory Agent connection")
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }
}

fn response_error(response: &serde_json::Value) -> anyhow::Error {
    anyhow::anyhow!(
        "daemon request failed: {}",
        response["error"]["message"]
            .as_str()
            .unwrap_or("missing result")
    )
}

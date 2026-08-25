//! Versioned client used by the independent Aletheon Memory Agent process.

use std::path::PathBuf;

use ::contracts::protocol::client::{
    ClientCapabilities, ClientEvent, ClientRequest, CLIENT_PROTOCOL_VERSION,
};
use ::contracts::protocol::memory::{
    MemoryFeedbackReceiptV1, MemoryFeedbackRequestV1, MemoryLifecycleReceiptV1,
    MemoryObservationReceiptV1, MemoryObservationRequestV1, MemoryRecallRequestV1,
    MemoryRecallResultV1, MemoryReceiptGetRequestV1, MemoryWorkspaceBindRequestV1,
    MemoryWorkspaceBindingPreviewV1, MemoryWorkspaceBindingViewV1,
    MemoryWorkspacePreviewBindRequestV1, MemoryWorkspaceUnbindRequestV1,
};
use ::contracts::protocol::memory_maintenance::{
    MemoryMaintenancePhaseV1, MemoryMaintenanceRunReceiptV1, MemoryMaintenanceRunRequestV1,
    MemoryMaintenanceStatusRequestV1, MemoryMaintenanceStatusV1,
};
use gateway::client::VersionedProtocolClient;

pub struct MemoryClient {
    transport: VersionedProtocolClient,
}

pub type MemoryAgentClient = MemoryClient;

impl MemoryClient {
    pub async fn connect_official(explicit_socket: Option<PathBuf>) -> anyhow::Result<Self> {
        Self::connect(explicit_socket, false, true, false).await
    }

    pub async fn connect_admin(explicit_socket: Option<PathBuf>) -> anyhow::Result<Self> {
        Self::connect(explicit_socket, false, false, true).await
    }

    pub async fn connect_gateway(explicit_socket: Option<PathBuf>) -> anyhow::Result<Self> {
        Self::connect(explicit_socket, true, false, false).await
    }

    async fn connect(
        explicit_socket: Option<PathBuf>,
        memory_gateway_v1: bool,
        memory_maintenance_v1: bool,
        memory_admin_v1: bool,
    ) -> anyhow::Result<Self> {
        let socket = crate::host::resolve_user_socket(explicit_socket)?;
        let transport = VersionedProtocolClient::connect(&socket)
            .await
            .map_err(|error| anyhow::anyhow!("connecting {}: {error}", socket.display()))?;
        let mut client = Self { transport };
        client
            .initialize(memory_gateway_v1, memory_maintenance_v1, memory_admin_v1)
            .await?;
        Ok(client)
    }

    pub async fn observe(
        &mut self,
        request: MemoryObservationRequestV1,
    ) -> anyhow::Result<MemoryObservationReceiptV1> {
        match self.request(ClientRequest::MemoryObserve(request)).await? {
            ClientEvent::MemoryObservationReceipt(receipt) => Ok(receipt),
            other => anyhow::bail!("unexpected memory observation response: {other:?}"),
        }
    }

    pub async fn receipt(
        &mut self,
        request: MemoryReceiptGetRequestV1,
    ) -> anyhow::Result<MemoryLifecycleReceiptV1> {
        match self
            .request(ClientRequest::MemoryReceiptGet(request))
            .await?
        {
            ClientEvent::MemoryLifecycleReceipt(receipt) => Ok(receipt),
            other => anyhow::bail!("unexpected memory lifecycle response: {other:?}"),
        }
    }

    pub async fn recall(
        &mut self,
        request: MemoryRecallRequestV1,
    ) -> anyhow::Result<MemoryRecallResultV1> {
        match self.request(ClientRequest::MemoryRecall(request)).await? {
            ClientEvent::MemoryRecallResult(result) => Ok(result),
            other => anyhow::bail!("unexpected memory recall response: {other:?}"),
        }
    }

    pub async fn feedback(
        &mut self,
        request: MemoryFeedbackRequestV1,
    ) -> anyhow::Result<MemoryFeedbackReceiptV1> {
        match self.request(ClientRequest::MemoryFeedback(request)).await? {
            ClientEvent::MemoryFeedbackReceipt(receipt) => Ok(receipt),
            other => anyhow::bail!("unexpected memory feedback response: {other:?}"),
        }
    }

    pub async fn preview_bind(
        &mut self,
        request: MemoryWorkspacePreviewBindRequestV1,
    ) -> anyhow::Result<MemoryWorkspaceBindingPreviewV1> {
        match self
            .request(ClientRequest::MemoryWorkspacePreviewBind(request))
            .await?
        {
            ClientEvent::MemoryWorkspaceBindingPreview(preview) => Ok(preview),
            other => anyhow::bail!("unexpected memory binding preview response: {other:?}"),
        }
    }

    pub async fn bind(
        &mut self,
        request: MemoryWorkspaceBindRequestV1,
    ) -> anyhow::Result<MemoryWorkspaceBindingViewV1> {
        match self
            .request(ClientRequest::MemoryWorkspaceBind(request))
            .await?
        {
            ClientEvent::MemoryWorkspaceBinding(binding) => Ok(binding),
            other => anyhow::bail!("unexpected memory binding response: {other:?}"),
        }
    }

    pub async fn unbind(
        &mut self,
        request: MemoryWorkspaceUnbindRequestV1,
    ) -> anyhow::Result<MemoryWorkspaceBindingViewV1> {
        match self
            .request(ClientRequest::MemoryWorkspaceUnbind(request))
            .await?
        {
            ClientEvent::MemoryWorkspaceBinding(binding) => Ok(binding),
            other => anyhow::bail!("unexpected memory unbind response: {other:?}"),
        }
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

    async fn initialize(
        &mut self,
        memory_gateway_v1: bool,
        memory_maintenance_v1: bool,
        memory_admin_v1: bool,
    ) -> anyhow::Result<()> {
        let initialized = self
            .transport
            .initialize(
                env!("CARGO_PKG_VERSION").into(),
                ClientCapabilities {
                    item_events: false,
                    cursors: false,
                    memory_gateway_v1,
                    memory_maintenance_v1,
                    memory_admin_v1,
                },
            )
            .await?;
        anyhow::ensure!(
            initialized.protocol_version == CLIENT_PROTOCOL_VERSION
                && (!memory_gateway_v1 || initialized.server_capabilities.memory_gateway_v1)
                && (!memory_maintenance_v1
                    || initialized.server_capabilities.memory_maintenance_v1)
                && (!memory_admin_v1 || initialized.server_capabilities.memory_admin_v1),
            "daemon did not negotiate requested memory capability"
        );
        Ok(())
    }

    async fn request(&mut self, request: ClientRequest) -> anyhow::Result<ClientEvent> {
        self.transport
            .request(request)
            .await
            .map_err(|error| anyhow::anyhow!("Memory Agent request failed: {error}"))
    }
}

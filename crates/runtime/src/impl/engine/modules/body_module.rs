//! BodyModule — bus handler wrapping ToolRegistry.
//!
//! Registers on `ModuleId::Body` and handles `BodyRequest` envelopes.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;
use tracing::{debug, error};

use base::envelope::*;
use base::envelope::Payload;
use base::CommunicationBus;

use tools::tools::ToolRegistry;

use super::{BodyRequest, BodyResponse, ToolDefinitionMsg};

/// Bus handler for the Body module.
///
/// Wraps `ToolRegistry` and exposes tool definitions and lookup via the bus.
pub struct BodyModule {
    registry: Arc<Mutex<ToolRegistry>>,
}

impl BodyModule {
    /// Create a new BodyModule with shared instance.
    pub fn new(registry: Arc<Mutex<ToolRegistry>>) -> Self {
        Self { registry }
    }

    /// Run the module loop: receive envelopes, dispatch, reply.
    pub async fn run(self, bus: Arc<CommunicationBus>) {
        let mut rx = bus.register_module(ModuleId::Body, None);
        debug!("BodyModule: registered on ModuleId::Body");

        while let Some(envelope) = rx.recv().await {
            self.handle_envelope(&bus, envelope).await;
        }
        debug!("BodyModule: mailbox closed, shutting down");
    }

    async fn handle_envelope(&self, bus: &CommunicationBus, envelope: Envelope) {
        let response = match deserialize_request(&envelope) {
            Ok(request) => self.dispatch(request).await,
            Err(e) => BodyResponse::Error {
                message: format!("Deserialization failed: {}", e),
            },
        };

        let resp_envelope = Envelope::response(&envelope, serialize_response(&response));
        if let Err(e) = bus.send(resp_envelope).await {
            error!("BodyModule: failed to send response: {}", e);
        }
    }

    async fn dispatch(&self, request: BodyRequest) -> BodyResponse {
        let registry = self.registry.lock().await;
        match request {
            BodyRequest::Definitions => {
                let defs = registry.definitions();
                let tools = defs
                    .into_iter()
                    .map(|d| ToolDefinitionMsg {
                        name: d.name,
                        description: d.description,
                        input_schema: d.input_schema,
                    })
                    .collect();
                BodyResponse::Definitions { tools }
            }
            BodyRequest::GetTool { name } => {
                if let Some(tool) = registry.get(&name) {
                    BodyResponse::ToolFound {
                        name: tool.name().to_string(),
                        description: tool.description().to_string(),
                    }
                } else {
                    BodyResponse::ToolNotFound { name }
                }
            }
            BodyRequest::ListTools => {
                let names: Vec<String> = registry.list().iter().map(|s| s.to_string()).collect();
                BodyResponse::ToolList { names }
            }
        }
    }
}

fn deserialize_request(envelope: &Envelope) -> Result<BodyRequest> {
    match &envelope.payload {
        Payload::Json(value) => Ok(serde_json::from_value(value.clone())?),
        _ => anyhow::bail!("BodyModule: expected JSON payload"),
    }
}

fn serialize_response(response: &BodyResponse) -> Payload {
    Payload::Json(serde_json::to_value(response).unwrap_or_default())
}

//! SelfFieldModule — bus handler wrapping SelfField.
//!
//! Registers on `ModuleId::SelfField` and handles `SelfFieldRequest` envelopes.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;
use tracing::{debug, error};

use base::envelope::Payload;
use base::envelope::*;
use base::CommunicationBus;
use base::SelfFieldOps;

use dasein::core::SelfField;

use super::{SelfFieldRequest, SelfFieldResponse};

/// Bus handler for the SelfField module.
///
/// Wraps `SelfField` and exposes review, narrate, identity, and cares via the bus.
pub struct SelfFieldModule {
    self_field: Arc<Mutex<SelfField>>,
}

impl SelfFieldModule {
    /// Create a new SelfFieldModule with a shared SelfField instance.
    pub fn new(self_field: Arc<Mutex<SelfField>>) -> Self {
        Self { self_field }
    }

    /// Run the module loop: receive envelopes, dispatch, reply.
    pub async fn run(self, bus: Arc<CommunicationBus>) {
        let mut rx = bus.register_module(ModuleId::SelfField, None);
        debug!("SelfFieldModule: registered on ModuleId::SelfField");

        while let Some(envelope) = rx.recv().await {
            self.handle_envelope(&bus, envelope).await;
        }
        debug!("SelfFieldModule: mailbox closed, shutting down");
    }

    async fn handle_envelope(&self, bus: &CommunicationBus, envelope: Envelope) {
        let response = match deserialize_request(&envelope) {
            Ok(request) => self.dispatch(request).await,
            Err(e) => SelfFieldResponse::Error {
                message: format!("Deserialization failed: {}", e),
            },
        };

        let resp_envelope = Envelope::response(&envelope, serialize_response(&response));
        if let Err(e) = bus.send(resp_envelope).await {
            error!("SelfFieldModule: failed to send response: {}", e);
        }
    }

    async fn dispatch(&self, request: SelfFieldRequest) -> SelfFieldResponse {
        let sf = self.self_field.lock().await;
        match request {
            SelfFieldRequest::Review { intent, ctx } => {
                // Deserialize the context from the request instead of using a default
                let ctx: base::Context = serde_json::from_value(ctx).unwrap_or_else(|_| {
                    base::Context::new("bus-request", std::path::PathBuf::from("."))
                });
                match sf.review(&intent, &ctx).await {
                    Ok(verdict) => SelfFieldResponse::Verdict { verdict },
                    Err(e) => SelfFieldResponse::Error {
                        message: format!("Review failed: {}", e),
                    },
                }
            }
            SelfFieldRequest::Narrate { event, reason } => {
                match sf.narrate(&event, &reason).await {
                    Ok(()) => SelfFieldResponse::Narrated,
                    Err(e) => SelfFieldResponse::Error {
                        message: format!("Narrate failed: {}", e),
                    },
                }
            }
            SelfFieldRequest::GetIdentity => match sf.identity().await {
                Ok(identity) => SelfFieldResponse::Identity { identity },
                Err(e) => SelfFieldResponse::Error {
                    message: format!("Identity failed: {}", e),
                },
            },
            SelfFieldRequest::GetCares => match sf.cares().await {
                Ok(cares) => SelfFieldResponse::Cares { cares },
                Err(e) => SelfFieldResponse::Error {
                    message: format!("Cares failed: {}", e),
                },
            },
        }
    }
}

fn deserialize_request(envelope: &Envelope) -> Result<SelfFieldRequest> {
    match &envelope.payload {
        Payload::Json(value) => Ok(serde_json::from_value(value.clone())?),
        _ => anyhow::bail!("SelfFieldModule: expected JSON payload"),
    }
}

fn serialize_response(response: &SelfFieldResponse) -> Payload {
    Payload::Json(serde_json::to_value(response).unwrap_or_default())
}

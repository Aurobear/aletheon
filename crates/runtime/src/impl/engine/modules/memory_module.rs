//! MemoryModule — bus handler wrapping CoreMemory and RecallMemory.
//!
//! Registers on `ModuleId::Memory` and handles `MemoryRequest` envelopes.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;
use tracing::{debug, error};

use base::envelope::*;
use base::envelope::Payload;
use base::CommunicationBus;

use crate::r#impl::memory::recall_memory::RecallMemory;
use crate::r#impl::memory::CoreMemory;

use super::{MemoryRequest, MemoryResponse, RecallEntry};

/// Bus handler for the Memory module.
///
/// Wraps `CoreMemory` (L1 context blocks) and `RecallMemory` (L2 SQLite history).
/// Dispatches incoming `MemoryRequest` envelopes and sends back `MemoryResponse`.
pub struct MemoryModule {
    core: Arc<Mutex<CoreMemory>>,
    recall: Option<Arc<Mutex<RecallMemory>>>,
}

impl MemoryModule {
    /// Create a new MemoryModule with shared instances.
    pub fn new(core: Arc<Mutex<CoreMemory>>, recall: Option<Arc<Mutex<RecallMemory>>>) -> Self {
        Self { core, recall }
    }

    /// Run the module loop: receive envelopes, dispatch, reply.
    pub async fn run(self, bus: Arc<CommunicationBus>) {
        let mut rx = bus.register_module(ModuleId::Memory, None);
        debug!("MemoryModule: registered on ModuleId::Memory");

        while let Some(envelope) = rx.recv().await {
            self.handle_envelope(&bus, envelope).await;
        }
        debug!("MemoryModule: mailbox closed, shutting down");
    }

    async fn handle_envelope(&self, bus: &CommunicationBus, envelope: Envelope) {
        let response = match deserialize_request(&envelope) {
            Ok(request) => self.dispatch(request).await,
            Err(e) => MemoryResponse::Error {
                message: format!("Deserialization failed: {}", e),
            },
        };

        let resp_envelope = Envelope::response(&envelope, serialize_response(&response));
        if let Err(e) = bus.send(resp_envelope).await {
            error!("MemoryModule: failed to send response: {}", e);
        }
    }

    async fn dispatch(&self, request: MemoryRequest) -> MemoryResponse {
        match request {
            MemoryRequest::FormatForContext => {
                let core = self.core.lock().await;
                let text = core.format_for_context();
                MemoryResponse::ContextFormatted { text }
            }
            MemoryRequest::StoreRecall {
                session_id,
                entry_type,
                content,
                metadata,
            } => match &self.recall {
                Some(recall_arc) => {
                    let recall = recall_arc.lock().await;
                    match recall.store(&session_id, &entry_type, &content, metadata.as_deref()) {
                        Ok(id) => MemoryResponse::RecallStored { id },
                        Err(e) => MemoryResponse::Error {
                            message: format!("Store failed: {}", e),
                        },
                    }
                }
                None => MemoryResponse::Error {
                    message: "RecallMemory not initialized".to_string(),
                },
            },
            MemoryRequest::SearchRecall { query, limit } => match &self.recall {
                Some(recall_arc) => {
                    let recall = recall_arc.lock().await;
                    match recall.search(&query, limit) {
                        Ok(entries) => {
                            let entries = entries
                                .into_iter()
                                .map(|e| RecallEntry {
                                    id: e.id,
                                    session_id: e.session_id,
                                    entry_type: e.entry_type,
                                    content: e.content,
                                    metadata: e.metadata,
                                })
                                .collect();
                            MemoryResponse::RecallSearchResults { entries }
                        }
                        Err(e) => MemoryResponse::Error {
                            message: format!("Search failed: {}", e),
                        },
                    }
                }
                None => MemoryResponse::Error {
                    message: "RecallMemory not initialized".to_string(),
                },
            },
        }
    }
}

fn deserialize_request(envelope: &Envelope) -> Result<MemoryRequest> {
    match &envelope.payload {
        Payload::Json(value) => Ok(serde_json::from_value(value.clone())?),
        _ => anyhow::bail!("MemoryModule: expected JSON payload"),
    }
}

fn serialize_response(response: &MemoryResponse) -> Payload {
    Payload::Json(serde_json::to_value(response).unwrap_or_default())
}

//! Tool dispatch and execution helpers for the cognitive engine.
//!
//! The actual tool dispatch logic is integrated into the ReAct loop in
//! cognitive_loop.rs and streaming.rs. This module exists for future
//! extraction of tool-related helper functions if needed.

use std::time::Duration;

use tracing::warn;

use base::envelope::*;
use base::ToolDefinition;
use comm::envelope::Payload;

use super::cognitive_loop::Engine;
use super::modules::{BodyRequest, BodyResponse};

impl Engine {
    /// Get tool definitions for LLM function-calling.
    ///
    /// Uses the CommunicationBus (request to BodyModule) if available,
    /// falls back to direct `ToolRegistry::definitions()` for backward compatibility.
    pub(super) async fn get_tool_definitions(&self) -> Vec<ToolDefinition> {
        if let Some(ref bus) = self.bus {
            let req = BodyRequest::Definitions;
            let envelope = Envelope::request(
                Endpoint::Module(ModuleId::Runtime),
                Target::Module(ModuleId::Body),
                Payload::Json(serde_json::to_value(&req).unwrap_or_default()),
                Duration::from_secs(5),
            );
            match bus.request(envelope).await {
                Ok(resp_envelope) => {
                    if let Payload::Json(val) = &resp_envelope.payload {
                        match serde_json::from_value::<BodyResponse>(val.clone()) {
                            Ok(BodyResponse::Definitions { tools }) => {
                                return tools
                                    .into_iter()
                                    .map(|t| ToolDefinition {
                                        name: t.name,
                                        description: t.description,
                                        input_schema: t.input_schema,
                                    })
                                    .collect();
                            }
                            Ok(BodyResponse::Error { message }) => {
                                warn!(error = %message, "BodyModule returned error for Definitions");
                            }
                            Ok(other) => {
                                warn!(
                                    ?other,
                                    "Unexpected response from BodyModule for Definitions"
                                );
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to deserialize BodyResponse");
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Bus request for Definitions failed, falling back to direct");
                }
            }
        }
        // Fallback: direct tool registry access
        self.tools.definitions()
    }
}

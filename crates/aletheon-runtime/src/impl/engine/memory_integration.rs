use std::sync::Arc;
use std::time::Duration;

use tracing::{info, warn};

use aletheon_abi::envelope::*;
use aletheon_abi::tool::ToolResult;
use aletheon_brain::r#impl::learning::{OutcomeContext, OutcomeRecord};
use aletheon_comm::envelope::Payload;
use aletheon_comm::CommunicationBus;

use super::cognitive_loop::Engine;
use super::modules::{MemoryRequest, MemoryResponse};

impl Engine {
    /// Record a tool call outcome for the learning pipeline.
    pub(super) async fn record_tool_outcome(
        &mut self,
        session_id: &str,
        turn_id: &str,
        tool_name: &str,
        tool_input: &serde_json::Value,
        result: &ToolResult,
        iteration: usize,
    ) {
        let record = OutcomeRecord {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            tool_name: tool_name.to_string(),
            args: tool_input.clone(),
            result_summary: if result.content.len() > 500 {
                format!("{}...", &result.content[..500])
            } else {
                result.content.clone()
            },
            is_error: result.is_error,
            user_feedback: None,
            timestamp: chrono::Utc::now(),
            context: OutcomeContext {
                preceding_errors: 0,
                iteration_count: iteration,
                system_state: None,
            },
        };

        // Record to learning SQLite database
        if let Some(ref recorder) = self.outcome_recorder {
            if let Err(e) = recorder.record(&record) {
                warn!(error = %e, "Failed to record tool outcome");
            }
        }

        // Also store in recall memory for cross-session persistence
        {
            let rm = self.recall_memory.lock().await;
            let metadata = serde_json::json!({
                "type": "learning_outcome",
                "tool": tool_name,
                "is_error": result.is_error,
                "iteration": iteration,
            });
            let label = if result.is_error { "ERROR" } else { "OK" };
            let content = format!("[{}] {}: {}", label, tool_name, record.result_summary);
            if let Err(e) = rm.store(
                session_id,
                "learning_outcome",
                &content,
                Some(&metadata.to_string()),
            ) {
                warn!(error = %e, "Failed to store learning outcome in recall memory");
            }
        }

        // Extract patterns from recent outcomes and update rule store
        if let (Some(ref recorder), Some(ref extractor)) =
            (&self.outcome_recorder, &self.pattern_extractor)
        {
            if let Ok(recent_outcomes) = recorder.get_recent(100) {
                let new_rules = extractor.extract(&recent_outcomes);
                for rule in new_rules {
                    info!(rule_id = %rule.id, rule_type = %rule.rule_type, tool = %rule.tool_pattern, "Adding learned rule");
                    self.rule_store.add(rule);
                }
            }
        }
    }

    /// Get core memory context for system prompt injection.
    ///
    /// Uses the CommunicationBus (request to MemoryModule) if available,
    /// falls back to direct lock on CoreMemory for backward compatibility.
    pub(super) async fn get_core_memory_context(&self) -> String {
        if let Some(ref bus) = self.bus {
            let req = MemoryRequest::FormatForContext;
            let envelope = Envelope::request(
                Endpoint::Module(ModuleId::Runtime),
                Target::Module(ModuleId::Memory),
                Payload::Json(serde_json::to_value(&req).unwrap_or_default()),
                Duration::from_secs(5),
            );
            match bus.request(envelope).await {
                Ok(resp_envelope) => {
                    if let Payload::Json(val) = &resp_envelope.payload {
                        match serde_json::from_value::<MemoryResponse>(val.clone()) {
                            Ok(MemoryResponse::ContextFormatted { text }) => return text,
                            Ok(MemoryResponse::Error { message }) => {
                                warn!(error = %message, "MemoryModule returned error for FormatForContext");
                            }
                            Ok(other) => {
                                warn!(
                                    ?other,
                                    "Unexpected response from MemoryModule for FormatForContext"
                                );
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to deserialize MemoryResponse");
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Bus request for FormatForContext failed, falling back to direct lock");
                }
            }
        }
        // Fallback: direct lock
        let cm = self.core_memory.lock().await;
        cm.format_for_context()
    }

    /// Store a recall memory entry.
    ///
    /// Uses the CommunicationBus (request to MemoryModule) if available,
    /// falls back to direct lock on RecallMemory for backward compatibility.
    pub(super) async fn store_recall(
        &self,
        session_id: &str,
        entry_type: &str,
        content: &str,
        metadata: Option<&str>,
    ) {
        if let Some(ref bus) = self.bus {
            let req = MemoryRequest::StoreRecall {
                session_id: session_id.to_string(),
                entry_type: entry_type.to_string(),
                content: content.to_string(),
                metadata: metadata.map(|s| s.to_string()),
            };
            let envelope = Envelope::request(
                Endpoint::Module(ModuleId::Runtime),
                Target::Module(ModuleId::Memory),
                Payload::Json(serde_json::to_value(&req).unwrap_or_default()),
                Duration::from_secs(5),
            );
            match bus.request(envelope).await {
                Ok(resp_envelope) => {
                    if let Payload::Json(val) = &resp_envelope.payload {
                        match serde_json::from_value::<MemoryResponse>(val.clone()) {
                            Ok(MemoryResponse::RecallStored { .. }) => return,
                            Ok(MemoryResponse::Error { message }) => {
                                warn!(error = %message, "MemoryModule returned error for StoreRecall");
                            }
                            Ok(other) => {
                                warn!(
                                    ?other,
                                    "Unexpected response from MemoryModule for StoreRecall"
                                );
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to deserialize MemoryResponse");
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Bus request for StoreRecall failed, falling back to direct lock");
                }
            }
        }
        // Fallback: direct lock
        let rm = self.recall_memory.lock().await;
        if let Err(e) = rm.store(session_id, entry_type, content, metadata) {
            warn!(error = %e, "Failed to store recall memory entry (fallback)");
        }
    }
}

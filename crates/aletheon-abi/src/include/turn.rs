//! Turn service contracts shared by executive adapters and cognitive sessions.

use crate::types::llm_types::{LlmProvider, ToolDefinition};
use crate::types::message::Message;
use crate::types::operation::ProcessId;
use crate::types::turn::{TurnEvent, TurnRequest};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecallRequest {
    pub session_id: String,
    pub input: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecallSet {
    pub snippets: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaseinView {
    pub text: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgoraView {
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRequest {
    pub process_id: ProcessId,
    pub name: String,
    pub input: serde_json::Value,
    pub call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityResult {
    pub call_id: String,
    pub output: String,
    pub is_error: bool,
}

#[async_trait]
pub trait TurnEventSink: Send + Sync {
    async fn emit(&self, event: TurnEvent);
}

#[async_trait]
pub trait TurnServices: Send + Sync {
    async fn recall(&self, req: RecallRequest) -> Result<RecallSet>;
    async fn dasein_view(&self, process: ProcessId) -> Result<DaseinView>;
    async fn agora_view(&self, session_id: &str) -> Result<AgoraView>;
    async fn invoke(&self, req: CapabilityRequest) -> CapabilityResult;

    fn llm_provider(&self) -> Option<&dyn LlmProvider> {
        None
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        Vec::new()
    }

    fn seed_messages(&self, _request: &TurnRequest) -> Vec<Message> {
        Vec::new()
    }
}

pub struct NoopTurnEventSink;

#[async_trait]
impl TurnEventSink for NoopTurnEventSink {
    async fn emit(&self, _event: TurnEvent) {}
}

#[derive(Default)]
pub struct StubTurnServices;

#[async_trait]
impl TurnServices for StubTurnServices {
    async fn recall(&self, _req: RecallRequest) -> Result<RecallSet> {
        Ok(RecallSet::default())
    }

    async fn dasein_view(&self, _process: ProcessId) -> Result<DaseinView> {
        Ok(DaseinView::default())
    }

    async fn agora_view(&self, _session_id: &str) -> Result<AgoraView> {
        Ok(AgoraView::default())
    }

    async fn invoke(&self, req: CapabilityRequest) -> CapabilityResult {
        CapabilityResult {
            call_id: req.call_id,
            output: format!("tool {} is unavailable in stub", req.name),
            is_error: true,
        }
    }
}

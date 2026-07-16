//! AgentControl-owned bridge between runtime events and child-scoped memory.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::{AgentControlError, AgentControlErrorKind, PrincipalId};
use mnemosyne::{
    AgentMemoryContext, AgentMemoryVault, ChildMemoryDraft, MemoryAuthority, MemoryKind,
    MemoryPromotionReceipt, MemoryPromotionRequest, MemoryRecordId, MemoryScope,
};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use super::{AgentContextProjection, AgentEventSink, AgentRuntimeEvent};

pub fn context_projection_receipt(
    projection: &AgentContextProjection,
) -> Result<String, AgentControlError> {
    let encoded = serde_json::to_vec(projection).map_err(|error| AgentControlError {
        kind: AgentControlErrorKind::Persistence,
        message: error.to_string(),
    })?;
    Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
}

pub struct MemoryRecordingAgentEventSink {
    downstream: Arc<dyn AgentEventSink>,
    vault: Arc<AgentMemoryVault>,
    context: AgentMemoryContext,
    error: Mutex<Option<AgentControlError>>,
}

impl MemoryRecordingAgentEventSink {
    pub fn new(
        downstream: Arc<dyn AgentEventSink>,
        vault: Arc<AgentMemoryVault>,
        context: AgentMemoryContext,
    ) -> Self {
        Self {
            downstream,
            vault,
            context,
            error: Mutex::new(None),
        }
    }

    pub fn take_error(&self) -> Option<AgentControlError> {
        self.error.lock().take()
    }

    pub fn promote_reviewed(
        &self,
        source_record: MemoryRecordId,
        root_content: fabric::ContentId,
        broadcast: fabric::BroadcastEpoch,
        selected_candidate: fabric::ContentId,
        selection_receipt: String,
        reviewer: PrincipalId,
        review_receipt: String,
        target_scope: MemoryScope,
    ) -> Result<MemoryPromotionReceipt, AgentControlError> {
        self.vault
            .promote(&MemoryPromotionRequest {
                source_record,
                child: self.context.clone(),
                root_content,
                broadcast,
                selected_candidate,
                selection_receipt,
                reviewer,
                review_receipt,
                target_scope,
            })
            .map_err(memory_error)
    }

    fn record(&self, event: &AgentRuntimeEvent) -> Result<(), AgentControlError> {
        let (kind, content, event_id) = match event {
            AgentRuntimeEvent::Started { operation_id, .. } => (
                MemoryKind::Episodic,
                "child Agent runtime started".to_string(),
                format!("operation:{operation_id:?}:started"),
            ),
            AgentRuntimeEvent::Progress {
                operation_id,
                summary,
                ..
            } => (
                MemoryKind::Episodic,
                format!("child Agent progress: {summary}"),
                format!("operation:{operation_id:?}:progress"),
            ),
            AgentRuntimeEvent::Tool {
                operation_id,
                name,
                is_error,
                ..
            } => (
                MemoryKind::ToolOutcome,
                format!("child Agent tool {name} error={is_error}"),
                format!("operation:{operation_id:?}:tool:{name}"),
            ),
            AgentRuntimeEvent::Terminal {
                operation_id,
                status,
                result,
                ..
            } => (
                MemoryKind::GoalOutcome,
                result.as_ref().map_or_else(
                    || format!("child Agent ended {status:?}"),
                    |result| format!("child Agent ended {status:?}: {}", result.output),
                ),
                format!("operation:{operation_id:?}:terminal:{status:?}"),
            ),
        };
        self.vault
            .record_child(
                &self.context,
                ChildMemoryDraft {
                    kind,
                    content,
                    authority: MemoryAuthority::RawExperience,
                    source_event_ids: vec![event_id],
                    tags: vec!["agent-runtime-experience".into()],
                },
            )
            .map(|_| ())
            .map_err(memory_error)
    }
}

#[async_trait]
impl AgentEventSink for MemoryRecordingAgentEventSink {
    async fn emit(&self, event: AgentRuntimeEvent) {
        if self.error.lock().is_none() {
            if let Err(error) = self.record(&event) {
                *self.error.lock() = Some(error);
            }
        }
        self.downstream.emit(event).await;
    }
}

fn memory_error(error: anyhow::Error) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::Persistence,
        message: error.to_string(),
    }
}

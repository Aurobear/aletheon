//! AgentControl-owned bridge between runtime events and child-scoped memory.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::{AgentControlError, AgentControlErrorKind, PrincipalId};
use mnemosyne::{
    AgentMemoryContext, AgentMemoryVault, ChildMemoryDraft, ExperienceEvent, MemoryAuthority,
    MemoryKind, MemoryMetadata, MemoryPromotionReceipt, MemoryPromotionRequest, MemoryRecordId,
    MemoryScope, MemoryService,
};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use super::{AgentContextProjection, AgentEventSink, AgentRuntimeEvent};

/// Host-reviewed promotion input. Fields are private so an unreviewed child
/// draft cannot be promoted merely by calling a method with receipt-shaped
/// strings.
pub(crate) struct ReviewedChildMemoryDraft {
    source_record: MemoryRecordId,
    root_content: fabric::ContentId,
    broadcast: fabric::BroadcastEpoch,
    selected_candidate: fabric::ContentId,
    selection_receipt: String,
    reviewer: PrincipalId,
    review_receipt: String,
}

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
    durable: Option<Arc<dyn MemoryService>>,
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
            durable: None,
            error: Mutex::new(None),
        }
    }

    pub fn with_durable_memory(mut self, durable: Arc<dyn MemoryService>) -> Self {
        self.durable = Some(durable);
        self
    }

    pub fn take_error(&self) -> Option<AgentControlError> {
        self.error.lock().take()
    }

    #[allow(dead_code)]
    pub(crate) fn review_draft(
        &self,
        source_record: MemoryRecordId,
        root_content: fabric::ContentId,
        broadcast: fabric::BroadcastEpoch,
        selected_candidate: fabric::ContentId,
        selection_receipt: String,
        reviewer: PrincipalId,
        review_receipt: String,
    ) -> Result<ReviewedChildMemoryDraft, AgentControlError> {
        if selection_receipt.trim().is_empty()
            || reviewer.0.trim().is_empty()
            || review_receipt.trim().is_empty()
            || broadcast.0 == 0
        {
            return Err(memory_error(anyhow::anyhow!(
                "reviewed child draft requires root selection and reviewer receipts"
            )));
        }
        let source = self
            .vault
            .get_record(&source_record)
            .map_err(memory_error)?
            .ok_or_else(|| memory_error(anyhow::anyhow!("child memory draft does not exist")))?;
        if source.scope != self.context.task_scope
            || source.authority == MemoryAuthority::ApprovedCore
        {
            return Err(memory_error(anyhow::anyhow!(
                "memory draft is not an unpromoted child-scoped record"
            )));
        }
        Ok(ReviewedChildMemoryDraft {
            source_record,
            root_content,
            broadcast,
            selected_candidate,
            selection_receipt,
            reviewer,
            review_receipt,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn promote_reviewed(
        &self,
        reviewed: ReviewedChildMemoryDraft,
        target_scope: MemoryScope,
    ) -> Result<MemoryPromotionReceipt, AgentControlError> {
        self.vault
            .promote(&MemoryPromotionRequest {
                source_record: reviewed.source_record,
                child: self.context.clone(),
                root_content: reviewed.root_content,
                broadcast: reviewed.broadcast,
                selected_candidate: reviewed.selected_candidate,
                selection_receipt: reviewed.selection_receipt,
                reviewer: reviewed.reviewer,
                review_receipt: reviewed.review_receipt,
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

    fn durable_event(event: &AgentRuntimeEvent) -> Option<ExperienceEvent> {
        let AgentRuntimeEvent::Terminal {
            agent_id,
            operation_id,
            status,
            result,
            ..
        } = event
        else {
            return None;
        };
        let source_id = format!("operation:{}:terminal:{status:?}", operation_id.0);
        let content = result.as_ref().map_or_else(
            || format!("child Agent ended {status:?}"),
            |result| format!("child Agent ended {status:?}: {}", result.output),
        );
        Some(ExperienceEvent::GoalOutcome {
            goal_id: format!("agent:{}", agent_id.0),
            outcome: format!("{status:?}").to_ascii_lowercase(),
            content,
            metadata: MemoryMetadata::local(
                format!("agent-outcome:{}", operation_id.0),
                source_id,
                chrono::Utc::now(),
            ),
        })
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
        if self.error.lock().is_none() {
            if let (Some(durable), Some(event)) = (&self.durable, Self::durable_event(&event)) {
                if let Err(error) = durable.record(event).await {
                    *self.error.lock() = Some(memory_error(error));
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::{AgentId, AgentTaskId, OperationId, ProcessId};

    #[test]
    fn reviewed_gate_rejects_receiptless_child_draft() {
        let vault = Arc::new(AgentMemoryVault::in_memory().unwrap());
        let context = AgentMemoryContext::verified(
            ProcessId::new(),
            AgentId::new(),
            AgentTaskId("review-test".into()),
            "projection:review-test",
        )
        .unwrap();
        vault.register(&context).unwrap();
        let source = vault
            .record_child(
                &context,
                ChildMemoryDraft {
                    kind: MemoryKind::Reflection,
                    content: "draft".into(),
                    authority: MemoryAuthority::RawExperience,
                    source_event_ids: vec![format!("operation:{}", OperationId::new().0)],
                    tags: vec![],
                },
            )
            .unwrap();
        let sink = MemoryRecordingAgentEventSink::new(
            Arc::new(super::super::NoopAgentEventSink),
            vault,
            context,
        );
        assert!(sink
            .review_draft(
                source.id,
                fabric::ContentId::new(),
                fabric::BroadcastEpoch(1),
                fabric::ContentId::new(),
                "selection".into(),
                PrincipalId("reviewer".into()),
                String::new(),
            )
            .is_err());
    }

    #[test]
    fn terminal_event_becomes_durable_goal_outcome() {
        let agent_id = AgentId::new();
        let operation_id = OperationId::new();
        let event = AgentRuntimeEvent::Terminal {
            agent_id,
            process_id: ProcessId::new(),
            operation_id,
            status: fabric::AgentRunStatus::Succeeded,
            result: Some(fabric::AgentResult {
                output: "fixture passed".into(),
                usage: fabric::AttemptUsage::default(),
                evidence: vec![],
                artifacts: vec![],
            }),
        };

        let projected = MemoryRecordingAgentEventSink::durable_event(&event).unwrap();
        let ExperienceEvent::GoalOutcome {
            goal_id,
            outcome,
            content,
            metadata,
        } = projected
        else {
            panic!("terminal Agent event did not become a GoalOutcome");
        };
        assert_eq!(goal_id, format!("agent:{}", agent_id.0));
        assert_eq!(outcome, "succeeded");
        assert!(content.contains("fixture passed"));
        assert_eq!(
            metadata.record_id,
            format!("agent-outcome:{}", operation_id.0)
        );
        assert!(!metadata.record_id.contains("fixture passed"));
    }
}

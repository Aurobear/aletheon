use std::sync::Arc;

use async_trait::async_trait;
use fabric::{
    AgoraSpaceId, Clock, ConsciousProcessor, ProcessorContext, ProcessorHealth, ProcessorId,
    ProcessorResponse, VisibilityScope, WorkspaceAttribution, WorkspaceBroadcast, WorkspaceContent,
    WorkspaceReflection,
};

use super::{acknowledgements, salience, truncate, BoundedAdapter};

/// Sanitizing Agent adapter. Only already-selected, root-visible child evidence
/// enters this adapter; private mailbox/progress payloads are never copied.
pub struct AgentAdapter {
    adapter: BoundedAdapter,
}

impl AgentAdapter {
    pub fn new(space: &AgoraSpaceId, clock: Arc<dyn Clock>) -> Self {
        Self {
            adapter: BoundedAdapter::new(space, "agent", clock),
        }
    }
}

#[async_trait]
impl ConsciousProcessor for AgentAdapter {
    fn id(&self) -> ProcessorId {
        self.adapter.id.clone()
    }

    async fn on_broadcast(
        &self,
        broadcast: WorkspaceBroadcast,
        context: ProcessorContext,
    ) -> ProcessorResponse {
        let candidates = broadcast
            .selected
            .iter()
            .enumerate()
            .filter_map(|(index, selected)| {
                let child = match &selected.content {
                    WorkspaceContent::Observation(value) => match value.attribution {
                        WorkspaceAttribution::ChildAgent { process } => Some(process),
                        _ => None,
                    },
                    WorkspaceContent::AgentResult(_) | WorkspaceContent::Evidence(_) => {
                        (selected.source != context.agent_root).then_some(selected.source)
                    }
                    _ => None,
                }?;
                let summary = match &selected.content {
                    WorkspaceContent::AgentResult(value) => truncate(&value.output, 2048),
                    WorkspaceContent::Evidence(value) => truncate(&value.content, 2048),
                    WorkspaceContent::Observation(value) => truncate(&value.what, 2048),
                    _ => return None,
                };
                let mut refs = vec![
                    format!("child-process:{}", child.0),
                    format!("selected-child-evidence:{}", selected.id.0),
                ];
                refs.extend(
                    selected
                        .provenance
                        .source_refs
                        .iter()
                        .filter(|value| value.starts_with("promotion-receipt:"))
                        .cloned(),
                );
                Some(self.adapter.candidate(
                    &broadcast,
                    index,
                    WorkspaceContent::Reflection(WorkspaceReflection {
                        findings: vec![summary],
                        confidence: selected.confidence,
                    }),
                    salience(0.3, 0.8, selected.confidence),
                    VisibilityScope::AgentTree {
                        root: context.agent_root,
                    },
                    refs,
                ))
            })
            .take(context.max_candidates)
            .collect();
        ProcessorResponse {
            processor: self.id(),
            source_epoch: context.source_epoch,
            health: ProcessorHealth::Healthy,
            candidates,
            acknowledgements: acknowledgements(&broadcast),
            detail: None,
        }
    }
}

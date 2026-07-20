//! Read-only projection of durable conscious-core state.

use std::sync::Arc;

use agora::SqliteBroadcastStore;
use fabric::{
    AgoraSpaceId, CandidateDisposition, ConsciousCoreSnapshot, InspectorProcessorAck,
    ProcessorHealth, VisibilityScope, WorkspaceAttribution, WorkspaceContent,
};

pub struct ConsciousCoreInspector {
    store: Arc<SqliteBroadcastStore>,
}

impl ConsciousCoreInspector {
    pub fn new(store: Arc<SqliteBroadcastStore>) -> Self {
        Self { store }
    }

    pub fn snapshot(&self, space: &AgoraSpaceId) -> anyhow::Result<ConsciousCoreSnapshot> {
        let replay = self
            .store
            .replay(space)?
            .into_iter()
            .next_back()
            .ok_or_else(|| anyhow::anyhow!("conscious workspace has no durable broadcast"))?;
        let broadcast = replay.broadcast;
        let integration = self
            .store
            .integration(space, broadcast.epoch)?
            .ok_or_else(|| anyhow::anyhow!("conscious broadcast has no Dasein integration"))?;
        let dispositions = broadcast
            .selected
            .iter()
            .enumerate()
            .map(|(index, candidate)| CandidateDisposition {
                id: candidate.id,
                source_kind: attribution(candidate),
                content_schema: content_schema(&candidate.content),
                salience: candidate.salience,
                winner: index == 0,
                coalition_member: index > 0,
                visibility: match candidate.visibility {
                    VisibilityScope::Session => "session",
                    VisibilityScope::AgentTree { .. } => "agent_tree",
                    VisibilityScope::PrivateProcess { .. } => "private",
                }
                .into(),
            })
            .collect();
        let acknowledgements = self
            .store
            .processor_responses(space, broadcast.epoch)?
            .into_iter()
            .map(|response| InspectorProcessorAck {
                processor: response.processor,
                health: response.health,
                accepted_count: response
                    .acknowledgements
                    .iter()
                    .filter(|ack| ack.accepted)
                    .count(),
                rejected_count: response
                    .acknowledgements
                    .iter()
                    .filter(|ack| !ack.accepted)
                    .count(),
                degraded_reason: (response.health != ProcessorHealth::Healthy)
                    .then(|| sanitize_reason(response.detail.as_deref())),
            })
            .collect();
        let snapshot = ConsciousCoreSnapshot {
            space: space.clone(),
            epoch: broadcast.epoch,
            dispositions,
            acknowledgements,
            dasein_version: integration.transition.current_version,
            indicator_limitations: vec![
                "Functional indicators describe observable integration, not proof of consciousness."
                    .into(),
                "Hidden reasoning, secrets, private memory, and child mailbox content are excluded."
                    .into(),
            ],
        };
        snapshot.validate()?;
        Ok(snapshot)
    }
}

fn attribution(candidate: &fabric::WorkspaceCandidate) -> String {
    let value = match &candidate.content {
        WorkspaceContent::Observation(value) => Some(&value.attribution),
        WorkspaceContent::RecalledExperience(value) => Some(&value.attribution),
        WorkspaceContent::GovernedActionOutcome(value) => Some(&value.attribution),
        _ => None,
    };
    match value {
        Some(WorkspaceAttribution::User) => "user".into(),
        Some(WorkspaceAttribution::Environment) => "environment".into(),
        Some(WorkspaceAttribution::RootAgent { .. }) => "root_agent".into(),
        Some(WorkspaceAttribution::ChildAgent { .. }) => "child_agent".into(),
        Some(WorkspaceAttribution::ExternalMemory { .. }) => "external_memory".into(),
        Some(WorkspaceAttribution::Dasein) => "dasein".into(),
        Some(WorkspaceAttribution::Cognit) => "cognit".into(),
        Some(WorkspaceAttribution::Metacog) => "metacog".into(),
        Some(WorkspaceAttribution::Corpus) => "corpus".into(),
        None => "typed_domain_candidate".into(),
    }
}

fn content_schema(content: &WorkspaceContent) -> String {
    match content {
        WorkspaceContent::Observation(_) => "observation/v1",
        WorkspaceContent::RecalledExperience(_) => "recalled_experience/v1",
        WorkspaceContent::Evidence(_) => "evidence/v1",
        WorkspaceContent::Hypothesis(_) => "hypothesis/v1",
        WorkspaceContent::Prediction(_) => "prediction/v1",
        WorkspaceContent::PredictionError(_) => "prediction_error/v1",
        WorkspaceContent::Goal(_) => "goal/v1",
        WorkspaceContent::Concern(_) => "concern/v1",
        WorkspaceContent::CareConcern(_) => "care_concern/v1",
        WorkspaceContent::Plan(_) => "plan/v1",
        WorkspaceContent::ActionProposal(_) => "action_proposal/v1",
        WorkspaceContent::ToolOutcome(_) => "tool_outcome/v1",
        WorkspaceContent::GovernedActionOutcome(_) => "governed_action_outcome/v1",
        WorkspaceContent::AgentResult(_) => "agent_result/v1",
        WorkspaceContent::Reflection(_) => "reflection/v1",
        WorkspaceContent::Extension { schema, .. } => return schema.clone(),
    }
    .into()
}

fn sanitize_reason(value: Option<&str>) -> String {
    value
        .unwrap_or("processor degraded without a public reason")
        .chars()
        .take(1024)
        .collect()
}

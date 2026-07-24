use std::sync::Arc;

use async_trait::async_trait;
use fabric::{
    ActionProposalFrame, AgoraSpaceId, Clock, ConsciousProcessor, ProcessorContext,
    ProcessorHealth, ProcessorId, ProcessorResponse, VisibilityScope, WorkspaceBroadcast,
    WorkspaceContent,
};
use tokio::sync::Mutex;

use super::{acknowledgements, broadcast_summary, salience, truncate, BoundedAdapter};

/// Bounded Corpus proposal adapter. It never invokes a capability; E03 remains
/// behind `ConsciousActionBridge`, which requires a selected proposal and permit.
pub struct CorpusProcessor {
    adapter: BoundedAdapter,
    skills: Arc<Mutex<corpus::SkillLoader>>,
}

impl CorpusProcessor {
    pub fn new(
        space: &AgoraSpaceId,
        clock: Arc<dyn Clock>,
        skills: Arc<Mutex<corpus::SkillLoader>>,
    ) -> Self {
        Self {
            adapter: BoundedAdapter::new(space, "corpus", clock),
            skills,
        }
    }
}

#[async_trait]
impl ConsciousProcessor for CorpusProcessor {
    fn id(&self) -> ProcessorId {
        self.adapter.id.clone()
    }

    async fn on_broadcast(
        &self,
        broadcast: WorkspaceBroadcast,
        context: ProcessorContext,
    ) -> ProcessorResponse {
        let query = broadcast_summary(&broadcast);
        let loader = self.skills.lock().await;
        let keywords = loader
            .plugins()
            .iter()
            .filter(|plugin| !plugin.keywords.is_empty())
            .map(|plugin| corpus::skill::keyword_matcher::SkillKeywords {
                name: plugin.name.clone(),
                keywords: plugin.keywords.clone(),
                body: plugin.system_prompt.clone(),
            })
            .collect::<Vec<_>>();
        let candidates = corpus::skill::keyword_matcher::match_skills(&query, &keywords)
            .into_iter()
            .take(context.max_candidates.min(3))
            .enumerate()
            .map(|(index, matched)| {
                self.adapter.candidate(
                    &broadcast,
                    index,
                    WorkspaceContent::ActionProposal(ActionProposalFrame {
                        id: format!("corpus:{}:{index}", broadcast.epoch.0),
                        summary: format!(
                            "consider governed Corpus capability: {}",
                            truncate(&matched, 2048)
                        ),
                        risk: 0.5,
                    }),
                    salience(0.4, 0.7, 0.6),
                    VisibilityScope::Session,
                    vec!["execution-boundary:selected-and-permitted".into()],
                )
            })
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

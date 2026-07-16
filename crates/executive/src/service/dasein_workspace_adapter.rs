//! Dasein adapter for recurrent workspace modulation and integration.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::dasein::{
    DaseinOps, ExperienceProvenance, ExperienceSource, InterpretedExperience, SelfEventId,
    SelfTransitionRequest, Stimmung,
};
use fabric::{Clock, SalienceVector, StructuredSelfView, WorkspaceBroadcast, WorkspaceCandidate};

use super::conscious_core_ports::{DaseinIntegration, DaseinWorkspacePort};

const MAX_LIVED_SEMANTIC_BYTES: usize = 24 * 1024;

pub struct DaseinWorkspaceAdapter {
    dasein: Arc<dyn DaseinOps>,
    clock: Arc<dyn Clock>,
}

impl DaseinWorkspaceAdapter {
    pub fn new(dasein: Arc<dyn DaseinOps>, clock: Arc<dyn Clock>) -> Self {
        Self { dasein, clock }
    }

    fn snapshot_self_view(&self, version: fabric::dasein::SelfVersion) -> StructuredSelfView {
        let temporality = self.dasein.temporality_snapshot();
        let care = self.dasein.care_snapshot();
        StructuredSelfView {
            version,
            mood: self.dasein.mood(),
            concerns: care
                .concerns
                .into_iter()
                .take(fabric::MAX_SELF_VIEW_ITEMS)
                .map(|concern| concern.purpose)
                .collect(),
            projection: care.projection,
            protentions: temporality
                .protentions
                .into_iter()
                .take(fabric::MAX_SELF_VIEW_ITEMS)
                .map(|protention| protention.content)
                .collect(),
        }
    }
}

#[async_trait]
impl DaseinWorkspacePort for DaseinWorkspaceAdapter {
    async fn modulate_salience(
        &self,
        candidate: &WorkspaceCandidate,
    ) -> anyhow::Result<SalienceVector> {
        candidate.validate()?;
        let view = self.self_view().await?;
        let content = serde_json::to_string(&candidate.content)?.to_ascii_lowercase();
        let concern_match = view
            .concerns
            .iter()
            .any(|concern| contains_meaningful_term(&content, concern));
        let projection_match = view
            .projection
            .as_ref()
            .is_some_and(|projection| contains_meaningful_term(&content, projection));
        let protention_match = view
            .protentions
            .iter()
            .any(|protention| contains_meaningful_term(&content, protention));
        let mut salience = candidate.salience;
        if concern_match {
            salience.self_relevance = (salience.self_relevance + 0.3).min(1.0);
            salience.urgency = (salience.urgency + 0.15).min(1.0);
        }
        if projection_match {
            salience.goal_relevance = (salience.goal_relevance + 0.35).min(1.0);
        }
        if protention_match {
            salience.prediction_error = (salience.prediction_error + 0.2).min(1.0);
        }
        match view.mood {
            Stimmung::Angst { .. } | Stimmung::Geknickt { .. } => {
                salience.urgency = (salience.urgency + 0.1).min(1.0);
            }
            Stimmung::Neugier { .. } => {
                salience.novelty = (salience.novelty + 0.1).min(1.0);
            }
            _ => {}
        }
        salience.validate()?;
        Ok(salience)
    }

    async fn integrate_broadcast(
        &self,
        broadcast: &WorkspaceBroadcast,
    ) -> anyhow::Result<DaseinIntegration> {
        broadcast.validate()?;
        let previous = self.dasein.self_version().await;
        anyhow::ensure!(
            previous == broadcast.dasein_version,
            "broadcast Dasein version is stale"
        );
        let checksum = broadcast.checksum()?;
        let event_id = SelfEventId(uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_OID,
            format!("{}:{}:{checksum}", broadcast.space.0, broadcast.epoch.0).as_bytes(),
        ));
        let selected_data = serde_json::to_string(&broadcast.contents)?;
        let semantic = truncate_utf8(
            &format!(
                "selected workspace data at {} epoch {}: {}",
                broadcast.space.0, broadcast.epoch.0, selected_data
            ),
            MAX_LIVED_SEMANTIC_BYTES,
        );
        let transition = self
            .dasein
            .transition(SelfTransitionRequest {
                event_id,
                source: ExperienceSource::Agora,
                observed_at: self.clock.wall_now(),
                content: InterpretedExperience::Lived {
                    semantic,
                    action: None,
                    perception: Some(format!(
                        "workspace broadcast {}:{}",
                        broadcast.space.0, broadcast.epoch.0
                    )),
                },
                provenance: ExperienceProvenance {
                    producer: "conscious-core".into(),
                    session_id: None,
                    turn_id: None,
                    source_ref: Some(format!(
                        "broadcast:{}:{}",
                        broadcast.space.0, broadcast.epoch.0
                    )),
                },
                expected_version: previous,
            })
            .await?;
        let self_view = self.snapshot_self_view(transition.current_version);
        self_view.validate()?;
        Ok(DaseinIntegration {
            transition,
            self_view,
        })
    }

    async fn self_view(&self) -> anyhow::Result<StructuredSelfView> {
        let view = self.snapshot_self_view(self.dasein.self_version().await);
        view.validate()?;
        Ok(view)
    }
}

fn contains_meaningful_term(content: &str, projection: &str) -> bool {
    projection
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.chars().count() >= 3)
        .map(str::to_ascii_lowercase)
        .any(|term| content.contains(&term))
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

//! Dasein adapter for recurrent workspace modulation and integration.

use std::sync::Arc;

use ::contracts::dasein::{
    ExperienceProvenance, ExperienceSource, InterpretedExperience, OutcomeStatus, SelfEventId,
    SelfTransitionRequest, Stimmung,
};
use ::contracts::{
    CareConcernFrame, Clock, SalienceVector, StructuredSelfView, WorkspaceBroadcast,
    WorkspaceCandidate,
};
use async_trait::async_trait;
use dasein::DaseinOps;

use agora::conscious_core_ports::{DaseinIntegration, DaseinWorkspacePort};

const MAX_LIVED_SEMANTIC_BYTES: usize = 24 * 1024;
const MAX_GROUNDED_OUTCOME_BYTES: usize = 24 * 1024;
const MAX_GROUNDED_OUTCOME_VERSION_RETRIES: usize = 3;
const MAX_WORKSPACE_INTEGRATION_VERSION_RETRIES: usize = 3;
const EVALUATION_DASEIN_EVENT_NAMESPACE: uuid::Uuid =
    uuid::Uuid::from_u128(0x91265e16_0709_45a0_a34d_0913240eef82);

/// Projects settled evaluation confidence into Dasein's lived-experience
/// ledger. It deliberately has no authority to mutate the receipt decision.
pub struct DaseinEvaluationProjectionSink {
    dasein: Arc<dyn DaseinOps>,
    clock: Arc<dyn Clock>,
}

impl DaseinEvaluationProjectionSink {
    pub fn new(dasein: Arc<dyn DaseinOps>, clock: Arc<dyn Clock>) -> Self {
        Self { dasein, clock }
    }
}

#[async_trait]
impl application::evaluation_projection::EvaluationProjectionSink
    for DaseinEvaluationProjectionSink
{
    fn name(&self) -> &'static str {
        "dasein"
    }

    async fn project(
        &self,
        record: &application::evaluation_projection::EvaluationProjectionRecord,
    ) -> anyhow::Result<()> {
        let status = match record.receipt.decision {
            ::contracts::EvaluationDecision::ObservedPass
            | ::contracts::EvaluationDecision::Accepted => OutcomeStatus::Succeeded,
            ::contracts::EvaluationDecision::ObservedFail
            | ::contracts::EvaluationDecision::Rejected => OutcomeStatus::Failed,
            ::contracts::EvaluationDecision::Indeterminate => OutcomeStatus::Cancelled,
        };
        let receipt_id = record.receipt.receipt_id.0;
        let event_id = SelfEventId(uuid::Uuid::new_v5(
            &EVALUATION_DASEIN_EVENT_NAMESPACE,
            receipt_id.as_bytes(),
        ));
        let summary = truncate_utf8(
            &serde_json::to_string(&serde_json::json!({
                "receipt_id": receipt_id,
                "decision": record.receipt.decision,
                "score_millis": record.receipt.weighted_total_millis,
                "coverage_millis": record.receipt.evidence_coverage_millis,
                "confidence_millis": record.receipt.confidence_millis,
                "failed_gates": record.receipt.failed_gates,
            }))?,
            MAX_GROUNDED_OUTCOME_BYTES,
        );
        for attempt in 0..MAX_GROUNDED_OUTCOME_VERSION_RETRIES {
            let expected_version = self.dasein.self_version().await;
            let result = self
                .dasein
                .transition(SelfTransitionRequest {
                    event_id,
                    source: ExperienceSource::Metacog,
                    observed_at: self.clock.wall_now(),
                    content: InterpretedExperience::Outcome {
                        summary: summary.clone(),
                        status,
                    },
                    provenance: ExperienceProvenance {
                        producer: "executive-evaluation-projection".into(),
                        session_id: uuid::Uuid::parse_str(&record.context.session_id).ok(),
                        turn_id: uuid::Uuid::parse_str(&record.receipt.subject_id).ok(),
                        source_ref: Some(format!("evaluation-receipt:{receipt_id}")),
                    },
                    expected_version,
                })
                .await;
            match result {
                Ok(_) => return Ok(()),
                Err(error)
                    if attempt + 1 < MAX_GROUNDED_OUTCOME_VERSION_RETRIES
                        && error.to_string().contains("version conflict") =>
                {
                    tokio::task::yield_now().await;
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("bounded Dasein evaluation retry loop always returns")
    }
}

pub struct DaseinWorkspaceAdapter {
    dasein: Arc<dyn DaseinOps>,
    clock: Arc<dyn Clock>,
}

impl DaseinWorkspaceAdapter {
    pub fn new(dasein: Arc<dyn DaseinOps>, clock: Arc<dyn Clock>) -> Self {
        Self { dasein, clock }
    }

    fn snapshot_self_view(&self, version: ::contracts::dasein::SelfVersion) -> StructuredSelfView {
        let temporality = self.dasein.temporality_snapshot();
        let care = self.dasein.care_snapshot();
        let care_concerns = care
            .concerns
            .into_iter()
            .take(::contracts::MAX_SELF_VIEW_ITEMS)
            .map(|concern| CareConcernFrame {
                purpose: concern.purpose,
                urgency: concern.urgency as f32,
            })
            .collect::<Vec<_>>();
        StructuredSelfView {
            version,
            mood: self.dasein.mood(),
            concerns: care_concerns
                .iter()
                .map(|concern| concern.purpose.clone())
                .collect(),
            care_concerns,
            projection: care.projection,
            protentions: temporality
                .protentions
                .into_iter()
                .take(::contracts::MAX_SELF_VIEW_ITEMS)
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
        let mut transition = None;
        for attempt in 0..MAX_WORKSPACE_INTEGRATION_VERSION_RETRIES {
            // The broadcast records the self version used for competition. Other
            // sessions may legitimately advance Dasein before this integration
            // is committed, so integrate against the current canonical head and
            // retry a bounded optimistic conflict rather than failing the turn.
            let expected_version = self.dasein.self_version().await;
            let result = self
                .dasein
                .transition(SelfTransitionRequest {
                    event_id,
                    source: ExperienceSource::Agora,
                    observed_at: self.clock.wall_now(),
                    content: InterpretedExperience::Lived {
                        semantic: semantic.clone(),
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
                    expected_version,
                })
                .await;
            match result {
                Ok(receipt) => {
                    transition = Some(receipt);
                    break;
                }
                Err(error)
                    if attempt + 1 < MAX_WORKSPACE_INTEGRATION_VERSION_RETRIES
                        && error.to_string().contains("version conflict") =>
                {
                    tokio::task::yield_now().await;
                }
                Err(error) => return Err(error),
            }
        }
        let transition =
            transition.expect("bounded Dasein workspace integration retry loop always resolves");
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

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::{
        AgoraSpaceId, BroadcastEpoch, ContentId, MonoTime, ProcessId, SelectionExplanation,
        SelectionResult, VisibilityScope, WallTime, WorkspaceAttribution, WorkspaceCandidate,
        WorkspaceContent, WorkspaceObservation, WorkspaceProvenance, WORKSPACE_SCHEMA_V1,
    };
    #[tokio::test]
    async fn workspace_integration_accepts_an_intervening_dasein_transition() {
        let clock = Arc::new(kernel::chronos::TestClock::default());
        let dasein = Arc::new(dasein::dasein::DaseinModule::new(clock.clone()).0);
        let adapter = DaseinWorkspaceAdapter::new(dasein.clone(), clock);
        let source = ProcessId::new();
        let candidate = WorkspaceCandidate {
            schema_version: WORKSPACE_SCHEMA_V1,
            id: ContentId(uuid::Uuid::new_v4()),
            space: AgoraSpaceId("concurrent-session".into()),
            source,
            turn: None,
            content: WorkspaceContent::Observation(WorkspaceObservation {
                what: "current input".into(),
                source: "test".into(),
                data: serde_json::json!({"kind": "input"}),
                attribution: WorkspaceAttribution::User,
            }),
            confidence: 1.0,
            salience: SalienceVector {
                urgency: 1.0,
                goal_relevance: 1.0,
                self_relevance: 1.0,
                novelty: 1.0,
                confidence: 1.0,
                prediction_error: 0.0,
                affect_intensity: 0.0,
                social_relevance: 0.0,
            },
            provenance: WorkspaceProvenance {
                producer: source,
                operation: None,
                source_refs: vec!["test://input".into()],
                observed_at: WallTime(1),
            },
            visibility: VisibilityScope::Session,
            dependencies: vec![],
            created_at: MonoTime(1),
            expires_at: None,
        };
        let selection = SelectionResult {
            selected: vec![candidate.clone()],
            explanation: SelectionExplanation {
                policy_version: 1,
                evaluated: vec![],
                selected_ids: vec![candidate.id],
                rejected_below_ignition: vec![],
            },
        };
        let broadcast = WorkspaceBroadcast::from_selection(
            BroadcastEpoch(1),
            selection,
            ::contracts::dasein::SelfVersion(0),
            1,
        )
        .unwrap();

        dasein
            .record_outcome("prior turn", OutcomeStatus::Succeeded, "test")
            .await
            .unwrap();
        let integration = adapter.integrate_broadcast(&broadcast).await.unwrap();

        assert_eq!(integration.transition.previous_version.0, 1);
        assert_eq!(integration.transition.current_version.0, 2);
    }
}

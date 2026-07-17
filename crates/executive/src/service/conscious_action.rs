//! Bridge between governed capability execution and workspace recurrence.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use fabric::dasein::{CareActionKind, SelfSignal};
use fabric::{
    ActionProposalFrame, CapabilityCall, CapabilityResult, Clock, ConsciousContextProjection,
    ContentId, GovernedActionOutcomeFrame, MonoDeadline, ProcessId, RiskLevel, SalienceVector,
    VisibilityScope, WorkspaceAttribution, WorkspaceBroadcast, WorkspaceCandidate, WorkspaceContent,
    WorkspaceProvenance, WORKSPACE_SCHEMA_V1,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::conscious_core_coordinator::ConsciousCoreCoordinator;
use super::conscious_core_ports::{
    CandidateAdmissionStatus, CandidateCause, CandidateSubmission, ConsciousCandidatePort,
    LatestConsciousContextPort,
};
use super::conscious_modulation::{recommend, ModulationMode, ModulationReceipt};
use super::governed_capability::{
    GovernedActionLoop, SelectedActionContext, SelectedActionOutcomeReceipt,
};

const ACTION_NAMESPACE: Uuid = Uuid::from_u128(0xd166_5f2f_f9d1_4e89_b0cf_9867_4c09_3101);

/// Projects model-proposed actions and their governed outcomes through Agora.
///
/// It never mutates Dasein directly: only a selected broadcast reaches the
/// coordinator's lived-transition boundary.
pub struct ConsciousActionBridge {
    coordinator: Arc<ConsciousCoreCoordinator>,
    source: ProcessId,
    outcome_source: ProcessId,
    root: ProcessId,
    clock: Arc<dyn Clock>,
    candidate_ttl: Duration,
}

impl ConsciousActionBridge {
    pub fn new(
        coordinator: Arc<ConsciousCoreCoordinator>,
        source: ProcessId,
        root: ProcessId,
        clock: Arc<dyn Clock>,
        candidate_ttl: Duration,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(!candidate_ttl.is_zero(), "action candidate TTL is zero");
        Ok(Self {
            coordinator,
            source,
            outcome_source: ProcessId(Uuid::new_v5(
                &ACTION_NAMESPACE,
                format!("outcome-source:{}", source.0).as_bytes(),
            )),
            root,
            clock,
            candidate_ttl,
        })
    }

    fn attribution(&self) -> WorkspaceAttribution {
        if self.source == self.root {
            WorkspaceAttribution::RootAgent {
                process: self.source,
            }
        } else {
            WorkspaceAttribution::ChildAgent {
                process: self.source,
            }
        }
    }

    fn action_id(call: &CapabilityCall) -> ContentId {
        ContentId(Uuid::new_v5(
            &ACTION_NAMESPACE,
            format!("{}:{}:{}", call.operation_id.0, call.call_id, call.name).as_bytes(),
        ))
    }

    fn outcome_id(action: ContentId, permit_id: &str) -> ContentId {
        ContentId(Uuid::new_v5(
            &ACTION_NAMESPACE,
            format!("outcome:{}:{permit_id}", action.0).as_bytes(),
        ))
    }

    fn max_salience() -> SalienceVector {
        SalienceVector {
            urgency: 1.0,
            goal_relevance: 1.0,
            self_relevance: 1.0,
            novelty: 1.0,
            confidence: 1.0,
            prediction_error: 1.0,
            affect_intensity: 0.5,
            social_relevance: 0.5,
        }
    }

    fn expiry(&self, now: fabric::MonoTime) -> MonoDeadline {
        MonoDeadline::after(
            now,
            self.candidate_ttl.as_millis().min(u128::from(u64::MAX)) as u64,
        )
    }

    /// R3a ObserveOnly: record what the conscious core would recommend for this
    /// governed action (proceed / reorder / defer / veto) and emit a structured
    /// receipt plus metric. Best-effort and infallible — it never affects whether
    /// or how the action executes.
    async fn observe_modulation(
        &self,
        call: &CapabilityCall,
        risk: RiskLevel,
        broadcast: &WorkspaceBroadcast,
        action_id: ContentId,
    ) {
        let care_decision = self
            .coordinator
            .latest_context(self.coordinator.space())
            .await
            .ok()
            .as_ref()
            .and_then(latest_care_decision);
        let (self_relevance, urgency) = dasein_intensity(broadcast, action_id);
        let (recommendation, rationale) =
            recommend(risk, care_decision, self_relevance, urgency);
        let receipt = ModulationReceipt {
            mode: ModulationMode::ObserveOnly,
            tool: call.name.clone(),
            call_id: call.call_id.clone(),
            risk_prior: risk,
            care_decision,
            self_relevance,
            urgency,
            recommendation,
            rationale,
        };
        tracing::info!(
            target: "conscious.modulation",
            tool = %receipt.tool,
            call_id = %receipt.call_id,
            risk_prior = ?receipt.risk_prior,
            care = ?receipt.care_decision,
            self_relevance = receipt.self_relevance,
            urgency = receipt.urgency,
            recommendation = ?receipt.recommendation,
            modulating = receipt.recommendation.is_modulating(),
            rationale = %receipt.rationale,
            "conscious modulation (observe-only)"
        );
    }
}

/// Extract the most recent Dasein care decision expressed in the latest
/// broadcast, if any. `CareDecision` signals flow into the workspace as
/// `Concern` content (conscious-core plan R1).
fn latest_care_decision(projection: &ConsciousContextProjection) -> Option<CareActionKind> {
    let broadcast = projection.latest_broadcast.as_ref()?;
    broadcast.contents.iter().rev().find_map(|content| match content {
        WorkspaceContent::Concern(SelfSignal::CareDecision { action, .. }) => Some(*action),
        _ => None,
    })
}

/// Aggregate the current conscious state's intensity from the selected
/// candidates other than the action itself: the maximum self-relevance and
/// urgency across the ignited Dasein concerns. Returns `(0.0, 0.0)` when no
/// other content is present.
fn dasein_intensity(broadcast: &WorkspaceBroadcast, action_id: ContentId) -> (f32, f32) {
    broadcast
        .selected
        .iter()
        .filter(|candidate| candidate.id != action_id)
        .fold((0.0_f32, 0.0_f32), |(self_relevance, urgency), candidate| {
            (
                self_relevance.max(candidate.salience.self_relevance),
                urgency.max(candidate.salience.urgency),
            )
        })
}

#[async_trait]
impl GovernedActionLoop for ConsciousActionBridge {
    async fn select_action(
        &self,
        call: &CapabilityCall,
        risk: RiskLevel,
    ) -> anyhow::Result<SelectedActionContext> {
        let now = self.clock.mono_now();
        let event_ref = format!("action-call:{}:{}", call.operation_id.0, call.call_id);
        let candidate = WorkspaceCandidate {
            schema_version: WORKSPACE_SCHEMA_V1,
            id: Self::action_id(call),
            space: self.coordinator.space().clone(),
            source: self.source,
            turn: None,
            content: WorkspaceContent::ActionProposal(ActionProposalFrame {
                id: call.call_id.clone(),
                summary: format!("invoke governed capability {}", call.name),
                risk: 0.5,
            }),
            confidence: 1.0,
            salience: Self::max_salience(),
            provenance: WorkspaceProvenance {
                producer: self.source,
                operation: Some(call.operation_id),
                source_refs: vec![event_ref, format!("capability-name:{}", call.name)],
                observed_at: self.clock.wall_now(),
            },
            visibility: VisibilityScope::Session,
            dependencies: vec![],
            created_at: now,
            expires_at: Some(self.expiry(now)),
        };
        let candidate_id = candidate.id;
        let receipt = self
            .coordinator
            .submit_candidate(CandidateSubmission {
                candidate,
                cause: CandidateCause::GovernedActionProposal {
                    operation_id: call.operation_id,
                    call_id: call.call_id.clone(),
                },
            })
            .await?;
        anyhow::ensure!(
            matches!(
                receipt.status,
                CandidateAdmissionStatus::Accepted | CandidateAdmissionStatus::Duplicate
            ),
            "governed action was not admitted: {:?}",
            receipt.status
        );
        let cycle = self.coordinator.run_cycle(self.source, 0).await?;
        let broadcast = cycle.broadcast.context("governed action did not ignite")?;
        anyhow::ensure!(
            broadcast.winner_ids.contains(&candidate_id),
            "governed action was not selected"
        );
        // R3a (ObserveOnly): observe what the conscious core would recommend for
        // this action and record a structured receipt + metric. This never
        // changes execution — the action proceeds exactly as before. Real
        // defer/reorder (R3b) is gated on R2 and a validated distribution.
        self.observe_modulation(call, risk, &broadcast, candidate_id)
            .await;
        Ok(SelectedActionContext {
            candidate_id,
            broadcast_epoch: broadcast.epoch,
            operation_id: call.operation_id,
            source_process: self.source,
            attribution: self.attribution(),
        })
    }

    async fn observe_outcome(
        &self,
        selected: &SelectedActionContext,
        call: &CapabilityCall,
        result: &CapabilityResult,
    ) -> anyhow::Result<SelectedActionOutcomeReceipt> {
        let permit_id = result.usage.permit_id.0.to_string();
        anyhow::ensure!(
            result.usage.permit_id.0 != Uuid::nil(),
            "governed capability result has no permit identity"
        );
        anyhow::ensure!(
            selected.operation_id == call.operation_id,
            "selected action operation changed"
        );
        anyhow::ensure!(
            selected.source_process == call.process_id,
            "selected action process changed"
        );
        anyhow::ensure!(
            selected.source_process == self.source,
            "selected action belongs to another process"
        );
        anyhow::ensure!(
            selected.attribution == self.attribution(),
            "selected action attribution changed"
        );
        let durable = self
            .coordinator
            .durable_selected_candidate(selected.broadcast_epoch, selected.candidate_id)?
            .context("selected action is not a durable winner")?;
        anyhow::ensure!(
            durable.space == *self.coordinator.space(),
            "selected action belongs to another workspace"
        );
        anyhow::ensure!(
            durable.source == self.source
                && durable.provenance.producer == self.source
                && durable.provenance.operation == Some(call.operation_id),
            "selected action authority provenance changed"
        );
        anyhow::ensure!(
            matches!(
                &durable.content,
                WorkspaceContent::ActionProposal(frame)
                    if frame.id == call.call_id
                        && frame.summary == format!("invoke governed capability {}", call.name)
            ),
            "selected winner is not the requested governed action"
        );
        let output_ref = format!("sha256:{:x}", Sha256::digest(result.output.as_bytes()));
        let now = self.clock.mono_now();
        let cause_ref = format!("capability:{permit_id}:{}", selected.operation_id.0);
        let mut source_refs = vec![
            cause_ref,
            format!("selected-action:{}", selected.candidate_id.0),
            format!("broadcast-epoch:{}", selected.broadcast_epoch.0),
        ];
        if let Some(audit_id) = result.audit_id {
            source_refs.push(format!("audit:{}", audit_id.0));
        }
        let candidate = WorkspaceCandidate {
            schema_version: WORKSPACE_SCHEMA_V1,
            id: Self::outcome_id(selected.candidate_id, &permit_id),
            space: self.coordinator.space().clone(),
            source: self.outcome_source,
            turn: None,
            content: WorkspaceContent::GovernedActionOutcome(GovernedActionOutcomeFrame {
                action_id: selected.candidate_id,
                permit_id: permit_id.clone(),
                operation: selected.operation_id,
                output_ref,
                is_error: result.is_error,
                attribution: selected.attribution.clone(),
            }),
            confidence: 1.0,
            salience: Self::max_salience(),
            provenance: WorkspaceProvenance {
                producer: self.outcome_source,
                operation: Some(selected.operation_id),
                source_refs,
                observed_at: self.clock.wall_now(),
            },
            visibility: VisibilityScope::Session,
            dependencies: vec![selected.candidate_id],
            created_at: now,
            expires_at: Some(self.expiry(now)),
        };
        let outcome_id = candidate.id;
        let admission = self
            .coordinator
            .submit_candidate(CandidateSubmission {
                candidate,
                cause: CandidateCause::GovernedActionOutcome {
                    permit_id: permit_id.clone(),
                    operation_id: selected.operation_id,
                },
            })
            .await?;
        anyhow::ensure!(
            matches!(
                admission.status,
                CandidateAdmissionStatus::Accepted | CandidateAdmissionStatus::Duplicate
            ),
            "governed outcome was not admitted: {:?}",
            admission.status
        );
        let cycle = self.coordinator.run_cycle(self.source, 1).await?;
        let broadcast = cycle.broadcast.context("governed outcome did not ignite")?;
        anyhow::ensure!(
            broadcast.winner_ids.contains(&outcome_id),
            "governed outcome was not selected"
        );
        Ok(SelectedActionOutcomeReceipt {
            outcome_id,
            permit_id,
            broadcast_epoch: broadcast.epoch,
        })
    }
}

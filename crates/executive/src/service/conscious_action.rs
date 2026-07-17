//! Bridge between governed capability execution and workspace recurrence.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use fabric::{
    ActionProposalFrame, CapabilityCall, CapabilityResult, Clock, ConsciousFieldReadout,
    ConsciousTraceEvent, ContentId, FieldDecisionKind, FieldDecisionReason,
    GovernedActionOutcomeFrame, LatestConsciousContextPort, MonoDeadline, ProcessId,
    SalienceVector, VisibilityScope, WorkspaceAttribution, WorkspaceCandidate, WorkspaceContent,
    WorkspaceProvenance, WORKSPACE_SCHEMA_V1,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::conscious_core_coordinator::ConsciousCoreCoordinator;
use super::conscious_core_ports::{
    CandidateAdmissionStatus, CandidateCause, CandidateSubmission, ConsciousCandidatePort,
};
use super::conscious_field::{proposal_salience, should_defer};
use super::governed_capability::{
    ActionModulationSnapshot, GovernedActionDecision, GovernedActionLoop, SelectedActionContext,
    SelectedActionOutcomeReceipt,
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
    arbitration_mode: fabric::ConsciousArbitrationMode,
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
            arbitration_mode: fabric::ConsciousArbitrationMode::Observe,
        })
    }

    pub fn with_arbitration_mode(mut self, mode: fabric::ConsciousArbitrationMode) -> Self {
        self.arbitration_mode = mode;
        self
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

    fn legacy_salience() -> SalienceVector {
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

    fn metric_ref(&self, epoch: fabric::BroadcastEpoch) -> String {
        self.coordinator
            .field_metric_snapshots()
            .into_iter()
            .rev()
            .find(|snapshot| snapshot.broadcast_epoch == epoch.0)
            .map(|snapshot| snapshot.trace_event_id)
            .filter(|reference| !reference.trim().is_empty())
            .unwrap_or_else(|| format!("broadcast:{}:{}", self.coordinator.space().0, epoch.0))
    }
}

#[async_trait]
impl GovernedActionLoop for ConsciousActionBridge {
    fn arbitration_mode(&self) -> fabric::ConsciousArbitrationMode {
        self.arbitration_mode
    }

    async fn select_action(&self, call: &CapabilityCall) -> anyhow::Result<GovernedActionDecision> {
        let readout = match self
            .coordinator
            .latest_context(self.coordinator.space())
            .await
        {
            Ok(projection) => ConsciousFieldReadout::from_projection(&projection)
                .ok()
                .flatten(),
            Err(error) => {
                tracing::warn!(error = %error, "conscious action field read failed; using legacy proposal");
                None
            }
        };
        let (confidence, salience) = readout
            .as_ref()
            .map(proposal_salience)
            .unwrap_or((1.0, Self::legacy_salience()));
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
            confidence,
            salience,
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
        let selected = cycle
            .broadcast
            .as_ref()
            .is_some_and(|broadcast| broadcast.winner_ids.contains(&candidate_id));
        let Some(readout) = readout else {
            let broadcast = cycle.broadcast.context("governed action did not ignite")?;
            anyhow::ensure!(selected, "governed action was not selected");
            return Ok(GovernedActionDecision::Proceed {
                selected: SelectedActionContext {
                    candidate_id,
                    broadcast_epoch: broadcast.epoch,
                    operation_id: call.operation_id,
                    source_process: self.source,
                    attribution: self.attribution(),
                },
                modulation: None,
            });
        };
        let reason = should_defer(&readout, selected);
        let modulation = ActionModulationSnapshot {
            decision: if reason.is_some() {
                FieldDecisionKind::Defer
            } else {
                FieldDecisionKind::Proceed
            },
            reason: reason.unwrap_or(FieldDecisionReason::Selected),
            broadcast_epoch: readout.epoch,
            confidence,
            salience,
            metric_ref: self.metric_ref(readout.epoch),
        };
        modulation.validate()?;
        if let Some(reason) = reason {
            Ok(GovernedActionDecision::Defer {
                reason,
                retryable: true,
                modulation,
            })
        } else {
            let broadcast = cycle
                .broadcast
                .context("selected governed action has no broadcast")?;
            Ok(GovernedActionDecision::Proceed {
                selected: SelectedActionContext {
                    candidate_id,
                    broadcast_epoch: broadcast.epoch,
                    operation_id: call.operation_id,
                    source_process: self.source,
                    attribution: self.attribution(),
                },
                modulation: Some(modulation),
            })
        }
    }

    async fn observe_modulation(
        &self,
        mode: fabric::ConsciousArbitrationMode,
        call: &CapabilityCall,
        modulation: &ActionModulationSnapshot,
    ) -> anyhow::Result<()> {
        modulation.validate()?;
        let event = ConsciousTraceEvent::FieldModulation {
            mode,
            decision: modulation.decision,
            reason: modulation.reason,
            operation_id: call.operation_id.0.to_string(),
            call_id: call.call_id.clone(),
            broadcast_epoch: Some(modulation.broadcast_epoch.0),
            baseline: None,
            effective: Some(f64::from(modulation.confidence)),
            delta: None,
            metric_ref: modulation.metric_ref.clone(),
        };
        self.coordinator.record_field_modulation(&event)?;
        tracing::info!(
            mode = ?mode,
            decision = ?modulation.decision,
            reason = ?modulation.reason,
            epoch = modulation.broadcast_epoch.0,
            metric_ref = %modulation.metric_ref,
            "conscious action modulation observed"
        );
        Ok(())
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
            salience: Self::legacy_salience(),
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

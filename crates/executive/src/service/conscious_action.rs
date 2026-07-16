//! Bridge between governed capability execution and workspace recurrence.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use fabric::{
    ActionProposalFrame, CapabilityCall, CapabilityResult, Clock, ContentId,
    GovernedActionOutcomeFrame, MonoDeadline, ProcessId, SalienceVector, VisibilityScope,
    WorkspaceAttribution, WorkspaceCandidate, WorkspaceContent, WorkspaceProvenance,
    WORKSPACE_SCHEMA_V1,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::conscious_core_coordinator::ConsciousCoreCoordinator;
use super::conscious_core_ports::{
    CandidateAdmissionStatus, CandidateCause, CandidateSubmission, ConsciousCandidatePort,
};
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
}

#[async_trait]
impl GovernedActionLoop for ConsciousActionBridge {
    async fn select_action(&self, call: &CapabilityCall) -> anyhow::Result<SelectedActionContext> {
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
            source: self.source,
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
                producer: self.source,
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

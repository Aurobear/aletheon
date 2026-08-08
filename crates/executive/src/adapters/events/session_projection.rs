use std::collections::BTreeMap;

use fabric::types::episode_report::{EpisodeReport, EpisodeSettlement, SettledEpisodeReport};
use fabric::types::outcome_verification::VerificationDecision;
use fabric::types::robot_failure::RobotFailureClass;
use fabric::{
    ActivityKind, ActivitySnapshot, ActivityState, EventPayload, EventVisibility, ItemPayload,
    ItemRecord, SessionForkedEvent, SessionId, SessionPrincipalBoundEvent, SessionRecord,
    SessionStatus, SpineEvent, TaskPhase, TaskRuntimeFacts, TaskSettlement, TaskSnapshot,
    TaskStepSnapshot, SESSION_SCHEMA_VERSION,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::adapters::session::projection_store::SessionProjectionStore;
use crate::application::event_projection::{
    EventProjection, ProjectionDescriptor, ProjectionError,
};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PublicSessionState {
    pub sessions: BTreeMap<String, PublicSessionView>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PublicSessionView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<SessionRecord>,
    pub turns: BTreeMap<String, Vec<u64>>,
    pub items: Vec<ItemRecord>,
}

pub struct SessionProjection;

impl SessionProjection {
    /// Fold the public Session history into the daemon-owned Task/Activity
    /// read model. The function is deliberately pure: replaying an identical
    /// ordered item prefix produces byte-equivalent JSON without consulting
    /// process-local state.
    pub fn read_model(
        session: &SessionRecord,
        items: &[ItemRecord],
    ) -> (Vec<TaskSnapshot>, Vec<ActivitySnapshot>) {
        let task_id = format!("session:{}:task", session.id.0);
        let goal = items.iter().find_map(|item| match &item.payload {
            ItemPayload::UserMessage { content } => Some(content.clone()),
            _ => None,
        });
        let mut turns = BTreeMap::<String, Vec<&ItemRecord>>::new();
        for item in items {
            turns
                .entry(item.turn_id.0.to_string())
                .or_default()
                .push(item);
        }

        let mut steps = turns
            .values()
            .filter_map(|turn_items| {
                let first = turn_items.first()?;
                let last = turn_items.last()?;
                Some(TaskStepSnapshot {
                    step_id: format!("turn:{}", first.turn_id.0),
                    turn_id: first.turn_id,
                    phase: turn_phase(turn_items),
                    first_sequence: first.sequence,
                    last_sequence: last.sequence,
                })
            })
            .collect::<Vec<_>>();
        steps.sort_by_key(|step| step.first_sequence);
        let active_turn_id = steps
            .iter()
            .rev()
            .find(|step| step.phase == TaskPhase::Active)
            .map(|step| step.turn_id);
        let latest_robot_receipt = items
            .iter()
            .filter_map(|item| match &item.payload {
                ItemPayload::RobotEpisodeReceipt { receipt }
                    if receipt.verify_integrity().is_ok() =>
                {
                    Some((item.sequence, receipt.as_ref()))
                }
                _ => None,
            })
            .max_by_key(|(sequence, _)| *sequence)
            .map(|(_, receipt)| receipt);
        let phase = latest_robot_receipt
            .map(robot_task_phase)
            .unwrap_or_else(|| session_task_phase(session.status));
        let latest_evaluation = items
            .iter()
            .filter_map(|item| match &item.payload {
                ItemPayload::EvaluationReceiptRef { receipt } => Some((item.sequence, receipt)),
                _ => None,
            })
            .max_by_key(|(sequence, _)| *sequence)
            .map(|(_, receipt)| receipt);
        let (evaluation_settlement, review_findings) = latest_evaluation
            .map(project_evaluation_settlement)
            .unwrap_or_default();
        let settlement = latest_robot_receipt
            .map(robot_task_settlement)
            .or(match session.status {
                SessionStatus::Failed => Some(TaskSettlement::Failed),
                SessionStatus::Interrupted => Some(TaskSettlement::Cancelled),
                // Completion is not acceptance. Only a persisted Host evaluation
                // receipt or immutable Robot episode receipt can project accepted.
                SessionStatus::Active | SessionStatus::Completed => evaluation_settlement,
            });

        let mut activities = project_activities(&task_id, items);
        if session.status != SessionStatus::Active {
            for activity in &mut activities {
                if activity.state == ActivityState::Running {
                    activity.state = ActivityState::Lost;
                }
            }
        }
        let runtime_facts = project_runtime_facts(items);
        let projection_fact = items
            .iter()
            .filter_map(|item| match &item.payload {
                ItemPayload::TaskProjection { fact }
                    if fact.schema_version == fabric::TASK_PROJECTION_FACT_SCHEMA_VERSION =>
                {
                    Some((item.sequence, fact))
                }
                _ => None,
            })
            .max_by_key(|(sequence, _)| *sequence)
            .map(|(_, fact)| fact.clone())
            .unwrap_or_default();
        let (active_runtime_children, active_commands, pending_approvals) =
            if session.status == SessionStatus::Active {
                (
                    projection_fact.active_runtime_children.clone(),
                    projection_fact.active_commands.clone(),
                    projection_fact.pending_approvals.clone(),
                )
            } else {
                (Vec::new(), Vec::new(), Vec::new())
            };
        let task = TaskSnapshot {
            task_id,
            session_id: session.id.clone(),
            goal,
            phase,
            plan_revision: projection_fact.plan_revision,
            steps,
            active_turn_id,
            active_runtime_children,
            active_commands,
            pending_approvals,
            budget: projection_fact.budget,
            checkpoint_head: projection_fact.checkpoint_head,
            checkpoint_review: None,
            settlement,
            review_findings,
            runtime_facts,
        };
        (vec![task], activities)
    }

    /// Materialize one already-persisted spine event into the compatibility
    /// SessionProjectionStore read model. Production handlers never pass an
    /// independently assembled Session/Item value to that store.
    pub async fn materialize(
        store: &dyn SessionProjectionStore,
        event: &SpineEvent,
    ) -> anyhow::Result<()> {
        if event.visibility == EventVisibility::Sensitive
            || is_legacy_evaluation_projection_event(event)
        {
            return Ok(());
        }
        match event.schema.0.as_str() {
            fabric::SchemaId::EVENT_SESSION_CREATED_V1 => {
                let session = current_session(decode_inline_anyhow(event)?)?;
                materialize_session_creation(store, session).await
            }
            fabric::SchemaId::EVENT_SESSION_FORKED_V1 => {
                let fork = current_fork(decode_inline_anyhow(event)?)?;
                materialize_session_creation(store, fork.child.clone()).await?;
                for item in fork.inherited_items {
                    let sequence = item.sequence;
                    let session_id = item.session_id.clone();
                    store.append(&session_id, sequence, item).await?;
                }
                Ok(())
            }
            fabric::SchemaId::EVENT_SESSION_PRINCIPAL_BOUND_V1 => {
                let binding: SessionPrincipalBoundEvent = decode_inline_anyhow(event)?;
                anyhow::ensure!(
                    binding.session_id.0 == event.identity.session_id,
                    "principal binding Session identity mismatch"
                );
                store
                    .bind_principal(&binding.session_id, &binding.principal)
                    .await
            }
            fabric::SchemaId::TURN_EVENT_V1 => {
                // A few legacy events were written under the turn schema with a
                // non-item payload (e.g. admin profile switches). Tolerate them
                // instead of poisoning the whole public-session projection: skip
                // the event with a warning, keep processing the rest.
                let item = match decode_inline_anyhow::<ItemRecord>(event).and_then(current_item) {
                    Ok(item) => item,
                    Err(error) => {
                        tracing::warn!(
                            event_id = %event.position.event_id.0,
                            %error,
                            "skipping turn-schema event with non-item payload"
                        );
                        return Ok(());
                    }
                };
                let sequence = item.sequence;
                let session_id = item.session_id.clone();
                store.append(&session_id, sequence, item).await?;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn apply_session_created(
        state: &mut PublicSessionState,
        event: &SpineEvent,
    ) -> Result<(), ProjectionError> {
        let record = current_session(decode_inline(event)?).map_err(ProjectionError::Storage)?;
        if record.id != SessionId(event.identity.session_id.clone()) {
            return Err(invalid("Session identity differs from spine"));
        }
        let view = state.sessions.entry(record.id.0.clone()).or_default();
        if view
            .record
            .as_ref()
            .is_some_and(|current| current != &record)
        {
            return Err(invalid("Session creation conflicts with prior event"));
        }
        view.record = Some(record);
        Ok(())
    }

    fn apply_session_forked(
        state: &mut PublicSessionState,
        event: &SpineEvent,
    ) -> Result<(), ProjectionError> {
        let fork = current_fork(decode_inline(event)?).map_err(ProjectionError::Storage)?;
        if fork.child.id != SessionId(event.identity.session_id.clone()) {
            return Err(invalid("Fork child identity differs from spine"));
        }
        let parent = fork
            .child
            .parent
            .as_ref()
            .ok_or_else(|| invalid("Fork child is missing parent metadata"))?;
        if parent.session_id != fork.parent_session_id
            || parent.through_sequence != fork.through_sequence
        {
            return Err(invalid("Fork payload and child metadata disagree"));
        }
        validate_items(&fork.child.id, &fork.inherited_items)?;
        if fork
            .inherited_items
            .last()
            .is_some_and(|item| item.sequence > fork.through_sequence)
        {
            return Err(invalid("Fork inherited items exceed boundary"));
        }
        let mut view = PublicSessionView {
            record: Some(fork.child.clone()),
            ..Default::default()
        };
        for item in fork.inherited_items {
            view.turns
                .entry(item.turn_id.0.to_string())
                .or_default()
                .push(item.sequence);
            view.items.push(item);
        }
        match state.sessions.get(&fork.child.id.0) {
            Some(current) if current != &view => {
                Err(invalid("Fork event conflicts with prior child projection"))
            }
            _ => {
                state.sessions.insert(fork.child.id.0, view);
                Ok(())
            }
        }
    }

    fn apply_item(
        state: &mut PublicSessionState,
        event: &SpineEvent,
    ) -> Result<(), ProjectionError> {
        // Tolerate legacy turn-schema events with non-item payloads (e.g. admin
        // profile switches written before the schema was fixed). Skipping them
        // keeps the public-session projection replayable instead of poisoned.
        let item = match decode_inline::<ItemRecord>(event) {
            Ok(value) => match current_item(value) {
                Ok(item) => item,
                Err(error) => {
                    tracing::warn!(
                        event_id = %event.position.event_id.0,
                        %error,
                        "skipping turn-schema event with non-item payload"
                    );
                    return Ok(());
                }
            },
            Err(error) => {
                tracing::warn!(
                    event_id = %event.position.event_id.0,
                    %error,
                    "skipping turn-schema event with non-item payload"
                );
                return Ok(());
            }
        };
        if item.session_id != SessionId(event.identity.session_id.clone()) {
            return Err(invalid("Session item identity differs from spine"));
        }
        let session = state.sessions.entry(item.session_id.0.clone()).or_default();
        if session
            .items
            .last()
            .is_some_and(|prior| prior.sequence >= item.sequence)
        {
            return Err(ProjectionError::NonMonotonic {
                previous: session.items.last().unwrap().sequence,
                current: item.sequence,
            });
        }
        session
            .turns
            .entry(item.turn_id.0.to_string())
            .or_default()
            .push(item.sequence);
        session.items.push(item);
        Ok(())
    }
}

fn project_evaluation_settlement(
    receipt: &fabric::EvaluationReceiptRef,
) -> (Option<TaskSettlement>, Vec<fabric::ReviewFinding>) {
    use fabric::{EvaluationDecision, ReviewFindingSeverity, ReviewFindingStatus};

    let repair_link =
        (receipt.subject_kind == "turn").then(|| format!("root:{}", receipt.subject_id));
    let evidence_ref = format!("evaluation-receipt:{}", receipt.receipt_id.0);
    let mut findings = receipt
        .failed_gates
        .iter()
        .enumerate()
        .map(|(index, gate)| fabric::ReviewFinding {
            finding_id: format!("evaluation:{}:{index}", receipt.receipt_id.0),
            severity: ReviewFindingSeverity::Error,
            summary: format!("required validation gate failed: {gate}"),
            location: None,
            evidence_refs: vec![evidence_ref.clone()],
            status: ReviewFindingStatus::Open,
            repair_link: repair_link.clone(),
        })
        .collect::<Vec<_>>();
    let settlement = match receipt.decision {
        EvaluationDecision::Accepted if findings.is_empty() => Some(TaskSettlement::Accepted),
        EvaluationDecision::Accepted
        | EvaluationDecision::Rejected
        | EvaluationDecision::ObservedFail => {
            if findings.is_empty() {
                findings.push(fabric::ReviewFinding {
                    finding_id: format!("evaluation:{}:decision", receipt.receipt_id.0),
                    severity: ReviewFindingSeverity::Error,
                    summary: "host evaluation rejected the engineering result".into(),
                    location: None,
                    evidence_refs: vec![evidence_ref],
                    status: ReviewFindingStatus::Open,
                    repair_link,
                });
            }
            Some(TaskSettlement::RepairRequired)
        }
        EvaluationDecision::Indeterminate => Some(TaskSettlement::Blocked),
        // Shadow success is evidence, not Host acceptance.
        EvaluationDecision::ObservedPass => None,
    };
    (settlement, findings)
}

fn turn_phase(items: &[&ItemRecord]) -> TaskPhase {
    if let Some(receipt) = items.iter().rev().find_map(|item| match &item.payload {
        ItemPayload::RobotEpisodeReceipt { receipt } if receipt.verify_integrity().is_ok() => {
            Some(receipt.as_ref())
        }
        _ => None,
    }) {
        return robot_task_phase(receipt);
    }
    if let Some(classification) = items.iter().rev().find_map(|item| match &item.payload {
        ItemPayload::TurnRecovery { classification } => Some(*classification),
        _ => None,
    }) {
        return match classification {
            fabric::TurnRecoveryClassification::Interrupted => TaskPhase::Interrupted,
            fabric::TurnRecoveryClassification::Failed => TaskPhase::Failed,
        };
    }
    if items.iter().any(|item| {
        matches!(
            item.payload,
            ItemPayload::AssistantMessage { .. } | ItemPayload::SystemNotice { .. }
        )
    }) {
        return TaskPhase::Completed;
    }
    if items.iter().any(|item| match &item.payload {
        ItemPayload::InferenceReceipt { receipt } => {
            receipt.status == fabric::types::inference_receipt::InferenceTerminalStatus::Failed
        }
        ItemPayload::CapabilityReceipt { receipt } => matches!(
            receipt.status,
            fabric::CapabilityTerminalStatus::Failed | fabric::CapabilityTerminalStatus::TimedOut
        ),
        _ => false,
    }) {
        return TaskPhase::Failed;
    }
    TaskPhase::Active
}

fn session_task_phase(status: SessionStatus) -> TaskPhase {
    match status {
        SessionStatus::Interrupted => TaskPhase::Interrupted,
        SessionStatus::Failed => TaskPhase::Failed,
        SessionStatus::Completed => TaskPhase::Completed,
        SessionStatus::Active => TaskPhase::Active,
    }
}

fn project_activities(task_id: &str, items: &[ItemRecord]) -> Vec<ActivitySnapshot> {
    let mut activities = BTreeMap::<String, ActivitySnapshot>::new();
    for item in items {
        match &item.payload {
            ItemPayload::ToolCall { call_id, name, .. } => {
                let activity_id = format!("tool:{}:{call_id}", item.turn_id.0);
                activities
                    .entry(activity_id.clone())
                    .or_insert(ActivitySnapshot {
                        activity_id,
                        task_id: task_id.to_owned(),
                        turn_id: item.turn_id,
                        parent_activity_id: None,
                        kind: ActivityKind::Tool,
                        label: name.clone(),
                        state: ActivityState::Running,
                        started_at: item.created_at_ms,
                        updated_at: item.created_at_ms,
                        progress: None,
                        artifact_refs: Vec::new(),
                        receipt_ref: None,
                    });
            }
            ItemPayload::ToolResult {
                call_id, is_error, ..
            } => {
                let activity_id = format!("tool:{}:{call_id}", item.turn_id.0);
                let activity = activities
                    .entry(activity_id.clone())
                    .or_insert(ActivitySnapshot {
                        activity_id,
                        task_id: task_id.to_owned(),
                        turn_id: item.turn_id,
                        parent_activity_id: None,
                        kind: ActivityKind::Tool,
                        label: call_id.clone(),
                        state: ActivityState::Lost,
                        started_at: item.created_at_ms,
                        updated_at: item.created_at_ms,
                        progress: None,
                        artifact_refs: Vec::new(),
                        receipt_ref: None,
                    });
                activity.state = if *is_error {
                    ActivityState::Failed
                } else {
                    ActivityState::Completed
                };
                activity.updated_at = item.created_at_ms;
                activity.receipt_ref = Some(format!("item:{}", item.id.0));
            }
            ItemPayload::CapabilityReceipt { receipt } => {
                let activity_id = format!("capability:{}", receipt.invocation_id);
                activities.insert(
                    activity_id.clone(),
                    ActivitySnapshot {
                        activity_id,
                        task_id: task_id.to_owned(),
                        turn_id: item.turn_id,
                        parent_activity_id: None,
                        kind: if matches!(receipt.capability.as_str(), "exec_command" | "shell") {
                            ActivityKind::Command
                        } else {
                            ActivityKind::Runtime
                        },
                        label: receipt.capability.clone(),
                        state: match receipt.status {
                            fabric::CapabilityTerminalStatus::Succeeded => ActivityState::Completed,
                            fabric::CapabilityTerminalStatus::Failed
                            | fabric::CapabilityTerminalStatus::TimedOut => ActivityState::Failed,
                            fabric::CapabilityTerminalStatus::Cancelled => ActivityState::Cancelled,
                        },
                        started_at: receipt.started_at.0,
                        updated_at: receipt.finished_at.0,
                        progress: None,
                        artifact_refs: receipt.artifact_ids.clone(),
                        receipt_ref: Some(format!("item:{}", item.id.0)),
                    },
                );
            }
            ItemPayload::RobotEpisodeReceipt { receipt } => {
                project_robot_episode_activities(task_id, item, receipt, &mut activities);
            }
            _ => {}
        }
    }
    let mut projected = activities.into_values().collect::<Vec<_>>();
    projected.sort_by(|left, right| {
        left.started_at
            .cmp(&right.started_at)
            .then_with(|| left.activity_id.cmp(&right.activity_id))
    });
    projected
}

fn robot_task_phase(receipt: &SettledEpisodeReport) -> TaskPhase {
    match receipt.report().settlement {
        EpisodeSettlement::Completed => TaskPhase::Completed,
        EpisodeSettlement::Failed => TaskPhase::Blocked,
        EpisodeSettlement::Cancelled => TaskPhase::Interrupted,
    }
}

fn robot_task_settlement(receipt: &SettledEpisodeReport) -> TaskSettlement {
    match receipt.report().settlement {
        EpisodeSettlement::Completed => TaskSettlement::Accepted,
        EpisodeSettlement::Failed => TaskSettlement::Blocked,
        EpisodeSettlement::Cancelled => TaskSettlement::Cancelled,
    }
}

fn robot_safety_denied(report: &EpisodeReport) -> bool {
    report.failures.iter().any(|failure| {
        matches!(
            failure.class,
            RobotFailureClass::ExecutionRejected | RobotFailureClass::Unsafe
        )
    })
}

fn project_robot_episode_activities(
    task_id: &str,
    item: &ItemRecord,
    receipt: &SettledEpisodeReport,
    activities: &mut BTreeMap<String, ActivitySnapshot>,
) {
    let report = receipt.report();
    let receipt_ref = format!(
        "robot-episode:{}:sha256:{}",
        report.episode_id,
        receipt.report_sha256()
    );
    let mut artifact_refs = report
        .selected_frames
        .iter()
        .map(|frame| frame.uri.clone())
        .chain(report.artifacts.iter().map(|artifact| artifact.uri.clone()))
        .chain(report.attempts.iter().flat_map(|attempt| {
            attempt
                .evidence_refs
                .iter()
                .map(|evidence| evidence.uri.clone())
        }))
        .collect::<Vec<_>>();
    artifact_refs.sort();
    artifact_refs.dedup();
    let at = u64::try_from(receipt.settled_at_unix_ms()).unwrap_or_default();
    let safety_denied = robot_safety_denied(report);
    let operation_ids = report
        .attempts
        .iter()
        .filter_map(|attempt| attempt.operation_id.clone())
        .collect::<Vec<_>>();
    let final_attempt = report.attempts.last();
    let final_decision = final_attempt
        .and_then(|attempt| attempt.verification_decision.as_ref())
        .map(|decision| match decision {
            VerificationDecision::Matched => "matched",
            VerificationDecision::RetryableMismatch => "retryable_mismatch",
            VerificationDecision::ReplannableMismatch => "replannable_mismatch",
            VerificationDecision::Unsafe => "unsafe",
            VerificationDecision::Unknown => "unknown",
        });
    let stable_window_ms = final_attempt.map(|attempt| attempt.expected.stable_window_ms);
    let mut parent = None;
    let mut insert = |order: u8,
                      stage: &str,
                      label: String,
                      state: ActivityState,
                      progress: serde_json::Value,
                      refs: Vec<String>| {
        // Every Robot stage settles at the same receipt timestamp. Prefix the
        // stable stage ordinal so the generic Activity ordering cannot turn
        // Observe→Plan→Authorize→Execute→Verify→Settle into lexical order.
        let activity_id = format!("robot:{}:{order:02}-{stage}", report.episode_id);
        activities.insert(
            activity_id.clone(),
            ActivitySnapshot {
                activity_id: activity_id.clone(),
                task_id: task_id.to_owned(),
                turn_id: item.turn_id,
                parent_activity_id: parent.clone(),
                kind: ActivityKind::Robot,
                label,
                state,
                started_at: at,
                updated_at: at,
                progress: Some(progress),
                artifact_refs: refs,
                receipt_ref: Some(receipt_ref.clone()),
            },
        );
        parent = Some(activity_id);
    };

    insert(
        1,
        "observe",
        format!("Observe {} · {}", report.device.0, report.sim_scene_version),
        if report.before_sequence.is_some() {
            ActivityState::Completed
        } else {
            ActivityState::Failed
        },
        serde_json::json!({
            "stage": "observe",
            "device": report.device.0,
            "scene": report.sim_scene_version,
            "before_sequence": report.before_sequence,
        }),
        Vec::new(),
    );
    let policy_label = report
        .policy_provenance
        .as_ref()
        .map(|policy| format!("{}/{}", policy.provider, policy.model))
        .unwrap_or_else(|| "unavailable".into());
    insert(
        2,
        "plan",
        format!("Plan {policy_label}"),
        if report.policy_provenance.is_some() && !report.attempts.is_empty() {
            ActivityState::Completed
        } else {
            ActivityState::Failed
        },
        serde_json::json!({
            "stage": "plan",
            "policy": report.policy_provenance,
            "selected_frames": report.selected_frames.iter().map(|frame| &frame.uri).collect::<Vec<_>>(),
        }),
        report
            .selected_frames
            .iter()
            .map(|frame| frame.uri.clone())
            .collect(),
    );
    insert(
        3,
        "authorize",
        format!(
            "Authorize hardware.command · attempt {}",
            report.attempts.len()
        ),
        if safety_denied {
            ActivityState::Blocked
        } else if operation_ids.is_empty() {
            ActivityState::Failed
        } else {
            ActivityState::Completed
        },
        serde_json::json!({
            "stage": "authorize",
            "operation_ids": operation_ids,
            "safety_denied": safety_denied,
        }),
        Vec::new(),
    );
    let execution_state = if safety_denied {
        ActivityState::Blocked
    } else {
        match final_attempt.and_then(|attempt| attempt.result_outcome.as_deref()) {
            Some("Succeeded") => ActivityState::Completed,
            Some(outcome) if outcome.starts_with("Cancelled") => ActivityState::Cancelled,
            Some(_) | None => ActivityState::Failed,
        }
    };
    insert(
        4,
        "execute",
        format!(
            "Execute semantic skill · {} attempt(s)",
            report.attempts.len()
        ),
        execution_state,
        serde_json::json!({
            "stage": "execute",
            "attempt_count": report.attempts.len(),
            "operation_ids": operation_ids,
        }),
        Vec::new(),
    );
    let verification_state = match final_decision {
        Some("matched") => ActivityState::Completed,
        Some("unsafe") => ActivityState::Blocked,
        Some(_) => ActivityState::Failed,
        None if safety_denied => ActivityState::Blocked,
        None => ActivityState::Failed,
    };
    insert(
        5,
        "verify",
        format!(
            "Verify {} · {}ms",
            final_decision.unwrap_or("not_run"),
            stable_window_ms.unwrap_or_default()
        ),
        verification_state,
        serde_json::json!({
            "stage": "verify",
            "decision": final_decision,
            "stable_window_ms": stable_window_ms,
            "verified_sequence": report.verified_sequence,
            "observed_paths": final_attempt.map(|attempt| &attempt.verification_observed_paths),
        }),
        Vec::new(),
    );
    let settlement = robot_task_settlement(receipt);
    let settlement_label = match settlement {
        TaskSettlement::Accepted => "accepted",
        TaskSettlement::Blocked => "blocked",
        TaskSettlement::Cancelled => "cancelled",
        TaskSettlement::RepairRequired => "repair_required",
        TaskSettlement::RolledBack => "rolled_back",
        TaskSettlement::Failed => "failed",
    };
    insert(
        6,
        "settle",
        format!("Settle {settlement_label} · EpisodeReport"),
        match settlement {
            TaskSettlement::Accepted => ActivityState::Completed,
            TaskSettlement::Blocked => ActivityState::Blocked,
            TaskSettlement::Cancelled => ActivityState::Cancelled,
            TaskSettlement::RepairRequired
            | TaskSettlement::RolledBack
            | TaskSettlement::Failed => ActivityState::Failed,
        },
        serde_json::json!({
            "stage": "settle",
            "settlement": settlement_label,
            "episode_id": report.episode_id,
            "report_sha256": receipt.report_sha256(),
            "device": report.device.0,
            "scene": report.sim_scene_version,
            "bridge_digest": report.bridge_protocol_digest,
            "skill_descriptor_digest": report.skill_descriptor_digest,
            "attempt_count": report.attempts.len(),
            "evidence_refs": artifact_refs,
        }),
        artifact_refs,
    );
}

fn project_runtime_facts(items: &[ItemRecord]) -> Option<TaskRuntimeFacts> {
    let receipts = items
        .iter()
        .filter_map(|item| match &item.payload {
            ItemPayload::InferenceReceipt { receipt } => Some(receipt),
            _ => None,
        })
        .collect::<Vec<_>>();
    let latest = receipts.last()?;
    let sum = |select: fn(&fabric::InferenceUsage) -> Option<u64>| {
        receipts.iter().try_fold(0_u64, |total, receipt| {
            select(&receipt.usage).map(|value| total.saturating_add(value))
        })
    };
    let total_input_tokens = sum(|usage| usage.total_input_tokens);
    let output_tokens = sum(|usage| usage.output_tokens);
    let cache_read_tokens = sum(|usage| usage.cache_read_tokens);
    let cache_write_tokens = sum(|usage| usage.cache_write_tokens);
    let uncached_input_tokens = sum(|usage| usage.uncached_input_tokens);
    let cache_telemetry = if receipts
        .iter()
        .all(|receipt| receipt.usage.cache_telemetry == fabric::CacheTelemetry::Reported)
    {
        fabric::CacheTelemetry::Reported
    } else if receipts
        .iter()
        .all(|receipt| receipt.usage.cache_telemetry == fabric::CacheTelemetry::Unsupported)
    {
        fabric::CacheTelemetry::Unsupported
    } else {
        fabric::CacheTelemetry::Unknown
    };
    let cache_known = cache_telemetry == fabric::CacheTelemetry::Reported;
    Some(TaskRuntimeFacts {
        effective_provider: Some(latest.provider_id.clone()),
        effective_model: Some(latest.model_id.clone()),
        context_capacity_tokens: latest.context_capacity_tokens,
        active_context_occupancy_tokens: latest.active_context_occupancy_tokens,
        cumulative_usage: fabric::InferenceUsage {
            total_input_tokens,
            output_tokens,
            uncached_input_tokens: cache_known.then_some(uncached_input_tokens).flatten(),
            cache_read_tokens: cache_known.then_some(cache_read_tokens).flatten(),
            cache_write_tokens: cache_known.then_some(cache_write_tokens).flatten(),
            cache_telemetry,
        },
        inference_rounds: receipts.len() as u64,
        provider_retries: None,
        tool_calls: items
            .iter()
            .filter(|item| matches!(item.payload, ItemPayload::ToolCall { .. }))
            .count() as u64,
        terminal_tool_results: items
            .iter()
            .filter(|item| matches!(item.payload, ItemPayload::ToolResult { .. }))
            .count() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME_DIGEST: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    fn robot_receipt(safety_denied: bool) -> SettledEpisodeReport {
        let expected = fabric::types::expected_outcome::ExpectedOutcome {
            predicate: fabric::types::expected_outcome::OutcomePredicate::Equals {
                path: "base_pose.mode".into(),
                value: serde_json::json!("standing"),
            },
            freshness_ms: 500,
            stable_window_ms: 3_000,
            timeout_ms: 9_000,
        };
        let attempt = fabric::types::episode_report::AttemptRecord {
            attempt: 1,
            attempt_id: "attempt-1".into(),
            operation_id: (!safety_denied).then(|| "operation-1".into()),
            request: Some(fabric::types::embodiment::SkillRequest {
                skill: fabric::types::embodiment::SkillId("kuavo.stance".into()),
                device: fabric::types::embodiment::DeviceId("kuavo-mujoco-01".into()),
                parameters: serde_json::json!({}),
            }),
            expected,
            result_outcome: (!safety_denied).then(|| "Succeeded".into()),
            verification_decision: (!safety_denied).then_some(VerificationDecision::Matched),
            verification_observed_paths: if safety_denied {
                Vec::new()
            } else {
                vec!["base_pose.mode".into()]
            },
            verification_reasons: Vec::new(),
            retry_reason: None,
            before_sequence: Some(10),
            after_sequence: (!safety_denied).then_some(11),
            verified_sequence: (!safety_denied).then_some(12),
            evidence_refs: if safety_denied {
                Vec::new()
            } else {
                vec![fabric::types::embodiment::EvidenceRef {
                    kind: "verification-log".into(),
                    uri: format!("artifact://sha256/{FRAME_DIGEST}"),
                }]
            },
        };
        let failure = safety_denied.then(|| {
            fabric::types::robot_failure::RobotFailure::new(
                RobotFailureClass::ExecutionRejected,
                "Safety Supervisor denied high-risk hardware permit",
            )
        });
        let report = fabric::types::episode_report::build_report(
            fabric::types::episode_report::EpisodeReportInput {
                episode_id: if safety_denied {
                    "episode-safety-denied".into()
                } else {
                    "episode-kuavo-sim".into()
                },
                goal: "stand and remain stable for three seconds".into(),
                device: fabric::types::embodiment::DeviceId("kuavo-mujoco-01".into()),
                sim_scene_version: "kuavo-mujoco/biped-s53".into(),
                aletheon_commit: "test-revision".into(),
                bridge_protocol_digest: "sha256:bridge-digest".into(),
                skill_descriptor_digest: "sha256:skill-digest".into(),
                policy_provenance: Some(fabric::types::skill_proposal::PolicyProvenance {
                    provider: "aletheon-vla-lejurobot".into(),
                    model: "deepseek-v4-flash".into(),
                    version: "2026.08.06-r8".into(),
                    protocol_version: "1.0".into(),
                    digest: "sha256:policy-digest".into(),
                }),
                failures: failure.into_iter().collect(),
                safe_stop: safety_denied.then_some(
                    fabric::types::episode_report::SafeStopReceipt {
                        attempted_after_attempt: 1,
                        trigger: Some(RobotFailureClass::ExecutionRejected),
                        outcome: fabric::types::episode_report::SafeStopOutcome::Succeeded,
                    },
                ),
                selected_frames: if safety_denied {
                    Vec::new()
                } else {
                    vec![fabric::types::frame::FrameRef {
                        uri: format!("artifact://sha256/{FRAME_DIGEST}"),
                        sha256: FRAME_DIGEST.into(),
                        mime_type: "image/png".into(),
                        width: 640,
                        height: 480,
                        byte_len: 32_000,
                        source_time_ms: 1_000,
                        camera_id: "mujoco-main".into(),
                        frame_id: 42,
                    }]
                },
                settlement: if safety_denied {
                    EpisodeSettlement::Failed
                } else {
                    EpisodeSettlement::Completed
                },
                attempts: vec![attempt],
                artifacts: Vec::new(),
            },
        );
        SettledEpisodeReport::new(report, 1_100).expect("valid immutable Robot receipt")
    }

    fn robot_projection(
        safety_denied: bool,
    ) -> (Vec<TaskSnapshot>, Vec<ActivitySnapshot>, Vec<ItemRecord>) {
        let session_id = SessionId("robot-session".into());
        let turn_id = fabric::TurnId::new();
        let items = vec![
            ItemRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id: fabric::ItemId::new(),
                session_id: session_id.clone(),
                turn_id,
                sequence: 1,
                created_at_ms: 1_000,
                payload: ItemPayload::UserMessage {
                    content: "让机器人站稳三秒".into(),
                },
            },
            ItemRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id: fabric::ItemId::new(),
                session_id: session_id.clone(),
                turn_id,
                sequence: 2,
                created_at_ms: 1_100,
                payload: ItemPayload::RobotEpisodeReceipt {
                    receipt: Box::new(robot_receipt(safety_denied)),
                },
            },
            ItemRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id: fabric::ItemId::new(),
                session_id: session_id.clone(),
                turn_id,
                sequence: 3,
                created_at_ms: 1_101,
                payload: ItemPayload::AssistantMessage {
                    content: "Robot episode settled".into(),
                },
            },
        ];
        let session = SessionRecord {
            schema_version: SESSION_SCHEMA_VERSION,
            id: session_id,
            parent: None,
            created_at_ms: 1_000,
            status: if safety_denied {
                SessionStatus::Failed
            } else {
                SessionStatus::Completed
            },
        };
        let (tasks, activities) = SessionProjection::read_model(&session, &items);
        (tasks, activities, items)
    }

    #[test]
    fn a_robot_001_kuavo_receipt_reuses_task_activity_mainline() {
        let (tasks, activities, _) = robot_projection(false);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].phase, TaskPhase::Completed);
        assert_eq!(tasks[0].settlement, Some(TaskSettlement::Accepted));
        assert_eq!(activities.len(), 6);
        assert!(activities
            .iter()
            .all(|activity| activity.kind == ActivityKind::Robot));
        let stages = activities
            .iter()
            .map(|activity| {
                activity.progress.as_ref().unwrap()["stage"]
                    .as_str()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            stages,
            [
                "observe",
                "plan",
                "authorize",
                "execute",
                "verify",
                "settle"
            ]
        );
        assert!(activities
            .windows(2)
            .all(|pair| pair[1].parent_activity_id.as_deref() == Some(&pair[0].activity_id)));
    }

    #[test]
    fn a_robot_002_hardware_is_not_projected_as_bash_or_mcp_tool() {
        let (_, activities, items) = robot_projection(false);
        assert!(!items.iter().any(|item| matches!(
            item.payload,
            ItemPayload::ToolCall { .. }
                | ItemPayload::ToolResult { .. }
                | ItemPayload::CapabilityReceipt { .. }
        )));
        assert!(activities.iter().all(|activity| {
            !matches!(
                activity.kind,
                ActivityKind::Tool | ActivityKind::Command | ActivityKind::Runtime
            )
        }));
        assert!(activities
            .iter()
            .any(|activity| activity.label.starts_with("Authorize hardware.command")));
    }

    #[test]
    fn u_robot_001_kuavo_simulation_closed_loop_is_visible() {
        let (_, activities, _) = robot_projection(false);
        let settle = activities.last().expect("settle activity");
        let progress = settle.progress.as_ref().expect("typed progress");
        assert_eq!(progress["device"], "kuavo-mujoco-01");
        assert_eq!(progress["scene"], "kuavo-mujoco/biped-s53");
        assert_eq!(progress["settlement"], "accepted");
        assert_eq!(progress["attempt_count"], 1);
        assert_eq!(progress["evidence_refs"].as_array().unwrap().len(), 1);
        assert!(settle
            .receipt_ref
            .as_deref()
            .is_some_and(|reference| reference.contains("sha256:")));
    }

    #[test]
    fn u_robot_003_policy_never_projects_as_realtime_tool_owner() {
        let (_, activities, _) = robot_projection(false);
        let plan = activities
            .iter()
            .find(|activity| activity.progress.as_ref().unwrap()["stage"] == "plan")
            .unwrap();
        let execute = activities
            .iter()
            .find(|activity| activity.progress.as_ref().unwrap()["stage"] == "execute")
            .unwrap();
        assert_eq!(plan.kind, ActivityKind::Robot);
        assert!(plan.label.contains("deepseek-v4-flash"));
        assert_eq!(execute.kind, ActivityKind::Robot);
        assert!(execute.label.contains("semantic skill"));
        assert!(!execute.label.contains("joint"));
        assert!(!execute.label.contains("torque"));
        assert!(!execute.label.contains("topic"));
    }

    #[test]
    fn robot_safety_denial_projects_blocked_not_tool_error() {
        let (tasks, activities, _) = robot_projection(true);
        assert_eq!(tasks[0].phase, TaskPhase::Blocked);
        assert_eq!(tasks[0].settlement, Some(TaskSettlement::Blocked));
        for stage in ["authorize", "execute", "verify", "settle"] {
            let activity = activities
                .iter()
                .find(|activity| activity.progress.as_ref().unwrap()["stage"] == stage)
                .unwrap();
            assert_eq!(activity.kind, ActivityKind::Robot);
            assert_eq!(activity.state, ActivityState::Blocked);
        }
        assert!(!activities
            .iter()
            .any(|activity| activity.kind == ActivityKind::Tool));
    }

    #[test]
    fn u_input_004_shell_receipt_projects_a_terminal_command_activity() {
        let session_id = SessionId("shell-session".into());
        let turn_id = fabric::TurnId::new();
        let item = ItemRecord {
            schema_version: SESSION_SCHEMA_VERSION,
            id: fabric::ItemId::new(),
            session_id,
            turn_id,
            sequence: 1,
            created_at_ms: 20,
            payload: ItemPayload::CapabilityReceipt {
                receipt: fabric::CapabilityTerminalReceipt {
                    invocation_id: "shell-call-1".into(),
                    operation_id: fabric::OperationId::new(),
                    process_id: fabric::ProcessId::new(),
                    capability: "exec_command".into(),
                    status: fabric::CapabilityTerminalStatus::Succeeded,
                    started_at: fabric::MonoTime(10),
                    finished_at: fabric::MonoTime(20),
                    exit_code: Some(0),
                    error_class: None,
                    artifact_ids: vec![],
                    evidence_ids: vec![],
                    output_ref: Some("item:shell-output".into()),
                    truncated: false,
                    retry_disposition: fabric::CapabilityRetryDisposition::Never,
                    audit_id: None,
                },
            },
        };

        let activities = project_activities("task", &[item]);
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].kind, ActivityKind::Command);
        assert_eq!(activities[0].state, ActivityState::Completed);
        assert!(activities[0]
            .receipt_ref
            .as_deref()
            .is_some_and(|receipt| receipt.starts_with("item:")));
    }
}

async fn materialize_session_creation(
    store: &dyn SessionProjectionStore,
    created: SessionRecord,
) -> anyhow::Result<()> {
    let Some(current) = store.load_session(&created.id).await? else {
        return store.create(created).await;
    };
    anyhow::ensure!(
        current.schema_version == created.schema_version
            && current.id == created.id
            && current.parent == created.parent
            && current.created_at_ms == created.created_at_ms,
        "session creation conflicts with persisted immutable content"
    );
    // Session status is a mutable read-model projection (for example startup
    // recovery can mark an interrupted Turn). Replaying the original creation
    // event must preserve that later status rather than treating it as a
    // conflicting create retry.
    Ok(())
}

impl EventProjection for SessionProjection {
    type State = PublicSessionState;

    fn descriptor(&self) -> ProjectionDescriptor {
        ProjectionDescriptor {
            name: "public-session",
            version: 1,
            accepted_schemas: &[
                fabric::SchemaId::EVENT_SESSION_CREATED_V1,
                fabric::SchemaId::EVENT_SESSION_FORKED_V1,
                fabric::SchemaId::EVENT_SESSION_PRINCIPAL_BOUND_V1,
                fabric::SchemaId::TURN_EVENT_V1,
            ],
        }
    }

    fn apply(&self, state: &mut Self::State, event: &SpineEvent) -> Result<(), ProjectionError> {
        if event.visibility == EventVisibility::Sensitive
            || is_legacy_evaluation_projection_event(event)
        {
            return Ok(());
        }
        match event.schema.0.as_str() {
            fabric::SchemaId::EVENT_SESSION_CREATED_V1 => Self::apply_session_created(state, event),
            fabric::SchemaId::EVENT_SESSION_FORKED_V1 => Self::apply_session_forked(state, event),
            fabric::SchemaId::EVENT_SESSION_PRINCIPAL_BOUND_V1 => Ok(()),
            fabric::SchemaId::TURN_EVENT_V1 => Self::apply_item(state, event),
            _ => Ok(()),
        }
    }
}

/// Releases before the dedicated evaluation schema incorrectly published
/// domain evaluation observations as `turn.event/v1`. Keep those immutable
/// historical events replayable without treating their non-ItemRecord payload
/// as public Session data. New observations use `evaluation_observed/v1`.
fn is_legacy_evaluation_projection_event(event: &SpineEvent) -> bool {
    event.schema.0 == fabric::SchemaId::TURN_EVENT_V1
        && event.envelope.source.0 == "evaluation-projection"
        && matches!(
            &event.payload,
            EventPayload::Inline { value }
                if value
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|kind| kind.starts_with("evaluation.") && kind.ends_with(".observed"))
        )
}

fn decode_inline<T: DeserializeOwned>(event: &SpineEvent) -> Result<T, ProjectionError> {
    let EventPayload::Inline { value } = &event.payload else {
        return Err(invalid("Session projection requires an inline payload"));
    };
    serde_json::from_value(value.clone())
        .map_err(anyhow::Error::from)
        .map_err(ProjectionError::Storage)
}

fn decode_inline_anyhow<T: DeserializeOwned>(event: &SpineEvent) -> anyhow::Result<T> {
    let EventPayload::Inline { value } = &event.payload else {
        anyhow::bail!("Session projection requires an inline payload");
    };
    Ok(serde_json::from_value(value.clone())?)
}

fn current_session(mut session: SessionRecord) -> anyhow::Result<SessionRecord> {
    ensure_supported_record_version(session.schema_version, "session")?;
    session.schema_version = SESSION_SCHEMA_VERSION;
    Ok(session)
}

fn current_item(mut item: ItemRecord) -> anyhow::Result<ItemRecord> {
    ensure_supported_record_version(item.schema_version, "item")?;
    item.schema_version = SESSION_SCHEMA_VERSION;
    Ok(item)
}

fn current_fork(mut fork: SessionForkedEvent) -> anyhow::Result<SessionForkedEvent> {
    fork.child = current_session(fork.child)?;
    fork.inherited_items = fork
        .inherited_items
        .into_iter()
        .map(current_item)
        .collect::<anyhow::Result<_>>()?;
    Ok(fork)
}

fn ensure_supported_record_version(version: u16, kind: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        (1..=SESSION_SCHEMA_VERSION).contains(&version),
        "unsupported {kind} schema version {version}"
    );
    Ok(())
}

fn validate_items(session: &SessionId, items: &[ItemRecord]) -> Result<(), ProjectionError> {
    let mut prior = 0;
    for item in items {
        if &item.session_id != session || item.sequence != prior + 1 {
            return Err(invalid(
                "Fork inherited items are not a contiguous child view",
            ));
        }
        prior = item.sequence;
    }
    Ok(())
}

fn invalid(message: &str) -> ProjectionError {
    ProjectionError::InvalidDescriptor(message.into())
}

#[cfg(test)]
mod host_settlement_tests {
    use super::*;

    fn receipt(
        decision: fabric::EvaluationDecision,
        failed_gates: Vec<&str>,
    ) -> fabric::EvaluationReceiptRef {
        fabric::EvaluationReceiptRef {
            schema_version: fabric::EVALUATION_SCHEMA_V1,
            receipt_id: fabric::EvaluationReceiptId::new(),
            contract_id: fabric::EvaluationContractId::new(),
            subject_kind: "turn".into(),
            subject_id: fabric::TurnId::new().0.to_string(),
            decision,
            weighted_total_millis: Some(50_000),
            evidence_coverage_millis: 800,
            confidence_millis: 900,
            failed_gates: failed_gates.into_iter().map(str::to_owned).collect(),
            created_at_ms: 1,
        }
    }

    #[test]
    fn failed_tests_override_model_completion_claims() {
        let receipt = receipt(
            fabric::EvaluationDecision::Rejected,
            vec!["required_tests_passed"],
        );
        let (settlement, findings) = project_evaluation_settlement(&receipt);
        assert_eq!(settlement, Some(TaskSettlement::RepairRequired));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].status, fabric::ReviewFindingStatus::Open);
        assert!(findings[0]
            .repair_link
            .as_deref()
            .unwrap()
            .starts_with("root:"));
    }

    #[test]
    fn missing_integration_test_is_a_visible_risk() {
        let receipt = receipt(
            fabric::EvaluationDecision::Rejected,
            vec!["integration_test_missing"],
        );
        let (settlement, findings) = project_evaluation_settlement(&receipt);
        assert_ne!(settlement, Some(TaskSettlement::Accepted));
        assert!(findings[0].summary.contains("integration_test_missing"));
        assert!(!findings[0].evidence_refs.is_empty());
    }
}

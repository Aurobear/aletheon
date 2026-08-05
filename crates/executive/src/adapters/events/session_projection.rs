use std::collections::BTreeMap;

use fabric::{
    ActivityKind, ActivitySnapshot, ActivityState, EventPayload, EventVisibility, ItemPayload,
    ItemRecord, SessionAppendStore, SessionForkedEvent, SessionId, SessionRecord, SessionStatus,
    SpineEvent, TaskPhase, TaskRuntimeFacts, TaskSettlement, TaskSnapshot, TaskStepSnapshot,
    SESSION_SCHEMA_VERSION,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

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
        let phase = session_task_phase(session.status);
        let settlement = match session.status {
            SessionStatus::Failed => Some(TaskSettlement::Failed),
            SessionStatus::Interrupted => Some(TaskSettlement::Cancelled),
            // Completion is not acceptance. Only a future authoritative Host
            // settlement receipt may project `accepted`.
            SessionStatus::Active | SessionStatus::Completed => None,
        };

        let activities = project_activities(&task_id, items);
        let runtime_facts = project_runtime_facts(items);
        let task = TaskSnapshot {
            task_id,
            session_id: session.id.clone(),
            goal,
            phase,
            plan_revision: None,
            steps,
            active_turn_id,
            active_runtime_children: Vec::new(),
            active_commands: Vec::new(),
            pending_approvals: Vec::new(),
            budget: None,
            checkpoint_head: None,
            checkpoint_review: None,
            settlement,
            runtime_facts,
        };
        (vec![task], activities)
    }

    /// Materialize one already-persisted spine event into the compatibility
    /// SessionAppendStore read model. Production handlers never pass an
    /// independently assembled Session/Item value to that store.
    pub async fn materialize(
        store: &dyn SessionAppendStore,
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

fn turn_phase(items: &[&ItemRecord]) -> TaskPhase {
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
                        kind: ActivityKind::Runtime,
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
        context_capacity_tokens: None,
        active_context_occupancy_tokens: None,
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

async fn materialize_session_creation(
    store: &dyn SessionAppendStore,
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

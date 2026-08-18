//! Daemon projection from canonical Turn events to the legacy client wire.

use contracts::ipc::TurnEventV1;
use gateway::protocol::legacy_progress::ClientEvent;
use std::future::Future;
use std::sync::Arc;
#[cfg(test)]
use tracing::debug;

#[derive(Default)]
pub(crate) struct TerminalEventBuffer {
    error: Option<String>,
    turn_done: bool,
}

impl TerminalEventBuffer {
    /// Buffer terminal stream events instead of forwarding them immediately.
    /// This lets the join result normalize failures without duplicate or
    /// out-of-order completion notifications.
    pub(crate) fn observe(&mut self, event: &TurnEventV1) -> bool {
        match event {
            TurnEventV1::Error { message } => {
                if self.error.is_none() {
                    self.error = Some(message.clone());
                }
                true
            }
            TurnEventV1::TurnDone { .. } => {
                self.turn_done = true;
                true
            }
            _ => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn into_client_events(self, turn_error: Option<String>) -> Vec<ClientEvent> {
        let error = self.error.or(turn_error);
        if !self.turn_done {
            debug!("Synthesizing missing terminal turn_done event");
        }

        let mut events = Vec::with_capacity(if error.is_some() { 2 } else { 1 });
        if let Some(message) = error {
            events.push(ClientEvent::Error { message });
        }
        events.push(ClientEvent::TurnDone);
        events
    }
}

pub(crate) struct ProjectedProtocolEvent {
    pub(crate) disconnected: bool,
}

pub(crate) struct PumpedCognitiveTurn<S> {
    pub(crate) result: anyhow::Result<contracts::TurnResult>,
    pub(crate) evidence: application::turn::evidence::TurnEvidenceAccumulator,
    pub(crate) terminal_events: TerminalEventBuffer,
    pub(crate) state: S,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn pump_cognitive_turn<S, F, Fut>(
    mut result_rx: tokio::sync::oneshot::Receiver<anyhow::Result<contracts::TurnResult>>,
    turn_stream: &mut contracts::ipc::TurnEventStream,
    approvals: &dyn application::turn::ports::TurnApprovalPort,
    sessions: &runtime::session_service::SessionService,
    session_id: &str,
    turn_id: contracts::TurnId,
    assistant_item_id: &str,
    notification: Option<&Arc<dyn application::turn::service::TurnNotificationPort>>,
    mut state: S,
    mut on_tool_terminal: F,
) -> anyhow::Result<PumpedCognitiveTurn<S>>
where
    F: FnMut(S, application::turn::evidence::ToolTerminalEvidence) -> Fut,
    Fut: Future<Output = anyhow::Result<S>>,
{
    let mut evidence = application::turn::evidence::TurnEvidenceAccumulator::default();
    let mut terminal_events = TerminalEventBuffer::default();
    let mut stream_open = true;
    let result = loop {
        tokio::select! {
            result = &mut result_rx => {
                break result.unwrap_or_else(|_| Err(anyhow::anyhow!(
                    "react task terminated without returning a result"
                )));
            }
            event_result = turn_stream.recv_optional(), if stream_open => {
                let Some(event_result) = event_result else {
                    stream_open = false;
                    tracing::debug!("Turn event stream closed after its producer settled");
                    continue;
                };
                let event = match event_result {
                    Ok(event) => event,
                    Err(rejection) => {
                        tracing::warn!(schema = %rejection.actual, "Schema mismatch in turn event stream, skipping event");
                        continue;
                    }
                };
                let projected = project_protocol_event(
                    sessions,
                    session_id,
                    turn_id,
                    assistant_item_id,
                    &event,
                    &mut terminal_events,
                    notification,
                ).await?;
                if let Some(tool) = evidence.observe_live(&event) {
                    state = on_tool_terminal(state, tool).await?;
                }
                if projected.disconnected {
                    tracing::debug!("Event sink closed, dropping event");
                }
            }
            Some(pending) = approvals.next() => {
                super::turn_approval_projection::project(sessions, pending, notification).await;
            }
        }
    };

    tokio::task::yield_now().await;
    loop {
        match turn_stream.try_recv() {
            Some(Ok(event)) => {
                let projected = project_protocol_event(
                    sessions,
                    session_id,
                    turn_id,
                    assistant_item_id,
                    &event,
                    &mut terminal_events,
                    notification,
                )
                .await?;
                evidence.observe_drain(&event);
                if projected.disconnected {
                    tracing::debug!("Event sink closed during drain, stopping event stream");
                    break;
                }
            }
            Some(Err(rejection)) => {
                tracing::warn!(schema = %rejection.actual, "Schema mismatch in event drain, skipping");
            }
            None => break,
        }
    }

    Ok(PumpedCognitiveTurn {
        result,
        evidence,
        terminal_events,
        state,
    })
}

pub(crate) async fn project_protocol_event(
    sessions: &runtime::session_service::SessionService,
    session_id: &str,
    turn_id: contracts::TurnId,
    assistant_item_id: &str,
    event: &TurnEventV1,
    terminal_events: &mut TerminalEventBuffer,
    notification: Option<&Arc<dyn application::turn::service::TurnNotificationPort>>,
) -> anyhow::Result<ProjectedProtocolEvent> {
    adapters_sqlite::session::turn_event_journal::journal_protocol_turn_event(
        sessions,
        session_id,
        turn_id,
        assistant_item_id,
        event,
    )
    .await?;
    let terminal = terminal_events.observe(event);
    let disconnected = if terminal {
        false
    } else if let (Some(port), Some(client_event)) =
        (notification, turn_event_to_client_event(event))
    {
        match event_to_json(&client_event) {
            Ok(payload) => port.send(payload).await.is_err(),
            Err(error) => {
                tracing::warn!(%error, "failed to serialize projected Turn event");
                false
            }
        }
    } else {
        false
    };
    Ok(ProjectedProtocolEvent { disconnected })
}

/// Convert a `TurnEventV1` into a `ClientEvent` for TUI forwarding.
/// Project the canonical daemon turn stream into the legacy TUI wire event.
/// Public so transport/consumer integration tests exercise the production
/// projection rather than duplicating its field mapping.
pub(crate) fn turn_event_to_client_event(event: &TurnEventV1) -> Option<ClientEvent> {
    match event {
        TurnEventV1::TurnStarted { iteration } => Some(ClientEvent::TurnStarted {
            iteration: *iteration,
        }),
        TurnEventV1::TextDelta { delta } => Some(ClientEvent::TextDelta {
            text: delta.clone(),
        }),
        TurnEventV1::ToolCallStart { name, call_id } => Some(ClientEvent::ToolCallStart {
            call_id: call_id.clone(),
            tool: name.clone(),
            args: serde_json::Value::Null,
        }),
        TurnEventV1::ToolCallComplete {
            call_id,
            name,
            args,
        } => Some(ClientEvent::ToolCallComplete {
            call_id: call_id.clone(),
            tool: name.clone(),
            args: args.clone(),
        }),
        TurnEventV1::ToolResult {
            name,
            call_id,
            content,
            is_error,
            execution_time_ms,
            patch_delta,
        } => Some(ClientEvent::ToolCallResult {
            call_id: call_id.clone(),
            tool: name.clone(),
            output: content.clone(),
            is_error: *is_error,
            elapsed_ms: *execution_time_ms,
            patch_delta: patch_delta.clone(),
        }),
        TurnEventV1::ToolProgress {
            name,
            call_id,
            kind,
            payload,
        } => Some(ClientEvent::ToolProgress {
            call_id: call_id.clone(),
            tool: name.clone(),
            kind: kind.clone(),
            payload: payload.clone(),
        }),
        TurnEventV1::PatchProgress {
            status,
            path,
            operation,
            error,
            applied_count,
            failed_count,
        } => Some(ClientEvent::PatchProgress {
            status: status.clone(),
            path: path.clone(),
            operation: operation.clone(),
            error: error.clone(),
            applied_count: *applied_count,
            failed_count: *failed_count,
        }),
        TurnEventV1::Usage { usage } => Some(ClientEvent::Usage {
            usage: usage.clone(),
        }),
        TurnEventV1::TurnDone { .. } => Some(ClientEvent::TurnDone),
        TurnEventV1::Error { message } => Some(ClientEvent::Error {
            message: message.clone(),
        }),
        TurnEventV1::AwarenessChanged { level, context } => Some(ClientEvent::AwarenessChanged {
            level: level.clone(),
            context: context.clone(),
        }),
        TurnEventV1::ModeChanged { mode } => Some(ClientEvent::ModeChanged { new: mode.clone() }),
        TurnEventV1::SubAgentStatusChanged {
            agent_id,
            status,
            task,
        } => Some(ClientEvent::SubAgentStatus {
            agent_id: agent_id.clone(),
            task: task.clone(),
            status: status.clone(),
        }),
        TurnEventV1::PlanUpdate {
            version,
            plan,
            critique,
            ready_for_approval,
        } => Some(ClientEvent::PlanUpdate {
            version: *version,
            plan: plan.clone(),
            critique: critique.clone(),
            ready_for_approval: *ready_for_approval,
        }),
        TurnEventV1::Interrupted { .. } => Some(ClientEvent::Interrupted),
        TurnEventV1::ContextUpdate {
            used_tokens,
            max_tokens,
        } => Some(ClientEvent::ContextUpdate {
            used_tokens: *used_tokens as u64,
            max_tokens: *max_tokens as u64,
        }),
        TurnEventV1::ModelSwitch { model_name } => Some(ClientEvent::ModelSwitch {
            model: model_name.clone(),
        }),
        TurnEventV1::GoalSet { goal, sub_goals } => Some(ClientEvent::GoalSet {
            goal: goal.clone(),
            sub_goals: sub_goals.clone(),
        }),
        TurnEventV1::Reflection { summary, .. } => Some(ClientEvent::Reflection {
            summary: summary.clone(),
        }),
        TurnEventV1::BudgetExceeded { max, .. } => {
            Some(ClientEvent::BudgetExceeded { limit: *max as u64 })
        }
        TurnEventV1::CircuitBreakerTripped { reason } => Some(ClientEvent::CircuitBreakerTripped {
            reason: reason.clone(),
        }),
        TurnEventV1::CompactionTriggered { .. } => Some(ClientEvent::CompactionTriggered),
        TurnEventV1::CompactionOutcome {
            strategy,
            applied: true,
            tokens_before,
            tokens_after,
            evicted_messages,
            ..
        } => Some(ClientEvent::CompactionCompleted {
            strategy: strategy.clone(),
            tokens_before: *tokens_before as u64,
            tokens_after: *tokens_after as u64,
            evicted_messages: *evicted_messages as u64,
        }),
        // Failed or skipped outcomes remain on the canonical stream for
        // diagnostics. The TUI must not imply success from a trigger alone.
        TurnEventV1::CompactionOutcome { .. }
        | TurnEventV1::TextDeltaStop
        | TurnEventV1::Approval { .. }
        | TurnEventV1::RobotEpisodeSettled { .. }
        | TurnEventV1::Generic { .. } => None,
    }
}

/// Serialize a `ClientEvent` into a JSON-RPC notification string.
pub(crate) fn event_to_json(event: &ClientEvent) -> serde_json::Result<String> {
    let notification = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "event",
        "params": event,
    });
    serde_json::to_string(&notification)
}

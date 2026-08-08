use std::io;

use fabric::cognit::{Critique, Plan};
use fabric::ui_event::{
    AwarenessLevel, ClientEvent, CollaborationMode, SubAgentHandle, SubAgentStatus,
};

use super::chat::Role as ChatRole;
use super::plan_view::PlanVersion;
use super::test_infra::EventRecorder;
use super::App;

/// Variant of `try_read_socket` that records events via `EventRecorder`.
pub fn try_read_socket_with_recorder(
    app: &mut App,
    event_recorder: &mut Option<EventRecorder>,
) -> bool {
    let mut changed = false;
    loop {
        match app.stream.try_read(&mut app.read_buf) {
            Ok(0) => {
                changed = true;
                app.streaming = false;
                app.status.waiting = false;
                app.app_state.streaming = false;
                app.compat_transcript
                    .add_text(ChatRole::System, "连接断开".to_string());
                break;
            }
            Ok(n) => {
                changed = true;
                app.response_buf.push(&app.read_buf[..n]);

                loop {
                    let line = match app.response_buf.take_line() {
                        Ok(Some(line)) => line.trim().to_string(),
                        Ok(None) => break,
                        Err(error) => {
                            app.compat_transcript.add_text(
                                ChatRole::System,
                                format!("Error: daemon protocol contained invalid UTF-8: {error}"),
                            );
                            app.streaming = false;
                            app.status.waiting = false;
                            app.app_state.streaming = false;
                            break;
                        }
                    };

                    if line.is_empty() {
                        continue;
                    }

                    if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                        if msg.get("method").and_then(|v| v.as_str()) == Some("event") {
                            if let Some(params) = msg.get("params") {
                                // Record event before processing
                                if let Some(ref mut recorder) = event_recorder {
                                    recorder.write(params);
                                }
                                handle_event(app, params);
                            }
                        } else if msg.get("method").and_then(|v| v.as_str())
                            == Some("approval_request")
                        {
                            handle_approval(app, &msg);
                        } else if msg.get("result").is_some() || msg.get("error").is_some() {
                            process_response(app, msg);
                            // Don't break — continue processing remaining lines
                            // in the buffer (streaming events may follow in the
                            // same chunk as the response).
                        }
                    }
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(_) => {
                changed = true;
                app.streaming = false;
                app.status.waiting = false;
                app.app_state.streaming = false;
                break;
            }
        }
    }
    changed
}

pub fn handle_event(app: &mut App, params: &serde_json::Value) {
    let event: ClientEvent = match serde_json::from_value(params.clone()) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to deserialize ClientEvent params");
            return;
        }
    };

    match event {
        ClientEvent::TurnStarted { iteration } => {
            let new_turn = iteration == 0 || !app.turn_active;
            let observed_at = app.clock.mono_now().0;
            super::reducer::begin_live_turn(&mut app.app_state, None);
            // Cognit emits iteration 0 as a turn-lifecycle marker, then emits
            // one-based inference iterations. Do not render the lifecycle
            // marker as a phantom inference round.
            if let Some(inference_index) = iteration.checked_sub(1) {
                let _ = super::reducer::reduce(
                    &mut app.app_state,
                    super::reducer::UiAction::LiveActivity(
                        super::reducer::LiveActivityEvent::InferenceStarted {
                            iteration: inference_index,
                            observed_at,
                        },
                    ),
                );
            }
            if new_turn {
                app.stream_ctrl.start_turn();
                app.status.elapsed_secs = 0.0;
                app.app_state.turn_tool_count = 0;
                app.app_state.turn_activity = super::state::TurnActivity::default();
                app.app_state.turn_input_tokens = 0;
                app.app_state.turn_output_tokens = 0;
            }
            app.status.waiting = false;
            app.turn_active = true;
            app.streaming = true;
            app.app_state.streaming = true;
            app.app_state.turn_active = true;
            app.current_iteration = iteration;
            app.app_state.current_iteration = iteration;
        }
        ClientEvent::ThinkingDelta { text } => {
            app.stream_ctrl.push_thinking(&text);
            app.compat_transcript
                .set_assistant_stream(app.stream_ctrl.current_text());
            app.dispatch_live_assistant_text();
        }
        ClientEvent::TextDelta { text } => {
            app.stream_ctrl.push_text(&text);
            app.compat_transcript
                .set_assistant_stream(app.stream_ctrl.current_text());
            app.dispatch_live_assistant_text();
        }
        ClientEvent::TextSnapshot { text } => {
            app.stream_ctrl.replace_text(&text);
            app.compat_transcript
                .set_assistant_stream(app.stream_ctrl.current_text());
            app.dispatch_live_assistant_text();
        }
        ClientEvent::ToolCallStart {
            call_id,
            tool,
            args,
        } => {
            let observed_at = app.clock.mono_now().0;
            let _ = super::reducer::reduce(
                &mut app.app_state,
                super::reducer::UiAction::LiveActivity(
                    super::reducer::LiveActivityEvent::ToolStarted {
                        call_id: call_id.clone(),
                        tool: tool.clone(),
                        args: public_tool_args(&args),
                        observed_at,
                    },
                ),
            );
            app.compat_transcript.discard_trailing_assistant_draft();
            let args_str = serde_json::to_string(&args).unwrap_or_default();
            app.compat_transcript
                .add_exec(call_id.clone(), tool.clone(), args_str);
            app.app_state.turn_tool_count += 1;
            app.app_state.turn_activity.tool_calls += 1;
        }
        ClientEvent::ToolCallComplete {
            call_id,
            tool: _,
            args,
        } => {
            let observed_at = app.clock.mono_now().0;
            let _ = super::reducer::reduce(
                &mut app.app_state,
                super::reducer::UiAction::LiveActivity(
                    super::reducer::LiveActivityEvent::ToolArguments {
                        call_id: call_id.clone(),
                        args: public_tool_args(&args),
                        observed_at,
                    },
                ),
            );
            let args_str = serde_json::to_string(&args).unwrap_or_default();
            app.compat_transcript.update_exec_args(&call_id, &args_str);
        }
        ClientEvent::ToolCallResult {
            call_id,
            output,
            is_error,
            elapsed_ms,
            patch_delta,
            ..
        } => {
            let observed_at = app.clock.mono_now().0;
            let _ = super::reducer::reduce(
                &mut app.app_state,
                super::reducer::UiAction::LiveActivity(
                    super::reducer::LiveActivityEvent::ToolFinished {
                        call_id: call_id.clone(),
                        is_error,
                        elapsed_ms,
                        observed_at,
                    },
                ),
            );
            if let Some(delta) = patch_delta.as_ref() {
                app.latest_patch = Some(delta.clone());
            }
            if let Some(preview) = patch_delta
                .as_ref()
                .and_then(|delta| delta.diff_preview.clone())
            {
                app.latest_diff = Some(preview);
            }
            app.compat_transcript
                .update_exec_with_delta(&call_id, &output, is_error, patch_delta);
            if is_error {
                if output.starts_with("Policy denied:")
                    || output.starts_with("Policy guidance:")
                    || output.contains("Policy denied")
                {
                    app.app_state.turn_activity.denied += 1;
                } else {
                    app.app_state.turn_activity.failed += 1;
                }
            } else {
                app.app_state.turn_activity.succeeded += 1;
            }
        }
        ClientEvent::ToolProgress {
            call_id, payload, ..
        } => {
            let observed_at = app.clock.mono_now().0;
            let _ = super::reducer::reduce(
                &mut app.app_state,
                super::reducer::UiAction::LiveActivity(
                    super::reducer::LiveActivityEvent::ToolProgress {
                        call_id: call_id.clone(),
                        payload: payload.clone(),
                        observed_at,
                    },
                ),
            );
            let progress = payload
                .as_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| payload.to_string());
            app.compat_transcript
                .update_exec_progress(&call_id, &progress);
        }
        ClientEvent::PatchProgress {
            status,
            path,
            operation,
            error,
            applied_count,
            failed_count,
        } => {
            let target = path.as_deref().unwrap_or("patch");
            let operation = operation.as_deref().unwrap_or("apply");
            let detail = error.unwrap_or_else(|| match (applied_count, failed_count) {
                (Some(applied), Some(failed)) => format!("{applied} applied, {failed} failed"),
                _ => String::new(),
            });
            app.compat_transcript.add_text(
                ChatRole::System,
                format!("Patch {status}: {operation} {target} {detail}")
                    .trim_end()
                    .to_owned(),
            );
        }
        ClientEvent::Usage { usage } => {
            let observed_at = app.clock.mono_now().0;
            let _ = super::reducer::reduce(
                &mut app.app_state,
                super::reducer::UiAction::LiveActivity(
                    super::reducer::LiveActivityEvent::InferenceFinished {
                        iteration: app.current_iteration.saturating_sub(1),
                        observed_at,
                    },
                ),
            );
            app.app_state.turn_activity.inference_rounds += 1;
            let tokens_in = usage.total_input_tokens.unwrap_or(0);
            let tokens_out = usage.output_tokens.unwrap_or(0);
            app.app_state.turn_input_tokens =
                app.app_state.turn_input_tokens.saturating_add(tokens_in);
            app.app_state.turn_output_tokens =
                app.app_state.turn_output_tokens.saturating_add(tokens_out);
            app.app_state.total_tokens = app
                .app_state
                .total_tokens
                .saturating_add(tokens_in)
                .saturating_add(tokens_out);
            app.status.token_count = Some(
                app.app_state
                    .turn_input_tokens
                    .saturating_add(app.app_state.turn_output_tokens),
            );
            app.status.total_tokens = app.app_state.total_tokens;
        }
        ClientEvent::TurnDone => {
            app.stream_ctrl.commit();
            app.compat_transcript
                .set_assistant_stream(app.stream_ctrl.current_text());
            app.streaming = false;
            app.status.waiting = false;
            app.app_state.streaming = false;
            app.turn_active = false;
            app.app_state.turn_active = false;
            app.status.session_turns += 1;
        }
        ClientEvent::Error { message } => {
            app.compat_transcript
                .add_text(ChatRole::System, format!("Error: {message}"));
            app.streaming = false;
            app.status.waiting = false;
            app.app_state.streaming = false;
            app.turn_active = false;
            app.app_state.turn_active = false;
        }
        ClientEvent::AwarenessChanged { level, context } => {
            if let Ok(awareness_level) =
                serde_json::from_str::<AwarenessLevel>(&format!("\"{level}\""))
            {
                app.app_state
                    .awareness
                    .update(awareness_level, context, app.clock.mono_now());
            }
        }
        ClientEvent::PlanUpdate {
            version,
            plan,
            critique,
            ready_for_approval,
        } => {
            if let Ok(plan_obj) = serde_json::from_str::<Plan>(&plan) {
                let critique_obj: Option<Vec<Critique>> = critique
                    .as_ref()
                    .and_then(|c| serde_json::from_str(c.as_str()).ok());
                app.plan_view.add_version(PlanVersion {
                    version: version as usize,
                    plan: plan_obj,
                    critique: critique_obj,
                });
                app.plan_view.set_ready(ready_for_approval);
            }
        }
        ClientEvent::SubAgentStatus {
            agent_id,
            task,
            status,
        } => {
            if let Ok(s) = serde_json::from_str::<SubAgentStatus>(&format!("\"{status}\"")) {
                let existing = app.sub_agents.iter_mut().find(|a| a.id == agent_id);
                match existing {
                    Some(handle) => {
                        handle.status = s;
                        handle.task = task;
                    }
                    None => {
                        app.sub_agents.push(SubAgentHandle {
                            id: agent_id,
                            task,
                            status: s,
                            parent_turn_id: String::new(),
                            spawned_at_ms: 0,
                        });
                    }
                }
            }
        }
        ClientEvent::ModeChanged { new } => {
            if let Ok(mode) = serde_json::from_str::<CollaborationMode>(&format!("\"{new}\"")) {
                app.app_state.mode = mode;
            }
        }
        ClientEvent::ContextUpdate {
            max_tokens,
            used_tokens,
        } => {
            app.app_state.context.used = usize::try_from(used_tokens).ok();
            app.app_state.context.max = usize::try_from(max_tokens).ok();
            app.status.context_used_tokens = used_tokens;
            app.status.context_window = max_tokens;
        }
        ClientEvent::ModelSwitch { model } => {
            app.app_state.model_name = model.clone();
            app.model_name = model.clone();
            app.status.model_name = model;
        }
        ClientEvent::Interrupted => {
            app.compat_transcript
                .add_text(ChatRole::System, "Interrupted".to_string());
        }
        ClientEvent::BudgetExceeded { limit } => {
            app.compat_transcript
                .add_text(ChatRole::System, format!("Budget exceeded: {limit} tokens"));
        }
        ClientEvent::CircuitBreakerTripped { reason } => {
            app.compat_transcript
                .add_text(ChatRole::System, format!("Circuit breaker: {reason}"));
        }
        ClientEvent::CompactionTriggered => {
            // Wait for the validated outcome; a trigger alone is not success.
        }
        ClientEvent::CompactionCompleted {
            strategy,
            tokens_before,
            tokens_after,
            evicted_messages,
        } => {
            let ratio = if tokens_before == 0 {
                0.0
            } else {
                tokens_after as f64 / tokens_before as f64 * 100.0
            };
            app.compat_transcript.add_text(
                ChatRole::System,
                format!(
                    "自动压缩完成：{tokens_before} → {tokens_after} tokens（{ratio:.1}%），\
                     移除 {evicted_messages} 条消息，策略 {strategy}"
                ),
            );
        }
        ClientEvent::Reflection { summary } => {
            // Routine reflection is internal control flow, not conversation.
            // Surface only a reflection that changes strategy or stops work.
            if !(summary.contains("Spec: on track") && summary.ends_with("Continuing...")) {
                let observed_at = app.clock.mono_now().0;
                let _ = super::reducer::reduce(
                    &mut app.app_state,
                    super::reducer::UiAction::LiveActivity(
                        super::reducer::LiveActivityEvent::ProgressSummary {
                            summary: summary.clone(),
                            observed_at,
                        },
                    ),
                );
                app.compat_transcript.add_text(ChatRole::System, summary);
            }
        }
        ClientEvent::GoalSet {
            goal: _,
            sub_goals: _,
        } => {
            // goal set — update app state
        }
    }
}

fn public_tool_args(value: &serde_json::Value) -> serde_json::Value {
    fn redact_inline_secrets(value: &str) -> String {
        let mut redacted = value.to_owned();
        for marker in [
            "anthropic_api_key=",
            "openai_api_key=",
            "google_api_key=",
            "api_key=",
            "authorization: bearer ",
            "secret=",
            "token=",
        ] {
            let mut search_from = 0;
            while let Some(offset) = redacted[search_from..].to_ascii_lowercase().find(marker) {
                let start = search_from + offset;
                let value_start = start + marker.len();
                let value_end = redacted[value_start..]
                    .find(char::is_whitespace)
                    .map(|offset| value_start + offset)
                    .unwrap_or(redacted.len());
                redacted.replace_range(value_start..value_end, "[redacted]");
                search_from = value_start + "[redacted]".len();
            }
        }
        redacted
    }

    fn redact(value: &serde_json::Value, depth: usize) -> serde_json::Value {
        if depth >= 3 {
            return serde_json::json!("…");
        }
        match value {
            serde_json::Value::Object(map) => serde_json::Value::Object(
                map.iter()
                    .take(8)
                    .map(|(key, value)| {
                        let sensitive = [
                            "token",
                            "secret",
                            "password",
                            "credential",
                            "api_key",
                            "authorization",
                            "bearer",
                        ]
                        .iter()
                        .any(|needle| key.to_ascii_lowercase().contains(needle));
                        (
                            key.clone(),
                            if sensitive {
                                serde_json::json!("[redacted]")
                            } else {
                                redact(value, depth + 1)
                            },
                        )
                    })
                    .collect(),
            ),
            serde_json::Value::Array(values) => serde_json::Value::Array(
                values
                    .iter()
                    .take(8)
                    .map(|value| redact(value, depth + 1))
                    .collect(),
            ),
            serde_json::Value::String(value) => {
                let value = redact_inline_secrets(value);
                let bounded = value.chars().take(160).collect::<String>();
                serde_json::Value::String(if value.chars().count() > 160 {
                    format!("{bounded}…")
                } else {
                    bounded
                })
            }
            value => value.clone(),
        }
    }
    redact(value, 0)
}

pub fn handle_approval(app: &mut App, msg: &serde_json::Value) {
    if let Some(params) = msg.get("params") {
        let approval_id = params
            .get("approval_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tool = params
            .get("tool")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let action_summary = params
            .get("action_summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let risk_level = params
            .get("risk_level")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let detail = params
            .get("detail")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let scope_subject = params
            .get("scope_subject")
            .cloned()
            .filter(|value| !value.is_null())
            .and_then(|value| serde_json::from_value(value).ok());
        app.pending_approval = Some(
            super::approval_dialog::ApprovalDialog::new(
                approval_id,
                tool,
                action_summary,
                risk_level,
            )
            .with_detail(detail)
            .with_scope_subject(scope_subject),
        );
    }
}

pub fn process_response(app: &mut App, msg: serde_json::Value) {
    if let Some(request_id) = msg.get("id").and_then(serde_json::Value::as_u64) {
        if app.pending_non_turn.remove(&request_id) {
            app.streaming = false;
            app.status.waiting = false;
            app.app_state.streaming = false;
        }
    }
    if apply_pending_command_response(app, &msg) {
        return;
    }
    if apply_typed_command_output(app, &msg) {
        return;
    }
    if apply_typed_protocol_event(app, &msg) {
        return;
    }
    // All unversioned response shapes are isolated in one V0 compatibility
    // adapter. New daemon output must use CommandOutputEnvelopeV1 or
    // ClientMessage<ClientEvent>; the main path never guesses business fields.
    let _legacy_v0_consumed = apply_legacy_v0_response(app, &msg);
    // NOTE: Do NOT clear streaming/waiting here. The JSON-RPC result arrives
    // BEFORE the turn_done event. Clearing streaming here causes a visible UI
    // freeze between tool calls. Let turn_done handle the state transition.
    //
    // Also do NOT clear response_buf — streaming events may follow in the
    // same try_read chunk.
}

/// Temporary V0 response adapter. This is the only location allowed to inspect
/// legacy result fields; remove after the compatibility window ending 2026-12-31.
fn set_compat_assistant(app: &mut App, text: String) {
    let already_streamed = !text.is_empty() && app.stream_ctrl.current_text() == text;
    app.compat_transcript.set_assistant_stream(text.clone());
    if !already_streamed {
        app.replace_transient_assistant(text);
    }
}

fn add_compat_notice(app: &mut App, text: String) {
    app.compat_transcript.add_text(ChatRole::System, text);
    app.sync_compat_notices();
}

fn apply_legacy_v0_response(app: &mut App, msg: &serde_json::Value) -> bool {
    if let Some(result) = msg.get("result") {
        if let Some(text) = result.get("response").and_then(|v| v.as_str()) {
            // Standard chat response - deduplicate consecutive identical text
            // Some models repeat thinking/reasoning text
            let deduped = deduplicate_consecutive_text(text);
            set_compat_assistant(app, deduped);
        } else if let Some(status) = result.get("status") {
            // /status response — rich self-evolution state
            let formatted = format_status(status);
            set_compat_assistant(app, formatted);
        } else if let Some(sessions) = result.get("sessions") {
            // /sessions response
            let formatted = format_sessions(sessions);
            set_compat_assistant(app, formatted);
        } else if let Some(_models) = result.get("models") {
            // /model response
            let formatted = format_models(result);
            set_compat_assistant(app, formatted);
        } else if let Some(skills) = result.get("skills") {
            // Phase B: SkillsCatalog response — populate the command registry
            // so Tab-completion and /help reflect daemon skills.
            app.registry.set_skills_from_json(skills);
            let formatted = format_skills_list(skills);
            set_compat_assistant(app, formatted);
        } else if let Some(facts) = result.get("facts") {
            // /memory response — render fact list
            let formatted = format_memory_facts(facts);
            set_compat_assistant(app, formatted);
        } else if let Some(memory) = result.get("memory") {
            // /memory status response
            set_compat_assistant(app, format_memory_status(memory));
        } else if let Some(receipt) = result.get("receipt") {
            if receipt.is_null() {
                add_compat_notice(
                    app,
                    "No evaluation receipt is available for this session.".to_string(),
                );
            } else {
                match serde_json::from_value::<fabric::EvaluationReceiptRef>(receipt.clone()) {
                    Ok(receipt) => {
                        app.app_state.latest_evaluation = Some(receipt.clone());
                        add_compat_notice(
                            app,
                            super::reducer::format_evaluation_receipt_ref(&receipt),
                        );
                    }
                    Err(error) => add_compat_notice(
                        app,
                        format!("Invalid evaluation receipt response: {error}"),
                    ),
                }
            }
        } else if let Some(content) = result.get("content").and_then(|value| value.as_str()) {
            // session.memory returns bounded markdown owned by the daemon.
            set_compat_assistant(app, content.to_string());
        } else if let Some(tools) = result.get("tools") {
            // tools/list response
            let formatted = format_tools_list(tools);
            set_compat_assistant(app, formatted);
        } else if let Some(agents) = result.get("agents") {
            // /agents response
            let formatted = format_agents(agents);
            set_compat_assistant(app, formatted);
        } else if let Some(msg_text) = result.get("message").and_then(|v| v.as_str()) {
            // Generic message response (e.g. /resume, /compact)
            set_compat_assistant(app, msg_text.to_string());
        }
    } else if let Some(error) = msg.get("error") {
        let err = error
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        add_compat_notice(app, format!("Error: {err}"));
    }
    msg.get("result").is_some() || msg.get("error").is_some()
}

fn apply_typed_command_output(app: &mut App, message: &serde_json::Value) -> bool {
    use fabric::contract::command::{
        CommandOutputEnvelopeV1, CommandOutputProtocol, CommandOutputV1,
    };

    let Some(result) = message.get("result") else {
        return false;
    };
    let is_command_output =
        result.get("protocol").and_then(serde_json::Value::as_str) == Some("command_output");
    if !is_command_output {
        return false;
    }

    let output = match serde_json::from_value::<CommandOutputEnvelopeV1>(result.clone()) {
        Ok(output) if output.protocol == CommandOutputProtocol::CommandOutput => output,
        Ok(_) => unreachable!("CommandOutputProtocol currently has one variant"),
        Err(error) => {
            add_compat_notice(app, format!("Command output protocol rejected: {error}"));
            return true;
        }
    };
    let output = match output.into_v1() {
        Ok(output) => output,
        Err(error) => {
            add_compat_notice(app, format!("Command output protocol rejected: {error}"));
            return true;
        }
    };

    match output {
        CommandOutputV1::PromptAccepted => {}
        CommandOutputV1::PromptCompleted(completion) => {
            set_compat_assistant(app, deduplicate_consecutive_text(&completion.response));
            if completion.stop != fabric::TurnStop::Completed {
                add_compat_notice(app, format!("Turn stopped: {:?}", completion.stop));
            }
        }
        CommandOutputV1::CancelRequested(cancel) => add_compat_notice(
            app,
            format!(
                "Cancellation requested for {} active turn(s).",
                cancel.active_turns
            ),
        ),
        CommandOutputV1::Status(status) => {
            set_compat_assistant(
                app,
                format!(
                    "{}: {}",
                    if status.ready { "ready" } else { "not ready" },
                    status.summary
                ),
            );
        }
        CommandOutputV1::StatusProjected(status) => {
            set_compat_assistant(app, format_status_projection(&status));
        }
        CommandOutputV1::Rejected(rejection) => add_compat_notice(
            app,
            format!("Error {}: {}", rejection.code, rejection.message),
        ),
    }
    true
}

fn apply_pending_command_response(app: &mut App, message: &serde_json::Value) -> bool {
    let Some(request_id) = message.get("id").and_then(serde_json::Value::as_u64) else {
        return false;
    };
    let Some(pending) = app.pending_commands.remove(&request_id) else {
        return false;
    };

    if matches!(
        pending,
        super::PendingCommand::ProjectionSnapshot { .. }
            | super::PendingCommand::ProjectionEvents { .. }
    ) {
        app.projection_request_in_flight = false;
    }

    match (pending, message.get("result"), message.get("error")) {
        (super::PendingCommand::InitializeSession, Some(result), None) => {
            if let Some(session_id) = result.get("session_id").and_then(serde_json::Value::as_str) {
                app.projection_target_session_id = Some(session_id.to_owned());
                app.projection_session_id = None;
            }
        }
        (super::PendingCommand::InitializeSkills, Some(result), None) => {
            if let Some(skills) = result.get("skills") {
                app.registry.set_skills_from_json(skills);
            }
        }
        (super::PendingCommand::OpenSessionPicker, Some(result), None) => {
            match serde_json::from_value::<
                fabric::protocol::client::ClientMessage<
                    fabric::protocol::client::SessionListSnapshot,
                >,
            >(result.clone())
            .map_err(|error| error.to_string())
            .and_then(|message| message.into_v1().map_err(|error| error.to_string()))
            {
                Ok(list) if list.schema_version == fabric::SESSION_READ_MODEL_SCHEMA_VERSION => {
                    match serde_json::to_value(list.sessions)
                        .map_err(|error| error.to_string())
                        .and_then(|sessions| {
                            super::session_picker::SessionPicker::from_json(
                                &sessions,
                                app.app_state.session_id.clone(),
                            )
                            .map_err(|error| error.to_string())
                        }) {
                        Ok(picker) => app.session_picker = Some(picker),
                        Err(error) => app
                            .compat_transcript
                            .add_text(ChatRole::System, format!("无法打开会话列表：{error}")),
                    }
                }
                Ok(list) => app.compat_transcript.add_text(
                    ChatRole::System,
                    format!(
                        "无法打开会话列表：unsupported schema {}",
                        list.schema_version
                    ),
                ),
                Err(error) => app
                    .compat_transcript
                    .add_text(ChatRole::System, format!("无法打开会话列表：{error}")),
            }
        }
        (super::PendingCommand::OpenAgentInspector { focus }, Some(result), None) => {
            let agents = result.get("agents").unwrap_or(&serde_json::Value::Null);
            let parsed = if let Some(inspector) = app.agent_inspector.as_mut() {
                inspector.replace(agents)
            } else {
                super::agent_inspector::AgentInspector::from_json(agents, focus.as_deref())
                    .map(|inspector| app.agent_inspector = Some(inspector))
            };
            if let Err(error) = parsed {
                app.agent_inspector = None;
                app.compat_transcript.add_text(
                    ChatRole::System,
                    format!("无法打开 Agent sessions：{error}"),
                );
            }
        }
        (super::PendingCommand::OpenCheckpointPicker, Some(result), None) => {
            match serde_json::from_value::<fabric::CheckpointListSnapshot>(result.clone()) {
                Ok(snapshot) => {
                    match super::checkpoint_picker::CheckpointPicker::from_snapshot(snapshot) {
                        Ok(picker) => app.checkpoint_picker = Some(picker),
                        Err(error) => app.compat_transcript.add_text(
                            ChatRole::System,
                            format!("无法打开工作区检查点列表：{error}"),
                        ),
                    }
                }
                Err(error) => app.compat_transcript.add_text(
                    ChatRole::System,
                    format!("无法读取工作区检查点列表：{error}"),
                ),
            }
        }
        (
            super::PendingCommand::CheckpointFork {
                parent_session_id,
                prompt_index,
            },
            Some(result),
            None,
        ) => match serde_json::from_value::<fabric::SessionRecord>(result.clone()) {
            Ok(child) => {
                let child_session_id = child.id.0;
                if let Some(prompt_index) = prompt_index {
                    app.deferred_checkpoint_rewind = Some(super::DeferredCheckpointRewind {
                        parent_session_id,
                        child_session_id,
                        prompt_index,
                    });
                } else {
                    app.projection_target_session_id = Some(child_session_id.clone());
                    app.projection_session_id = None;
                    app.projection_polling = false;
                    app.compat_transcript.add_text(
                        ChatRole::System,
                        format!("已分叉并切换到历史会话：{child_session_id}"),
                    );
                }
            }
            Err(error) => app.compat_transcript.add_text(
                ChatRole::System,
                format!("daemon 返回的会话分支无效：{error}；未恢复代码"),
            ),
        },
        (super::PendingCommand::CheckpointRewind { child_session_id }, Some(_), None) => {
            if let Some(child_session_id) = child_session_id {
                app.projection_target_session_id = Some(child_session_id.clone());
                app.projection_session_id = None;
                app.projection_polling = false;
                app.compat_transcript.add_text(
                    ChatRole::System,
                    format!("代码已恢复；已切换到历史会话分支：{child_session_id}"),
                );
            } else {
                app.compat_transcript
                    .add_text(ChatRole::System, "代码检查点恢复完成".to_string());
            }
        }
        (super::PendingCommand::TransactionReview, Some(result), None) => {
            match serde_json::from_value::<fabric::TransactionReviewSnapshot>(result.clone()) {
                Ok(snapshot) => {
                    if let Some(detail) = app.detail.as_mut() {
                        detail.project_settlement(snapshot.settlement.clone());
                    }
                    app.compat_transcript.add_text(
                        ChatRole::System,
                        format!(
                            "Host review {:?}: {}",
                            snapshot.settlement.decision, snapshot.settlement.reason
                        ),
                    );
                }
                Err(error) => app.compat_transcript.add_text(
                    ChatRole::System,
                    format!("daemon 返回的 Host review snapshot 无效：{error}"),
                ),
            }
        }
        (super::PendingCommand::TransactionSettlementLatest, Some(result), None) => {
            match result
                .get("receipt")
                .cloned()
                .ok_or_else(|| "missing receipt".to_string())
                .and_then(|value| {
                    serde_json::from_value::<fabric::TransactionSettlementReceipt>(value)
                        .map_err(|error| error.to_string())
                }) {
                Ok(receipt) => {
                    if let Some(detail) = app.detail.as_mut() {
                        detail.project_settlement(receipt);
                    }
                }
                Err(error) => app.compat_transcript.add_text(
                    ChatRole::System,
                    format!("daemon 返回的 settlement receipt 无效：{error}"),
                ),
            }
        }
        (super::PendingCommand::ProjectionSnapshot { session_id }, Some(result), None) => {
            match serde_json::from_value::<
                fabric::protocol::client::ClientMessage<
                    fabric::protocol::client::SessionReadSnapshot,
                >,
            >(result.clone())
            .map_err(|error| error.to_string())
            .and_then(|message| message.into_v1().map_err(|error| error.to_string()))
            {
                Ok(snapshot) => {
                    if app.app_state.session_id.as_deref() != Some(session_id.as_str()) {
                        app.app_state.reset_execution_target_for_session();
                    }
                    let effects = super::reducer::reduce(
                        &mut app.app_state,
                        super::reducer::UiAction::ReadSnapshot(snapshot),
                    );
                    apply_projection_effects(app, effects);
                    if app.app_state.session_id.as_deref() == Some(session_id.as_str()) {
                        app.projection_target_session_id = Some(session_id.clone());
                        app.projection_session_id = Some(session_id);
                        app.projection_polling = true;
                    } else {
                        app.projection_polling = false;
                    }
                    app.projection_next_poll_at = app.clock.mono_now();
                }
                Err(error) => {
                    app.projection_polling = false;
                    app.compat_transcript.add_text(
                        ChatRole::System,
                        format!("Session projection snapshot rejected: {error}"),
                    );
                }
            }
        }
        (super::PendingCommand::ProjectionEvents { session_id }, Some(result), None) => {
            if app.projection_target_session_id.as_deref() != Some(session_id.as_str()) {
                return true;
            }
            match serde_json::from_value::<
                fabric::protocol::client::ClientMessage<fabric::protocol::client::SessionEventPage>,
            >(result.clone())
            .map_err(|error| error.to_string())
            .and_then(|message| message.into_v1().map_err(|error| error.to_string()))
            {
                Ok(page) => {
                    let effects = super::reducer::reduce(
                        &mut app.app_state,
                        super::reducer::UiAction::EventPage(page),
                    );
                    apply_projection_effects(app, effects);
                    app.projection_next_poll_at =
                        fabric::MonoTime(app.clock.mono_now().0.saturating_add(200));
                }
                Err(error) => {
                    app.projection_session_id = None;
                    app.projection_polling = false;
                    app.compat_transcript.add_text(
                        ChatRole::System,
                        format!("Session projection event page rejected: {error}"),
                    );
                }
            }
        }
        (super::PendingCommand::NewSession { clear_screen }, Some(result), None)
            if result
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                .is_some() =>
        {
            if clear_screen {
                app.compat_transcript = super::chat::ChatWidget::new(app.caps.clone());
                app.compat_projected_entries = 0;
            }
            let session_id = result
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            app.app_state.reset_execution_target_for_session();
            app.projection_target_session_id = Some(session_id.to_owned());
            app.projection_session_id = None;
            app.compat_transcript
                .add_text(ChatRole::System, format!("已创建新会话：{session_id}"));
        }
        (super::PendingCommand::InitializeSession, _, Some(error)) => {
            let message = error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("初始化会话失败");
            app.compat_transcript
                .add_text(ChatRole::System, format!("Error: {message}"));
        }
        (super::PendingCommand::InitializeSkills, _, Some(_)) => {
            // Startup catalog refresh is best-effort. Keep the TUI clean and
            // retain the built-in command registry when the daemon is unavailable.
        }
        (super::PendingCommand::CheckpointFork { .. }, _, Some(error)) => {
            let message = error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("会话分支创建失败");
            app.compat_transcript.add_text(
                ChatRole::System,
                format!("Error: {message}。未恢复代码，原会话保持不变。"),
            );
        }
        (super::PendingCommand::CheckpointRewind { .. }, _, Some(error)) => {
            let message = error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("工作区恢复失败");
            app.compat_transcript.add_text(
                ChatRole::System,
                format!("Error: {message}。请检查 terminal restore receipt 后重试或人工恢复。"),
            );
        }
        (super::PendingCommand::TransactionReview, _, Some(error)) => {
            let message = error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Host review action 失败");
            app.compat_transcript
                .add_text(ChatRole::System, format!("Error: {message}"));
        }
        (super::PendingCommand::TransactionSettlementLatest, _, Some(error)) => {
            // No prior receipt is a normal pending-review state. Other daemon
            // errors remain visible without fabricating local settlement.
            let message = error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("settlement receipt 查询失败");
            if message != "transaction settlement not found" {
                app.compat_transcript
                    .add_text(ChatRole::System, format!("Error: {message}"));
            }
        }
        (super::PendingCommand::ProjectionSnapshot { session_id }, _, Some(error)) => {
            if app.projection_target_session_id.as_deref() == Some(session_id.as_str()) {
                app.projection_target_session_id = app.app_state.session_id.clone();
                app.projection_session_id = app.app_state.session_id.clone();
                app.projection_polling = app.app_state.session_id.is_some();
            }
            let message = error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("恢复会话失败");
            app.compat_transcript.add_text(
                ChatRole::System,
                format!("Error: {message}。旧会话保持不变。"),
            );
        }
        (super::PendingCommand::ProjectionEvents { .. }, _, Some(error)) => {
            app.projection_session_id = None;
            app.projection_polling = false;
            let message = error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("会话事件读取失败");
            app.compat_transcript
                .add_text(ChatRole::System, format!("Error: {message}"));
        }
        (_, _, Some(error)) => {
            let message = error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("创建新会话失败");
            app.compat_transcript.add_text(
                ChatRole::System,
                format!("Error: {message}。旧会话和界面保持不变。"),
            );
        }
        _ => {
            app.compat_transcript.add_text(
                ChatRole::System,
                "Error: daemon 返回了无效的会话创建响应；旧会话和界面保持不变。".to_string(),
            );
        }
    }
    true
}

fn apply_projection_effects(app: &mut App, effects: Vec<super::reducer::UiEffect>) {
    for effect in effects {
        match effect {
            super::reducer::UiEffect::Render | super::reducer::UiEffect::SubscribeAfter(_) => {}
            super::reducer::UiEffect::ReloadSnapshot(session_id) => {
                if app.app_state.session_id.as_deref() == Some(session_id.0.as_str()) {
                    app.projection_session_id = None;
                    app.projection_polling = false;
                }
            }
            super::reducer::UiEffect::AnnounceError(message) => {
                app.compat_transcript.add_text(ChatRole::System, message);
            }
        }
    }
    if app
        .selected_activity
        .is_some_and(|index| index >= app.app_state.activities.len())
    {
        app.selected_activity = app.app_state.activities.len().checked_sub(1);
    }
}

fn apply_typed_protocol_event(app: &mut App, message: &serde_json::Value) -> bool {
    use super::reducer::{format_evaluation_receipt_ref, reduce, UiAction, UiError};
    use fabric::protocol::client::{ClientEvent as ProtocolEvent, ClientMessage, ItemPhase};

    let candidate = message
        .get("params")
        .or_else(|| message.get("result"))
        .unwrap_or(message);
    let claims_typed_protocol = candidate.get("protocol_version").is_some();
    let message = match serde_json::from_value::<ClientMessage<ProtocolEvent>>(candidate.clone()) {
        Ok(message) => message,
        Err(error) if claims_typed_protocol => {
            app.compat_transcript.add_text(
                ChatRole::System,
                format!("Typed client protocol rejected: {error}"),
            );
            return true;
        }
        Err(_) => return false,
    };
    let event = match message.into_v1() {
        Ok(event) => event,
        Err(error) => {
            app.compat_transcript.add_text(
                ChatRole::System,
                format!("Typed client protocol rejected: {error}"),
            );
            return true;
        }
    };
    let evaluation = match &event {
        ProtocolEvent::Item(item) if item.phase == ItemPhase::Completed => {
            item.item.as_ref().and_then(|record| match &record.payload {
                fabric::ItemPayload::EvaluationReceiptRef { receipt } => Some(receipt.clone()),
                _ => None,
            })
        }
        _ => None,
    };
    if matches!(
        &event,
        ProtocolEvent::TurnCompleted { .. } | ProtocolEvent::TurnStopped { .. }
    ) {
        return super::reducer::reduce_terminal(&mut app.app_state, &event);
    }
    if matches!(&event, ProtocolEvent::Failed { .. }) {
        let _ = super::reducer::reduce_terminal(&mut app.app_state, &event);
    }
    let action = match event {
        ProtocolEvent::InitializeResponse(_) => return true,
        ProtocolEvent::MemoryObservationReceipt(_)
        | ProtocolEvent::MemoryLifecycleReceipt(_)
        | ProtocolEvent::MemoryRecallResult(_)
        | ProtocolEvent::MemoryFeedbackReceipt(_)
        | ProtocolEvent::MemoryMaintenanceStatus(_)
        | ProtocolEvent::MemoryMaintenanceRunReceipt(_)
        | ProtocolEvent::MemoryWorkspaceBindingPreview(_)
        | ProtocolEvent::MemoryWorkspaceBinding(_) => return true,
        ProtocolEvent::Snapshot(value) => UiAction::Snapshot(value),
        ProtocolEvent::Item(value) => UiAction::Item(value),
        ProtocolEvent::Approval(value) => UiAction::Approval(value),
        ProtocolEvent::Agent(value) => UiAction::Agent(value),
        ProtocolEvent::Reconnected(value) => UiAction::Reconnected(value),
        ProtocolEvent::CommandCompleted { .. } => return true,
        ProtocolEvent::Failed { cursor, message } => UiAction::Failed(UiError { cursor, message }),
        ProtocolEvent::TurnStarted {
            thread_id, turn_id, ..
        } => {
            app.app_state.session_id = Some(thread_id.0);
            super::reducer::begin_live_turn(&mut app.app_state, Some(turn_id));
            return true;
        }
        ProtocolEvent::TurnCompleted { .. } | ProtocolEvent::TurnStopped { .. } => unreachable!(),
    };
    let effects = reduce(&mut app.app_state, action);
    if !effects.is_empty() {
        if let Some(receipt) = evaluation {
            app.compat_transcript
                .add_text(ChatRole::System, format_evaluation_receipt_ref(&receipt));
        }
    }
    true
}

/// Deduplicate consecutive identical text blocks.
/// Some models repeat thinking/reasoning text twice.
pub fn deduplicate_consecutive_text(text: &str) -> String {
    let midpoint = text.len() / 2;
    if text.len().is_multiple_of(2) && text.is_char_boundary(midpoint) {
        let (first, second) = text.split_at(midpoint);
        if first == second {
            return first.to_string();
        }
    }
    text.to_string()
}

/// Format sessions list for display.
pub fn format_sessions(sessions: &serde_json::Value) -> String {
    let empty = vec![];
    let arr = sessions.as_array().unwrap_or(&empty);
    if arr.is_empty() {
        return "No sessions found.".to_string();
    }
    let mut lines = Vec::new();
    lines.push(format!("=== Sessions ({}) ===\n", arr.len()));
    for entry in arr {
        let id = entry.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let created = entry.get("created").and_then(|v| v.as_str()).unwrap_or("");
        let turns = entry
            .get("turn_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let summary = entry.get("summary").and_then(|v| v.as_str()).unwrap_or("");
        let short_id = &id[..8.min(id.len())];
        lines.push(format!("[{short_id}] {created} ({turns} turns) {summary}"));
    }
    lines.join("\n")
}

/// Format model list for display.
pub fn format_models(result: &serde_json::Value) -> String {
    let empty = vec![];
    let models = result
        .get("models")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    let current = result
        .get("current")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    if models.is_empty() {
        return "No models available.".to_string();
    }
    let mut lines = Vec::new();
    lines.push("=== Available Models ===".to_string());
    for entry in models {
        let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let desc = entry
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let marker = if name == current { " (current)" } else { "" };
        lines.push(format!("  {name}{marker} - {desc}"));
    }
    lines.push(String::new());
    lines.push("Use /model <name> to switch.".to_string());
    lines.join("\n")
}

/// Format status response for display.
pub fn format_status(status: &serde_json::Value) -> String {
    match serde_json::from_value::<fabric::contract::command::StatusProjectionV1>(status.clone()) {
        Ok(status) => format_status_projection(&status),
        Err(error) => format!("Invalid typed status projection: {error}"),
    }
}

pub fn format_status_projection(status: &fabric::contract::command::StatusProjectionV1) -> String {
    let session_id = status.session_id.as_str();

    let mut lines = Vec::new();
    lines.push("=== Aletheon Status ===".to_string());
    lines.push(format!(
        "Session: {}",
        &session_id[..8.min(session_id.len())]
    ));
    lines.push(format!("Turns: {}", status.turn_count));
    lines.push(format!("Reflections: {}", status.reflection_count));
    lines.push(format!("Evolutions: {}", status.evolution_count));
    lines.push(format!(
        "Compactions: {}/{} successful",
        status.compaction.successful, status.compaction.attempts
    ));
    if let Some(last) = &status.compaction.last {
        lines.push(format!(
            "Last compaction: {} → {} ({})",
            last.tokens_before, last.tokens_after, last.strategy
        ));
    }
    lines.push(String::new());
    lines.push("Memory:".to_string());
    lines.push(format!("  Provider: {}", status.memory.provider));
    lines.push(format!("  Local: {}", status.memory.local));
    lines.push(format!(
        "  Supplemental: {} (queue depth {})",
        if status.memory.supplemental.enabled {
            status.memory.supplemental.state.as_str()
        } else {
            "disabled"
        },
        status.memory.supplemental.queue_depth
    ));
    lines.push(String::new());
    lines.push("Care Weights:".to_string());

    for care in &status.care_weights {
        lines.push(format!("  {}: {:.2}", care.topic, care.weight));
    }

    lines.push(String::new());
    lines.push(format!(
        "Boundary Rules: {} (immutable: {})",
        status.boundary_rules, status.boundary_immutable
    ));

    let focus_display = if status.attention_focus.is_empty() {
        "none"
    } else {
        status.attention_focus.as_str()
    };
    lines.push(format!("Attention Focus: {focus_display}"));

    lines.join("\n")
}

pub fn format_tools_list(tools: &serde_json::Value) -> String {
    let empty = vec![];
    let arr = tools.as_array().unwrap_or(&empty);
    if arr.is_empty() {
        return "No tools registered.".to_string();
    }
    let mut lines = Vec::new();
    lines.push(format!("=== Tools ({}) ===\n", arr.len()));
    for t in arr {
        let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let desc = t.get("description").and_then(|v| v.as_str()).unwrap_or("");
        let short_desc = if desc.len() > 60 { &desc[..60] } else { desc };
        lines.push(format!("  {name} — {short_desc}"));
    }
    lines.join("\n")
}

pub fn format_agents(agents: &serde_json::Value) -> String {
    let empty = vec![];
    let arr = agents.as_array().unwrap_or(&empty);
    if arr.is_empty() {
        return "No sub-agents running.".to_string();
    }
    let mut lines = Vec::new();
    lines.push(format!("=== Sub-Agents ({}) ===\n", arr.len()));
    for a in arr {
        let id = a.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let task = a.get("task").and_then(|v| v.as_str()).unwrap_or("?");
        let status = a.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        lines.push(format!("  {id} [{status}] — {task}"));
    }
    lines.join("\n")
}

pub fn format_skills_list(skills: &serde_json::Value) -> String {
    let empty = vec![];
    let arr = skills.as_array().unwrap_or(&empty);
    if arr.is_empty() {
        return "No skills available.".to_string();
    }
    let mut lines = Vec::new();
    lines.push(format!("=== Skills ({}) ===\n", arr.len()));
    for sk in arr {
        let name = sk.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let desc = sk.get("description").and_then(|v| v.as_str()).unwrap_or("");
        lines.push(format!("  /{name} — {desc}"));
    }
    // Phase B: when SkillsCatalog response arrives, call
    // app.registry.set_skills(skills);
    lines.join("\n")
}

pub fn format_memory_facts(facts: &serde_json::Value) -> String {
    let empty = vec![];
    let arr = facts.as_array().unwrap_or(&empty);
    if arr.is_empty() {
        return "=== Memory Facts ===\n\n(no facts stored yet)".to_string();
    }
    let mut lines = vec![format!("=== Memory Facts ({}) ===\n", arr.len())];
    for fact in arr.iter().take(30) {
        let content = fact.get("content").and_then(|v| v.as_str()).unwrap_or("?");
        let category = fact.get("category").and_then(|v| v.as_str()).unwrap_or("");
        let trust = fact
            .get("trust_score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let pinned = fact
            .get("pinned")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let pin = if pinned { " 📌" } else { "" };
        lines.push(format!(
            "  [{category}] trust={trust:.2}{pin}\n    {content}"
        ));
    }
    if arr.len() > 30 {
        lines.push(format!("  ... and {} more facts", arr.len() - 30));
    }
    lines.join("\n")
}

pub fn format_memory_status(memory: &serde_json::Value) -> String {
    let provider = memory
        .get("provider")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let local = memory
        .get("local")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let supplemental = memory.get("supplemental");
    let enabled = supplemental
        .and_then(|value| value.get("enabled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let state = supplemental
        .and_then(|value| value.get("state"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let queue_depth = supplemental
        .and_then(|value| value.get("queue_depth"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    format!(
        "=== Memory Status ===\nProvider: {provider}\nLocal: {local}\nSupplemental: {} (queue depth {queue_depth})",
        if enabled { state } else { "disabled" }
    )
}

#[cfg(test)]
mod tests {
    use super::{
        deduplicate_consecutive_text, format_memory_status, handle_event, process_response,
        public_tool_args,
    };
    use crate::tui::{
        chat::ChatEntry, host_time::ClientClock, term_compat::TermCaps, App, PendingCommand,
    };
    use executive::application::{tool_stream_bridge::ToolStreamHandle, turn_pipeline};
    use fabric::{
        ipc::{StreamConfig, TurnEventStream, TurnEventV1},
        ToolProgress, ToolResult, ToolResultMeta,
    };
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn public_tool_arguments_are_bounded_and_redacted() {
        let value = public_tool_args(&serde_json::json!({
            "path": "/workspace/src/lib.rs",
            "authorization": "Bearer visible-secret",
            "nested": {"api_key": "sk-secret"},
            "command": "run OPENAI_API_KEY=inline-secret tool",
            "query": "x".repeat(300),
        }));

        assert_eq!(value["authorization"], "[redacted]");
        assert_eq!(value["nested"]["api_key"], "[redacted]");
        assert!(!value.to_string().contains("visible-secret"));
        assert!(!value.to_string().contains("sk-secret"));
        assert!(!value.to_string().contains("inline-secret"));
        assert!(value["query"].as_str().unwrap().ends_with('…'));
    }

    #[test]
    fn deduplicates_only_an_exact_repeated_response() {
        assert_eq!(deduplicate_consecutive_text("完整回答完整回答"), "完整回答");
        assert_eq!(
            deduplicate_consecutive_text("abcabc trailing"),
            "abcabc trailing"
        );
    }

    #[test]
    fn memory_status_reports_local_and_supplemental_health() {
        let rendered = format_memory_status(&serde_json::json!({
            "provider": "composite",
            "local": "healthy",
            "supplemental": {
                "enabled": true,
                "state": "degraded",
                "queue_depth": 3
            }
        }));
        assert!(rendered.contains("Provider: composite"));
        assert!(rendered.contains("Local: healthy"));
        assert!(rendered.contains("Supplemental: degraded (queue depth 3)"));
    }

    #[test]
    fn preserves_markdown_that_starts_with_repeated_rule_characters() {
        let response = "----------------------------------------\n邮件分析结果\n- 重点一\n- 重点二";
        assert_eq!(deduplicate_consecutive_text(response), response);
    }

    #[tokio::test]
    async fn typed_command_completion_uses_the_shared_versioned_contract() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = App::new(
            stream,
            TermCaps {
                color: true,
                true_color: false,
                unicode: false,
                width: 80,
                height: 24,
            },
            "test".into(),
            Arc::new(ClientClock::new()),
            workspace,
            Vec::new(),
        );
        let output = fabric::contract::command::CommandOutputEnvelopeV1::new(
            "tui:1",
            fabric::contract::command::CommandOutputV1::PromptCompleted(
                fabric::contract::command::PromptCompletionV1 {
                    response: "typed answer".into(),
                    stop: fabric::TurnStop::Completed,
                    failure: None,
                    usage: Default::default(),
                    metrics: Default::default(),
                },
            ),
        );

        handle_event(
            &mut app,
            &serde_json::json!({"type": "text_snapshot", "text": "typed answer"}),
        );

        process_response(&mut app, serde_json::json!({"id": 1, "result": output}));

        assert!(matches!(
            app.compat_transcript.entries.last(),
            Some(ChatEntry::Text(message)) if message.content == "typed answer"
        ));
        assert_eq!(
            app.app_state
                .items
                .values()
                .filter(|item| item.kind == "assistant")
                .count(),
            1,
            "multiple completion transports must replace the same transient item"
        );

        process_response(
            &mut app,
            serde_json::json!({"id": 2, "result": {"response": "typed answer"}}),
        );
        assert_eq!(
            app.app_state
                .items
                .values()
                .filter(|item| item.kind == "assistant")
                .count(),
            1,
            "legacy and typed completion envelopes must not render separately"
        );
    }

    #[tokio::test]
    async fn typed_cancel_ack_is_not_parsed_as_a_status_projection() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = App::new(
            stream,
            TermCaps {
                color: true,
                true_color: false,
                unicode: false,
                width: 80,
                height: 24,
            },
            "test".into(),
            Arc::new(ClientClock::new()),
            workspace,
            Vec::new(),
        );
        let output = fabric::contract::command::CommandOutputEnvelopeV1::new(
            "cancel:1",
            fabric::contract::command::CommandOutputV1::CancelRequested(
                fabric::contract::command::CancelRequestedV1 { active_turns: 1 },
            ),
        );

        process_response(&mut app, serde_json::json!({"id": 1, "result": output}));

        assert!(app.compat_transcript.entries.iter().any(|entry| {
            matches!(entry, ChatEntry::Text(message)
                if message.content == "Cancellation requested for 1 active turn(s).")
        }));
        assert!(!app.compat_transcript.entries.iter().any(|entry| {
            matches!(entry, ChatEntry::Text(message)
                if message.content.contains("Invalid typed status projection"))
        }));
    }

    #[tokio::test]
    async fn claimed_command_protocol_mismatch_is_rejected_without_legacy_fallback() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = App::new(
            stream,
            TermCaps {
                color: true,
                true_color: false,
                unicode: false,
                width: 80,
                height: 24,
            },
            "test".into(),
            Arc::new(ClientClock::new()),
            workspace,
            Vec::new(),
        );

        process_response(
            &mut app,
            serde_json::json!({
                "id": 1,
                "result": {
                    "protocol": "command_output",
                    "schema_version": 99,
                    "correlation_id": "tui:bad",
                    "output": {"kind": "prompt_accepted"},
                    "response": "must not be guessed as a legacy response"
                }
            }),
        );

        assert!(app.compat_transcript.entries.iter().any(|entry| {
            matches!(entry, ChatEntry::Text(message)
                if message.content.contains("unsupported command output schema 99"))
        }));
        assert!(!app.compat_transcript.entries.iter().any(|entry| {
            matches!(entry, ChatEntry::Text(message)
                if message.content.contains("must not be guessed"))
        }));
    }

    #[tokio::test]
    async fn startup_skill_catalog_updates_registry_without_rendering_chat() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let caps = TermCaps {
            color: true,
            true_color: false,
            unicode: false,
            width: 80,
            height: 24,
        };
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = App::new(
            stream,
            caps,
            "test".into(),
            Arc::new(ClientClock::new()),
            workspace,
            Vec::new(),
        );
        app.pending_commands
            .insert(7, PendingCommand::InitializeSkills);

        process_response(
            &mut app,
            serde_json::json!({
                "id": 7,
                "result": {
                    "skills": [{
                        "id": "test-skill",
                        "name": "test-skill",
                        "description": "must remain hidden at startup"
                    }]
                }
            }),
        );

        assert!(app.registry.is_skill("test-skill"));
        assert!(app.compat_transcript.entries.is_empty());
    }

    #[tokio::test]
    async fn tui_review_projects_the_exact_host_settlement_receipt() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = App::new(
            stream,
            TermCaps {
                color: true,
                true_color: false,
                unicode: false,
                width: 80,
                height: 24,
            },
            "test".into(),
            Arc::new(ClientClock::new()),
            workspace,
            Vec::new(),
        );
        app.detail = Some(crate::tui::diff_view::DiffView::new("diff"));
        app.pending_commands
            .insert(8, PendingCommand::TransactionSettlementLatest);
        let receipt = fabric::TransactionSettlementReceipt {
            settlement_id: "settlement-1".into(),
            transaction_id: "transaction-1".into(),
            session_id: "session-1".into(),
            workspace_version: "version-1".into(),
            decision: fabric::TransactionSettlementDecision::RepairRequired,
            finding_ids: vec!["finding-1".into()],
            validation_receipt_refs: vec!["artifact://validation".into()],
            validation_omissions: vec![],
            reason: "required validation failed".into(),
        };

        process_response(
            &mut app,
            serde_json::json!({"id": 8, "result": {"receipt": receipt.clone()}}),
        );

        assert_eq!(
            app.detail.and_then(|detail| detail.settlement),
            Some(receipt)
        );
    }

    #[tokio::test]
    async fn fork_and_rewind_waits_for_authoritative_fork_response() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let caps = TermCaps {
            color: true,
            true_color: false,
            unicode: false,
            width: 80,
            height: 24,
        };
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = App::new(
            stream,
            caps,
            "test".into(),
            Arc::new(ClientClock::new()),
            workspace,
            Vec::new(),
        );
        app.pending_commands.insert(
            9,
            PendingCommand::CheckpointFork {
                parent_session_id: "parent".into(),
                prompt_index: Some(4),
            },
        );
        let child = fabric::SessionRecord {
            schema_version: fabric::SESSION_SCHEMA_VERSION,
            id: fabric::SessionId("child".into()),
            parent: Some(fabric::SessionFork {
                session_id: fabric::SessionId("parent".into()),
                through_sequence: 12,
            }),
            created_at_ms: 1,
            status: fabric::SessionStatus::Active,
        };

        process_response(&mut app, serde_json::json!({"id": 9, "result": child}));

        let deferred = app
            .deferred_checkpoint_rewind
            .expect("rewind must be deferred until the fork succeeds");
        assert_eq!(deferred.parent_session_id, "parent");
        assert_eq!(deferred.child_session_id, "child");
        assert_eq!(deferred.prompt_index, 4);
        assert_eq!(app.projection_target_session_id, None);
    }

    #[tokio::test]
    async fn error_event_releases_the_active_turn() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let caps = TermCaps {
            color: true,
            true_color: false,
            unicode: false,
            width: 80,
            height: 24,
        };
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = App::new(
            stream,
            caps,
            "test".into(),
            Arc::new(ClientClock::new()),
            workspace,
            Vec::new(),
        );
        app.streaming = true;
        app.status.waiting = true;
        app.app_state.streaming = true;
        app.turn_active = true;
        app.app_state.turn_active = true;

        handle_event(
            &mut app,
            &serde_json::json!({"type": "error", "message": "compaction failed"}),
        );

        assert!(!app.streaming);
        assert!(!app.status.waiting);
        assert!(!app.app_state.streaming);
        assert!(!app.turn_active);
        assert!(!app.app_state.turn_active);
    }

    #[tokio::test]
    async fn multi_round_usage_accumulates_without_resetting_the_active_turn() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = App::new(
            stream,
            TermCaps {
                color: true,
                true_color: false,
                unicode: false,
                width: 80,
                height: 24,
            },
            "test".into(),
            Arc::new(ClientClock::new()),
            workspace,
            Vec::new(),
        );

        for event in [
            fabric::ui_event::ClientEvent::TurnStarted { iteration: 0 },
            fabric::ui_event::ClientEvent::TurnStarted { iteration: 1 },
            fabric::ui_event::ClientEvent::Usage {
                usage: fabric::InferenceUsage::unsupported(Some(100), Some(10)),
            },
            fabric::ui_event::ClientEvent::ToolCallStart {
                call_id: "call-1".into(),
                tool: "file_read".into(),
                args: serde_json::Value::Null,
            },
            fabric::ui_event::ClientEvent::TurnStarted { iteration: 2 },
            fabric::ui_event::ClientEvent::Usage {
                usage: fabric::InferenceUsage::unsupported(Some(200), Some(20)),
            },
            fabric::ui_event::ClientEvent::ContextUpdate {
                used_tokens: 200,
                max_tokens: 1_000_000,
            },
        ] {
            handle_event(&mut app, &serde_json::to_value(event).unwrap());
        }

        assert_eq!(app.app_state.turn_input_tokens, 300);
        assert_eq!(app.app_state.turn_output_tokens, 30);
        assert_eq!(app.app_state.total_tokens, 330);
        assert_eq!(app.app_state.turn_activity.inference_rounds, 2);
        assert_eq!(app.app_state.turn_activity.tool_calls, 1);
        assert_eq!(app.app_state.current_iteration, 2);
        let inference = app
            .app_state
            .activities
            .iter()
            .filter(|activity| activity.activity_id.contains(":inference:"))
            .collect::<Vec<_>>();
        assert_eq!(inference.len(), 2);
        assert!(inference
            .iter()
            .all(|activity| activity.state == fabric::ActivityState::Completed));
        assert_eq!(app.app_state.context.used, Some(200));
        assert_eq!(app.app_state.context.max, Some(1_000_000));
    }

    #[tokio::test]
    async fn terminal_text_snapshot_replaces_an_incomplete_stream() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let caps = TermCaps {
            color: true,
            true_color: false,
            unicode: false,
            width: 80,
            height: 24,
        };
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = App::new(
            stream,
            caps,
            "test".into(),
            Arc::new(ClientClock::new()),
            workspace,
            Vec::new(),
        );

        handle_event(
            &mut app,
            &serde_json::json!({"type": "text_delta", "text": "partial |---"}),
        );
        handle_event(
            &mut app,
            &serde_json::json!({
                "type": "text_snapshot",
                "text": "complete authoritative answer."
            }),
        );

        assert_eq!(
            app.stream_ctrl.current_text(),
            "complete authoritative answer."
        );
        assert!(matches!(
            app.compat_transcript.entries.last(),
            Some(ChatEntry::Text(message))
                if message.content == "complete authoritative answer."
        ));
    }

    #[tokio::test]
    async fn patch_progress_is_materialized_immediately_in_chat() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let caps = TermCaps {
            color: true,
            true_color: false,
            unicode: false,
            width: 80,
            height: 24,
        };
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = App::new(
            stream,
            caps,
            "test".into(),
            Arc::new(ClientClock::new()),
            workspace,
            Vec::new(),
        );
        let event = fabric::ui_event::ClientEvent::PatchProgress {
            status: "file_changed".into(),
            path: Some("src/lib.rs".into()),
            operation: Some("update".into()),
            error: None,
            applied_count: None,
            failed_count: None,
        };

        handle_event(&mut app, &serde_json::to_value(event).unwrap());

        assert!(app.compat_transcript.entries.iter().any(|entry| {
            matches!(entry, ChatEntry::Text(message)
                if message.content.contains("Patch file_changed")
                    && message.content.contains("src/lib.rs"))
        }));
    }

    #[tokio::test]
    async fn governed_tool_progress_reaches_tui_and_keeps_one_terminal() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let caps = TermCaps {
            color: true,
            true_color: false,
            unicode: false,
            width: 80,
            height: 24,
        };
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = App::new(
            stream,
            caps,
            "test".into(),
            Arc::new(ClientClock::new()),
            workspace,
            Vec::new(),
        );
        handle_event(
            &mut app,
            &serde_json::to_value(fabric::ui_event::ClientEvent::TurnStarted { iteration: 0 })
                .unwrap(),
        );
        assert!(!app
            .app_state
            .activities
            .iter()
            .any(|activity| activity.activity_id.contains(":inference:")));
        handle_event(
            &mut app,
            &serde_json::to_value(fabric::ui_event::ClientEvent::TurnStarted { iteration: 1 })
                .unwrap(),
        );
        handle_event(
            &mut app,
            &serde_json::to_value(fabric::ui_event::ClientEvent::Reflection {
                summary: "Inspecting known entry files before scoped discovery".into(),
            })
            .unwrap(),
        );
        assert!(app.app_state.activities.iter().any(|activity| {
            activity.activity_id.ends_with(":inference:0")
                && activity.state == fabric::ActivityState::Running
        }));
        assert!(app.app_state.activities.iter().any(|activity| {
            activity.activity_id.ends_with(":progress")
                && activity
                    .label
                    .contains("Inspecting known entry files before scoped discovery")
        }));
        handle_event(
            &mut app,
            &serde_json::to_value(fabric::ui_event::ClientEvent::ToolCallStart {
                call_id: "call-e2e".into(),
                tool: "bash_exec".into(),
                args: serde_json::Value::Null,
            })
            .unwrap(),
        );
        assert!(app.app_state.activities.iter().any(|activity| {
            activity.activity_id.ends_with(":tool:call-e2e")
                && activity.label == "bash_exec"
                && activity.state == fabric::ActivityState::Running
        }));

        let ToolStreamHandle { mut sink, event_rx } = ToolStreamHandle::new();
        let (mut daemon_stream, daemon_sender) =
            TurnEventStream::new(StreamConfig::turn_events(16));
        let bridge_sender = daemon_sender.clone();
        let bridge = tokio::spawn(async move {
            executive::application::tool_stream_bridge::bridge_tool_stream(
                event_rx,
                bridge_sender,
                "bash_exec".into(),
                "call-e2e".into(),
                CancellationToken::new(),
            )
            .await
        });

        assert!(sink.progress(ToolProgress::Structured(serde_json::json!({"line": 1}))));
        assert!(sink.progress(ToolProgress::Structured(serde_json::json!({"line": 2}))));
        sink.terminal(Ok(ToolResult {
            content: "finished".into(),
            is_error: false,
            metadata: ToolResultMeta::default(),
        }))
        .await;
        let outcome = bridge.await.unwrap();
        let terminal = outcome.terminal.unwrap();

        // Production settlement emits the unique authoritative ToolResult only
        // after the progress bridge returns its single terminal.
        daemon_sender
            .send(&TurnEventV1::ToolResult {
                name: "bash_exec".into(),
                call_id: "call-e2e".into(),
                content: terminal.content,
                is_error: terminal.is_error,
                execution_time_ms: terminal.metadata.execution_time_ms,
                patch_delta: None,
            })
            .unwrap();

        let mut progress_count = 0;
        let mut terminal_count = 0;
        let mut tui_progress_observations = 0;
        while let Some(event) = daemon_stream.try_recv() {
            let event = event.unwrap();
            match &event {
                TurnEventV1::ToolProgress { .. } => progress_count += 1,
                TurnEventV1::ToolResult { .. } => terminal_count += 1,
                other => panic!("unexpected daemon turn event: {other:?}"),
            }
            let client = turn_pipeline::turn_event_to_client_event(&event).unwrap();
            handle_event(&mut app, &serde_json::to_value(client).unwrap());
            if matches!(event, TurnEventV1::ToolProgress { .. }) {
                let visible = app.compat_transcript.entries.iter().any(|entry| {
                    matches!(entry, ChatEntry::Exec(execution)
                        if execution.call_id == "call-e2e" && !execution.output.is_empty())
                });
                assert!(visible, "TUI must materialize each progress event");
                tui_progress_observations += 1;
            }
        }

        assert_eq!(progress_count, 2);
        assert_eq!(tui_progress_observations, 2);
        assert_eq!(terminal_count, 1);
        let execution = app
            .compat_transcript
            .entries
            .iter()
            .find_map(|entry| match entry {
                ChatEntry::Exec(execution) if execution.call_id == "call-e2e" => Some(execution),
                _ => None,
            })
            .unwrap();
        assert!(execution.finished);
        assert_eq!(execution.output, "finished");
        assert!(
            app.app_state.activities.iter().any(|activity| {
                activity.activity_id.ends_with(":tool:call-e2e")
                    && activity.state == fabric::ActivityState::Completed
                    && activity.progress.as_ref().is_some_and(|progress| {
                        progress.get("status").and_then(serde_json::Value::as_str)
                            == Some("completed")
                    })
            }),
            "canonical activities after terminal: {:#?}",
            app.app_state.activities
        );
    }
}

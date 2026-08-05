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
                app.chat.add_text(ChatRole::System, "连接断开".to_string());
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
                            app.chat.add_text(
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
            app.stream_ctrl.start_turn();
            app.status.waiting = false;
            app.status.elapsed_secs = 0.0;
            app.turn_active = true;
            app.streaming = true;
            app.app_state.streaming = true;
            app.app_state.turn_tool_count = 0;
            app.app_state.turn_activity = super::state::TurnActivity::default();
            app.turn_tokens = None;
            app.current_iteration = iteration;
        }
        ClientEvent::ThinkingDelta { text } => {
            app.stream_ctrl.push_thinking(&text);
            app.chat
                .set_assistant_stream(app.stream_ctrl.current_text());
        }
        ClientEvent::TextDelta { text } => {
            app.stream_ctrl.push_text(&text);
            app.chat
                .set_assistant_stream(app.stream_ctrl.current_text());
        }
        ClientEvent::TextSnapshot { text } => {
            app.stream_ctrl.replace_text(&text);
            app.chat
                .set_assistant_stream(app.stream_ctrl.current_text());
        }
        ClientEvent::ToolCallStart {
            call_id,
            tool,
            args,
        } => {
            app.chat.discard_trailing_assistant_draft();
            let args_str = serde_json::to_string(&args).unwrap_or_default();
            app.chat.add_exec(call_id.clone(), tool.clone(), args_str);
            app.app_state.turn_tool_count += 1;
            app.app_state.turn_activity.tool_calls += 1;
        }
        ClientEvent::ToolCallComplete {
            call_id,
            tool: _,
            args,
        } => {
            let args_str = serde_json::to_string(&args).unwrap_or_default();
            app.chat.update_exec_args(&call_id, &args_str);
        }
        ClientEvent::ToolCallResult {
            call_id,
            output,
            is_error,
            patch_delta,
            ..
        } => {
            if let Some(preview) = patch_delta
                .as_ref()
                .and_then(|delta| delta.diff_preview.clone())
            {
                app.latest_diff = Some(preview);
            }
            app.chat
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
            let progress = payload
                .as_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| payload.to_string());
            app.chat.update_exec_progress(&call_id, &progress);
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
            app.chat.add_text(
                ChatRole::System,
                format!("Patch {status}: {operation} {target} {detail}")
                    .trim_end()
                    .to_owned(),
            );
        }
        ClientEvent::Usage { usage } => {
            app.app_state.turn_activity.inference_rounds += 1;
            let tokens_in = usage.total_input_tokens.unwrap_or(0);
            let tokens_out = usage.output_tokens.unwrap_or(0);
            app.turn_tokens = Some((tokens_in as u32, tokens_out as u32));
            app.total_tokens = app
                .total_tokens
                .saturating_add(tokens_in as u32)
                .saturating_add(tokens_out as u32);
            app.status.token_count = Some(tokens_in as u32 + tokens_out as u32);
            app.status.total_tokens = app.total_tokens;
        }
        ClientEvent::TurnDone => {
            app.stream_ctrl.commit();
            app.chat
                .set_assistant_stream(app.stream_ctrl.current_text());
            app.streaming = false;
            app.status.waiting = false;
            app.app_state.streaming = false;
            app.turn_active = false;
            app.status.session_turns += 1;
        }
        ClientEvent::Error { message } => {
            app.chat
                .add_text(ChatRole::System, format!("Error: {message}"));
            app.streaming = false;
            app.status.waiting = false;
            app.app_state.streaming = false;
            app.turn_active = false;
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
            app.app_state.context.used = used_tokens as usize;
            app.app_state.context.max = max_tokens as usize;
            app.status.context_used_tokens = used_tokens as u32;
            app.status.context_window = max_tokens as u32;
        }
        ClientEvent::ModelSwitch { model } => {
            app.app_state.model_name = model.clone();
            app.model_name = model.clone();
            app.status.model_name = model;
        }
        ClientEvent::Interrupted => {
            app.chat
                .add_text(ChatRole::System, "Interrupted".to_string());
        }
        ClientEvent::BudgetExceeded { limit } => {
            app.chat
                .add_text(ChatRole::System, format!("Budget exceeded: {limit} tokens"));
        }
        ClientEvent::CircuitBreakerTripped { reason } => {
            app.chat
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
            app.chat.add_text(
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
                app.chat.add_text(ChatRole::System, summary);
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
    if apply_typed_protocol_event(app, &msg) {
        return;
    }
    if let Some(result) = msg.get("result") {
        if let Some(text) = result.get("response").and_then(|v| v.as_str()) {
            // Standard chat response - deduplicate consecutive identical text
            // Some models repeat thinking/reasoning text
            let deduped = deduplicate_consecutive_text(text);
            app.chat.set_assistant_stream(deduped);
        } else if let Some(status) = result.get("status") {
            // /status response — rich self-evolution state
            let formatted = format_status(status);
            app.chat.set_assistant_stream(formatted);
        } else if let Some(sessions) = result.get("sessions") {
            // /sessions response
            let formatted = format_sessions(sessions);
            app.chat.set_assistant_stream(formatted);
        } else if let Some(_models) = result.get("models") {
            // /model response
            let formatted = format_models(result);
            app.chat.set_assistant_stream(formatted);
        } else if let Some(skills) = result.get("skills") {
            // Phase B: SkillsCatalog response — populate the command registry
            // so Tab-completion and /help reflect daemon skills.
            app.registry.set_skills_from_json(skills);
            let formatted = format_skills_list(skills);
            app.chat.set_assistant_stream(formatted);
        } else if let Some(facts) = result.get("facts") {
            // /memory response — render fact list
            let formatted = format_memory_facts(facts);
            app.chat.set_assistant_stream(formatted);
        } else if let Some(memory) = result.get("memory") {
            // /memory status response
            app.chat.set_assistant_stream(format_memory_status(memory));
        } else if let Some(receipt) = result.get("receipt") {
            if receipt.is_null() {
                app.chat.add_text(
                    ChatRole::System,
                    "No evaluation receipt is available for this session.".to_string(),
                );
            } else {
                match serde_json::from_value::<fabric::EvaluationReceiptRef>(receipt.clone()) {
                    Ok(receipt) => {
                        app.app_state.latest_evaluation = Some(receipt.clone());
                        app.chat.add_text(
                            ChatRole::System,
                            super::reducer::format_evaluation_receipt_ref(&receipt),
                        );
                    }
                    Err(error) => app.chat.add_text(
                        ChatRole::System,
                        format!("Invalid evaluation receipt response: {error}"),
                    ),
                }
            }
        } else if let Some(content) = result.get("content").and_then(|value| value.as_str()) {
            // session.memory returns bounded markdown owned by the daemon.
            app.chat.set_assistant_stream(content.to_string());
        } else if let Some(tools) = result.get("tools") {
            // tools/list response
            let formatted = format_tools_list(tools);
            app.chat.set_assistant_stream(formatted);
        } else if let Some(agents) = result.get("agents") {
            // /agents response
            let formatted = format_agents(agents);
            app.chat.set_assistant_stream(formatted);
        } else if let Some(msg_text) = result.get("message").and_then(|v| v.as_str()) {
            // Generic message response (e.g. /resume, /compact)
            app.chat.set_assistant_stream(msg_text.to_string());
        }
    } else if let Some(error) = msg.get("error") {
        let err = error
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        app.chat.add_text(ChatRole::System, format!("Error: {err}"));
    }
    // NOTE: Do NOT clear streaming/waiting here. The JSON-RPC result arrives
    // BEFORE the turn_done event. Clearing streaming here causes a visible UI
    // freeze between tool calls. Let turn_done handle the state transition.
    //
    // Also do NOT clear response_buf — streaming events may follow in the
    // same try_read chunk.
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
                            .chat
                            .add_text(ChatRole::System, format!("无法打开会话列表：{error}")),
                    }
                }
                Ok(list) => app.chat.add_text(
                    ChatRole::System,
                    format!(
                        "无法打开会话列表：unsupported schema {}",
                        list.schema_version
                    ),
                ),
                Err(error) => app
                    .chat
                    .add_text(ChatRole::System, format!("无法打开会话列表：{error}")),
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
                    app.chat.add_text(
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
                    app.chat.add_text(
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
                app.chat = super::chat::ChatWidget::new(app.caps.clone());
            }
            let session_id = result
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            app.projection_target_session_id = Some(session_id.to_owned());
            app.projection_session_id = None;
            app.chat
                .add_text(ChatRole::System, format!("已创建新会话：{session_id}"));
        }
        (super::PendingCommand::InitializeSession, _, Some(error)) => {
            let message = error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("初始化会话失败");
            app.chat
                .add_text(ChatRole::System, format!("Error: {message}"));
        }
        (super::PendingCommand::InitializeSkills, _, Some(_)) => {
            // Startup catalog refresh is best-effort. Keep the TUI clean and
            // retain the built-in command registry when the daemon is unavailable.
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
            app.chat.add_text(
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
            app.chat
                .add_text(ChatRole::System, format!("Error: {message}"));
        }
        (_, _, Some(error)) => {
            let message = error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("创建新会话失败");
            app.chat.add_text(
                ChatRole::System,
                format!("Error: {message}。旧会话和界面保持不变。"),
            );
        }
        _ => {
            app.chat.add_text(
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
                app.chat.add_text(ChatRole::System, message);
            }
        }
    }
}

fn apply_typed_protocol_event(app: &mut App, message: &serde_json::Value) -> bool {
    use super::reducer::{format_evaluation_receipt_ref, reduce, UiAction, UiError};
    use fabric::protocol::client::{ClientEvent as ProtocolEvent, ClientMessage, ItemPhase};

    let candidate = message
        .get("params")
        .or_else(|| message.get("result"))
        .unwrap_or(message);
    let Ok(message) = serde_json::from_value::<ClientMessage<ProtocolEvent>>(candidate.clone())
    else {
        return false;
    };
    let Ok(event) = message.into_v1() else {
        return false;
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
        ProtocolEvent::TurnStarted { .. } => return true,
        ProtocolEvent::TurnCompleted { .. } | ProtocolEvent::TurnStopped { .. } => unreachable!(),
    };
    let effects = reduce(&mut app.app_state, action);
    if !effects.is_empty() {
        if let Some(receipt) = evaluation {
            app.chat
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
    let session_id = status
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let turn_count = status
        .get("turn_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let reflection_count = status
        .get("reflection_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let evolution_count = status
        .get("evolution_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let boundary_rules = status
        .get("boundary_rules")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let boundary_immutable = status
        .get("boundary_immutable")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let attention_focus = status
        .get("attention_focus")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut lines = Vec::new();
    lines.push("=== Aletheon Status ===".to_string());
    lines.push(format!(
        "Session: {}",
        &session_id[..8.min(session_id.len())]
    ));
    lines.push(format!("Turns: {turn_count}"));
    lines.push(format!("Reflections: {reflection_count}"));
    lines.push(format!("Evolutions: {evolution_count}"));
    if let Some(compaction) = status.get("compaction") {
        let attempts = compaction
            .get("attempts")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let successful = compaction
            .get("successful")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        lines.push(format!("Compactions: {successful}/{attempts} successful"));
        if let Some(last) = compaction.get("last").filter(|value| !value.is_null()) {
            let before = last
                .get("tokens_before")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let after = last
                .get("tokens_after")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let strategy = last
                .get("strategy")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            lines.push(format!("Last compaction: {before} → {after} ({strategy})"));
        }
    }
    if let Some(memory) = status.get("memory") {
        lines.push(String::new());
        lines.push("Memory:".to_string());
        lines.push(format!(
            "  Provider: {}",
            memory
                .get("provider")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
        ));
        lines.push(format!(
            "  Local: {}",
            memory
                .get("local")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
        ));
        if let Some(supplemental) = memory.get("supplemental") {
            let enabled = supplemental
                .get("enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let state = supplemental
                .get("state")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let depth = supplemental
                .get("queue_depth")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            lines.push(format!(
                "  Supplemental: {} (queue depth {depth})",
                if enabled { state } else { "disabled" }
            ));
        }
    }
    lines.push(String::new());
    lines.push("Care Weights:".to_string());

    if let Some(cares) = status.get("care_weights").and_then(|v| v.as_array()) {
        for care in cares {
            let topic = care.get("topic").and_then(|v| v.as_str()).unwrap_or("?");
            let weight = care.get("weight").and_then(|v| v.as_f64()).unwrap_or(0.0);
            lines.push(format!("  {topic}: {weight:.2}"));
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "Boundary Rules: {boundary_rules} (immutable: {boundary_immutable})"
    ));

    let focus_display = if attention_focus.is_empty() {
        "none"
    } else {
        attention_focus
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
    async fn startup_skill_catalog_updates_registry_without_rendering_chat() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let caps = TermCaps {
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
        assert!(app.chat.entries.is_empty());
    }

    #[tokio::test]
    async fn error_event_releases_the_active_turn() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let caps = TermCaps {
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

        handle_event(
            &mut app,
            &serde_json::json!({"type": "error", "message": "compaction failed"}),
        );

        assert!(!app.streaming);
        assert!(!app.status.waiting);
        assert!(!app.app_state.streaming);
        assert!(!app.turn_active);
    }

    #[tokio::test]
    async fn terminal_text_snapshot_replaces_an_incomplete_stream() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let caps = TermCaps {
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
            app.chat.entries.last(),
            Some(ChatEntry::Text(message))
                if message.content == "complete authoritative answer."
        ));
    }

    #[tokio::test]
    async fn patch_progress_is_materialized_immediately_in_chat() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let caps = TermCaps {
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

        assert!(app.chat.entries.iter().any(|entry| {
            matches!(entry, ChatEntry::Text(message)
                if message.content.contains("Patch file_changed")
                    && message.content.contains("src/lib.rs"))
        }));
    }

    #[tokio::test]
    async fn governed_tool_progress_reaches_tui_and_keeps_one_terminal() {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let caps = TermCaps {
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
            &serde_json::to_value(fabric::ui_event::ClientEvent::ToolCallStart {
                call_id: "call-e2e".into(),
                tool: "bash_exec".into(),
                args: serde_json::Value::Null,
            })
            .unwrap(),
        );

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
                let visible = app.chat.entries.iter().any(|entry| {
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
            .chat
            .entries
            .iter()
            .find_map(|entry| match entry {
                ChatEntry::Exec(execution) if execution.call_id == "call-e2e" => Some(execution),
                _ => None,
            })
            .unwrap();
        assert!(execution.finished);
        assert_eq!(execution.output, "finished");
    }
}

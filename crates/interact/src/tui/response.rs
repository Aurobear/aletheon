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
                let chunk = String::from_utf8_lossy(&app.read_buf[..n]);
                app.response_buf.push_str(&chunk);

                while let Some(newline_pos) = app.response_buf.find('\n') {
                    let line = app.response_buf[..newline_pos].trim().to_string();
                    app.response_buf.drain(..=newline_pos);

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
        ClientEvent::ToolCallStart {
            call_id,
            tool,
            args,
        } => {
            app.chat.discard_trailing_assistant_draft();
            let args_str = serde_json::to_string(&args).unwrap_or_default();
            app.chat.add_exec(call_id.clone(), tool.clone(), args_str);
            app.app_state.turn_tool_count += 1;
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
            ..
        } => {
            app.chat.update_exec(&call_id, &output, is_error);
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
        ClientEvent::Usage {
            tokens_in,
            tokens_out,
        } => {
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
            // compaction is internal, just note it
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
        app.pending_approval = Some(super::approval_dialog::ApprovalDialog::new(
            approval_id,
            tool,
            action_summary,
            risk_level,
        ));
    }
}

pub fn process_response(app: &mut App, msg: serde_json::Value) {
    if apply_typed_protocol_event(app, &msg) {
        return;
    }
    if let Some(result) = msg.get("result") {
        if let Some(text) = result.get("response").and_then(|v| v.as_str()) {
            // Standard chat response - deduplicate consecutive identical text
            // Some models repeat thinking/reasoning text
            let deduped = deduplicate_consecutive_text(text);
            app.chat.set_assistant_stream(deduped);
        } else if let Some(entries) = result.get("reflections") {
            // /reflect response — format reflection entries
            let formatted = format_reflections(entries);
            app.chat.set_assistant_stream(formatted);
        } else if let Some(genome) = result.get("genome") {
            // /genome response — format genome JSON
            let formatted = format_genome(genome);
            app.chat.set_assistant_stream(formatted);
        } else if let Some(evo) = result.get("evolution") {
            // /evolution response — format evolution history
            let formatted = format_evolution(evo);
            app.chat.set_assistant_stream(formatted);
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
        } else if let Some(hooks) = result.get("hooks") {
            // /hooks response
            let formatted = format_hooks(hooks);
            app.chat.set_assistant_stream(formatted);
        } else if let Some(skills) = result.get("skills") {
            // Phase B: SkillsCatalog response — populate the command registry
            // so Tab-completion and /help reflect daemon skills.
            app.registry.set_skills_from_json(skills);
            let formatted = format_skills_list(skills);
            app.chat.set_assistant_stream(formatted);
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

fn apply_typed_protocol_event(app: &mut App, message: &serde_json::Value) -> bool {
    use super::reducer::{reduce, UiAction, UiError};
    use fabric::protocol::client::{ClientEvent as ProtocolEvent, ClientMessage};

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
    let _effects = reduce(&mut app.app_state, action);
    true
}

/// Deduplicate consecutive identical text blocks.
/// Some models repeat thinking/reasoning text twice.
pub fn deduplicate_consecutive_text(text: &str) -> String {
    let midpoint = text.len() / 2;
    if text.len() % 2 == 0 && text.is_char_boundary(midpoint) {
        let (first, second) = text.split_at(midpoint);
        if first == second {
            return first.to_string();
        }
    }
    text.to_string()
}

/// Format reflection entries for display.
pub fn format_reflections(entries: &serde_json::Value) -> String {
    let empty = vec![];
    let arr = entries.as_array().unwrap_or(&empty);
    if arr.is_empty() {
        return "No reflections found.".to_string();
    }
    let mut lines = Vec::new();
    lines.push(format!("=== Reflections ({}) ===\n", arr.len()));
    for (i, entry) in arr.iter().enumerate() {
        let _trigger = entry
            .get("trigger")
            .and_then(|v| {
                if let Some(s) = v.as_str() {
                    Some(s.to_string())
                } else {
                    serde_json::to_string(v).ok()
                }
            })
            .unwrap_or_else(|| "unknown".to_string());
        let task = entry
            .get("task_summary")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let outcome = entry
            .get("outcome")
            .and_then(|v| {
                if let Some(s) = v.as_str() {
                    Some(s.to_string())
                } else {
                    serde_json::to_string(v).ok()
                }
            })
            .unwrap_or_else(|| "unknown".to_string());
        let confidence = entry
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let timestamp = entry
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        lines.push(format!(
            "[{}] #{} {} ({}) conf={:.0}%",
            timestamp,
            i + 1,
            task,
            outcome,
            confidence * 100.0
        ));

        if let Some(arr) = entry.get("learned").and_then(|v| v.as_array()) {
            for l in arr {
                if let Some(s) = l.as_str() {
                    lines.push(format!("  learned: {s}"));
                }
            }
        }
        if let Some(arr) = entry.get("behavior_changes").and_then(|v| v.as_array()) {
            for c in arr {
                if let Some(s) = c.as_str() {
                    lines.push(format!("  changed: {s}"));
                }
            }
        }
        if let Some(arr) = entry.get("what_worked").and_then(|v| v.as_array()) {
            for w in arr {
                if let Some(s) = w.as_str() {
                    lines.push(format!("  worked: {s}"));
                }
            }
        }
        if let Some(arr) = entry.get("what_failed").and_then(|v| v.as_array()) {
            for f in arr {
                if let Some(s) = f.as_str() {
                    lines.push(format!("  failed: {s}"));
                }
            }
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

/// Format genome for display.
pub fn format_genome(genome: &serde_json::Value) -> String {
    if let Some(s) = genome.as_str() {
        return s.to_string();
    }
    serde_json::to_string_pretty(genome).unwrap_or_else(|_| format!("{genome:?}"))
}

/// Format evolution history for display.
pub fn format_evolution(evo: &serde_json::Value) -> String {
    if let Some(s) = evo.as_str() {
        return s.to_string();
    }
    if let Some(arr) = evo.as_array() {
        if arr.is_empty() {
            return "No evolution history found.".to_string();
        }
        let mut lines = Vec::new();
        lines.push(format!("=== Evolution History ({}) ===\n", arr.len()));
        for entry in arr {
            lines
                .push(serde_json::to_string_pretty(entry).unwrap_or_else(|_| format!("{entry:?}")));
            lines.push(String::new());
        }
        return lines.join("\n");
    }
    // Handle object form with version/message fields
    serde_json::to_string_pretty(evo).unwrap_or_else(|_| format!("{evo:?}"))
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

pub fn format_hooks(hooks: &serde_json::Value) -> String {
    let empty = vec![];
    let arr = hooks.as_array().unwrap_or(&empty);
    if arr.is_empty() {
        return "No hooks registered.".to_string();
    }
    let mut lines = Vec::new();
    lines.push(format!("=== Hooks ({}) ===\n", arr.len()));
    for h in arr {
        let name = h.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let point = h.get("point").and_then(|v| v.as_str()).unwrap_or("?");
        let source = h.get("source").and_then(|v| v.as_str()).unwrap_or("?");
        let priority = h.get("priority").and_then(|v| v.as_i64()).unwrap_or(0);
        lines.push(format!(
            "  {name} [{point}] (priority: {priority}, source: {source})"
        ));
    }
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

#[cfg(test)]
mod tests {
    use super::{deduplicate_consecutive_text, handle_event};
    use crate::tui::{chat::ChatEntry, host_time::ClientClock, term_compat::TermCaps, App};
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
    fn preserves_markdown_that_starts_with_repeated_rule_characters() {
        let response = "----------------------------------------\n邮件分析结果\n- 重点一\n- 重点二";
        assert_eq!(deduplicate_consecutive_text(response), response);
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

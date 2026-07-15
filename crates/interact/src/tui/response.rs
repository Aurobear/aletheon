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
            app.streaming = false;
            app.status.waiting = false;
            app.app_state.streaming = false;
            app.turn_active = false;
            app.status.session_turns += 1;
        }
        ClientEvent::Error { message } => {
            app.chat
                .add_text(ChatRole::System, format!("Error: {}", message));
            app.streaming = false;
            app.status.waiting = false;
            app.app_state.streaming = false;
        }
        ClientEvent::AwarenessChanged { level, context } => {
            if let Ok(awareness_level) =
                serde_json::from_str::<AwarenessLevel>(&format!("\"{}\"", level))
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
            if let Ok(s) = serde_json::from_str::<SubAgentStatus>(&format!("\"{}\"", status)) {
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
            if let Ok(mode) = serde_json::from_str::<CollaborationMode>(&format!("\"{}\"", new)) {
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
            app.chat.add_text(
                ChatRole::System,
                format!("Budget exceeded: {} tokens", limit),
            );
        }
        ClientEvent::CircuitBreakerTripped { reason } => {
            app.chat
                .add_text(ChatRole::System, format!("Circuit breaker: {}", reason));
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
        app.chat
            .add_text(ChatRole::System, format!("Error: {}", err));
    }
    // NOTE: Do NOT clear streaming/waiting here. The JSON-RPC result arrives
    // BEFORE the turn_done event. Clearing streaming here causes a visible UI
    // freeze between tool calls. Let turn_done handle the state transition.
    //
    // Also do NOT clear response_buf — streaming events may follow in the
    // same try_read chunk.
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
                    lines.push(format!("  learned: {}", s));
                }
            }
        }
        if let Some(arr) = entry.get("behavior_changes").and_then(|v| v.as_array()) {
            for c in arr {
                if let Some(s) = c.as_str() {
                    lines.push(format!("  changed: {}", s));
                }
            }
        }
        if let Some(arr) = entry.get("what_worked").and_then(|v| v.as_array()) {
            for w in arr {
                if let Some(s) = w.as_str() {
                    lines.push(format!("  worked: {}", s));
                }
            }
        }
        if let Some(arr) = entry.get("what_failed").and_then(|v| v.as_array()) {
            for f in arr {
                if let Some(s) = f.as_str() {
                    lines.push(format!("  failed: {}", s));
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
    serde_json::to_string_pretty(genome).unwrap_or_else(|_| format!("{:?}", genome))
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
            lines.push(
                serde_json::to_string_pretty(entry).unwrap_or_else(|_| format!("{:?}", entry)),
            );
            lines.push(String::new());
        }
        return lines.join("\n");
    }
    // Handle object form with version/message fields
    serde_json::to_string_pretty(evo).unwrap_or_else(|_| format!("{:?}", evo))
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
        lines.push(format!(
            "[{}] {} ({} turns) {}",
            short_id, created, turns, summary
        ));
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
        lines.push(format!("  {}{} - {}", name, marker, desc));
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
    lines.push(format!("Turns: {}", turn_count));
    lines.push(format!("Reflections: {}", reflection_count));
    lines.push(format!("Evolutions: {}", evolution_count));
    lines.push(String::new());
    lines.push("Care Weights:".to_string());

    if let Some(cares) = status.get("care_weights").and_then(|v| v.as_array()) {
        for care in cares {
            let topic = care.get("topic").and_then(|v| v.as_str()).unwrap_or("?");
            let weight = care.get("weight").and_then(|v| v.as_f64()).unwrap_or(0.0);
            lines.push(format!("  {}: {:.2}", topic, weight));
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "Boundary Rules: {} (immutable: {})",
        boundary_rules, boundary_immutable
    ));

    let focus_display = if attention_focus.is_empty() {
        "none"
    } else {
        attention_focus
    };
    lines.push(format!("Attention Focus: {}", focus_display));

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
            "  {} [{}] (priority: {}, source: {})",
            name, point, priority, source
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
        lines.push(format!("  {} — {}", name, short_desc));
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
        lines.push(format!("  {} [{}] — {}", id, status, task));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::deduplicate_consecutive_text;

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
}

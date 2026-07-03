use std::io;

use base::ui_event::{
    AwarenessLevel, CollaborationMode, EvolutionStage, PlanUpdate, SubAgentStatus,
};

use super::chat::Role as ChatRole;
use super::plan_view::PlanVersion;
use super::test_infra::EventRecorder;
use super::toolcard::ToolCard;
use super::App;

/// Variant of `try_read_socket` that records events via `EventRecorder`.
pub fn try_read_socket_with_recorder(app: &mut App, event_recorder: &mut Option<EventRecorder>) {
    loop {
        match app.stream.try_read(&mut app.read_buf) {
            Ok(0) => {
                app.streaming = false;
                app.status.waiting = false;
                app.app_state.streaming = false;
                app.chat
                    .add_message(ChatRole::System, "连接断开".to_string());
                break;
            }
            Ok(n) => {
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
                app.streaming = false;
                app.status.waiting = false;
                app.app_state.streaming = false;
                break;
            }
        }
    }
}

pub fn handle_event(app: &mut App, params: &serde_json::Value) {
    let event_type = params.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match event_type {
        "turn_start" => {
            let iteration = params
                .get("iteration")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            app.stream_ctrl.start_turn();
            app.status.waiting = true;
            app.status.elapsed_secs = 0.0;
            app.turn_active = true;
            app.app_state.streaming = true;
            app.app_state.turn_active = true;
            app.app_state.turn_tool_count = 0;
            app.current_iteration = iteration;
            app.app_state.current_iteration = iteration;
        }
        "thinking_delta" => {
            if let Some(text) = params.get("text").and_then(|v| v.as_str()) {
                app.stream_ctrl.push_thinking(text);
            }
        }
        "text_delta" => {
            if let Some(text) = params.get("text").and_then(|v| v.as_str()) {
                app.stream_ctrl.push_text(text);
                app.chat.update_last_message(app.stream_ctrl.current_text());
            }
        }
        "tool_call_start" => {
            let call_id = params
                .get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool = params
                .get("tool")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            let args = params
                .get("args")
                .map(|v| v.to_string())
                .unwrap_or_default();
            // Compute summary before tool/args are moved into ToolCard
            let cmd_summary = args_summary(&tool, &args);
            app.active_tools
                .insert(call_id.clone(), ToolCard::new(call_id, tool, args));
            app.app_state.turn_tool_count = app.active_tools.len();
            // Show tool execution start in chat area — prevents "frozen" perception
            app.chat
                .add_message(ChatRole::System, format!("🔧 {}", cmd_summary));
        }
        "tool_call_result" => {
            let call_id = params.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(card) = app.active_tools.get_mut(call_id) {
                let output = params.get("output").and_then(|v| v.as_str()).unwrap_or("");
                let is_error = params
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                card.finish(output, is_error);
                // Show completion status in chat area
                let elapsed = params
                    .get("elapsed_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let emoji = if is_error { "❌" } else { "✅" };
                let output_preview: String = output.chars().take(120).collect();
                let preview = if output_preview.len() < output.len() {
                    format!("{}…", output_preview)
                } else {
                    output_preview
                };
                app.chat.add_message(
                    ChatRole::System,
                    format!(
                        "{} {} ({:.1}s): {}",
                        emoji,
                        card.tool,
                        elapsed as f64 / 1000.0,
                        preview
                    ),
                );
            }
        }
        "usage" => {
            let tokens_in = params
                .get("tokens_in")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            let tokens_out = params
                .get("tokens_out")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            app.turn_tokens = Some((tokens_in, tokens_out));
            app.status.token_count = Some(tokens_in + tokens_out);
            app.total_tokens += tokens_in + tokens_out;
            app.status.total_tokens = app.total_tokens;
            app.status.context_window = 128_000;
            app.app_state.total_tokens = app.total_tokens;
        }
        "turn_done" => {
            app.stream_ctrl.commit();
            app.streaming = false;
            app.turn_active = false;
            app.status.waiting = false;
            app.status.elapsed_secs = 0.0;
            app.status.session_turns += 1;
            app.app_state.streaming = false;
            app.app_state.turn_active = false;
            // Clear active tool cards — they've been rendered inline and are
            // no longer needed. Without this, finished tools persist on screen.
            app.active_tools.clear();
            app.app_state.turn_tool_count = 0;
        }
        "error" => {
            let msg = params
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            app.chat
                .add_message(ChatRole::System, format!("Error: {}", msg));
            app.streaming = false;
            app.status.waiting = false;
        }
        // ── New P2 events ──
        "awareness_changed" => {
            if let Some(level_val) = params.get("level") {
                if let Ok(level) = serde_json::from_value::<AwarenessLevel>(level_val.clone()) {
                    let context = params
                        .get("context")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    app.app_state.awareness.update(level, context);
                }
            }
        }
        "plan_update" => {
            if let Ok(update) = serde_json::from_value::<PlanUpdate>(params.clone()) {
                app.plan_view.add_version(PlanVersion {
                    version: update.version,
                    plan: update.plan,
                    critique: update.critique,
                });
                app.plan_view.set_ready(update.ready_for_approval);
            }
        }
        "sub_agent_status" => {
            let agent_id = params
                .get("agent_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(status_val) = params.get("status") {
                if let Ok(status) = serde_json::from_value::<SubAgentStatus>(status_val.clone()) {
                    if let Some(agent) = app.sub_agents.iter_mut().find(|a| a.id == agent_id) {
                        agent.status = status;
                    } else {
                        // If the agent is not yet tracked, add it with minimal info.
                        let task = params
                            .get("task")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        app.sub_agents.push(base::ui_event::SubAgentHandle {
                            id: agent_id,
                            task,
                            status,
                            parent_turn_id: String::new(),
                            spawned_at_ms: 0,
                        });
                    }
                }
            }
        }
        "mode_changed" => {
            if let Some(new_val) = params.get("new") {
                if let Ok(new_mode) = serde_json::from_value::<CollaborationMode>(new_val.clone()) {
                    app.app_state.mode = new_mode;
                }
            }
        }
        "context_update" => {
            // Daemon sends "used_tokens" and "max_tokens"
            let used = params
                .get("used_tokens")
                .and_then(|v| v.as_u64())
                .or_else(|| params.get("used").and_then(|v| v.as_u64()))
                .unwrap_or(0) as usize;
            let max = params
                .get("max_tokens")
                .and_then(|v| v.as_u64())
                .or_else(|| params.get("max").and_then(|v| v.as_u64()))
                .unwrap_or(200_000) as usize;
            app.app_state.context.used = used;
            app.app_state.context.max = max;
            // Also update the legacy status bar context_window
            app.status.context_window = max as u32;
        }
        "model_switch" => {
            let to = params
                .get("to")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            app.app_state.model_name = to.clone();
            app.model_name = to.clone();
            app.status.model_name = to;
        }
        "evolution_progress" => {
            if let Some(stage_val) = params.get("stage") {
                if let Ok(_stage) = serde_json::from_value::<EvolutionStage>(stage_val.clone()) {
                    // Evolution progress is informational; show inline message
                    let msg = format!(
                        "Evolution: {}",
                        serde_json::to_string(stage_val).unwrap_or_default()
                    );
                    app.chat.add_message(ChatRole::System, msg);
                }
            }
        }
        _ => {}
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
            app.chat.update_last_message(deduped);
        } else if let Some(entries) = result.get("reflections") {
            // /reflect response — format reflection entries
            let formatted = format_reflections(entries);
            app.chat.update_last_message(formatted);
        } else if let Some(genome) = result.get("genome") {
            // /genome response — format genome JSON
            let formatted = format_genome(genome);
            app.chat.update_last_message(formatted);
        } else if let Some(evo) = result.get("evolution") {
            // /evolution response — format evolution history
            let formatted = format_evolution(evo);
            app.chat.update_last_message(formatted);
        } else if let Some(status) = result.get("status") {
            // /status response — rich self-evolution state
            let formatted = format_status(status);
            app.chat.update_last_message(formatted);
        } else if let Some(sessions) = result.get("sessions") {
            // /sessions response
            let formatted = format_sessions(sessions);
            app.chat.update_last_message(formatted);
        } else if let Some(_models) = result.get("models") {
            // /model response
            let formatted = format_models(result);
            app.chat.update_last_message(formatted);
        } else if let Some(hooks) = result.get("hooks") {
            // /hooks response
            let formatted = format_hooks(hooks);
            app.chat.update_last_message(formatted);
        } else if let Some(tools) = result.get("tools") {
            // tools/list response
            let formatted = format_tools_list(tools);
            app.chat.update_last_message(formatted);
        } else if let Some(agents) = result.get("agents") {
            // /agents response
            let formatted = format_agents(agents);
            app.chat.update_last_message(formatted);
        } else if let Some(msg_text) = result.get("message").and_then(|v| v.as_str()) {
            // Generic message response (e.g. /resume, /compact)
            app.chat.update_last_message(msg_text.to_string());
        }
    } else if let Some(error) = msg.get("error") {
        let err = error
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        app.chat.update_last_message(format!("Error: {}", err));
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
fn deduplicate_consecutive_text(text: &str) -> String {
    let len = text.len();

    // Try to find the longest repeated prefix
    for split_pos in (1..=len / 2).rev() {
        // Ensure we split at a valid UTF-8 boundary
        if !text.is_char_boundary(split_pos) {
            continue;
        }

        let prefix = &text[..split_pos];
        let suffix = &text[split_pos..];

        // Check if the suffix starts with the same prefix
        if suffix.starts_with(prefix) {
            // Found a repeated block - return just the prefix
            return prefix.to_string();
        }
    }

    // No repeated block found, return original text
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

/// Extract a human-readable summary from tool args for chat display.
fn args_summary(tool: &str, args: &str) -> String {
    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(args) {
        match tool {
            "bash_exec" | "bash" => obj
                .get("command")
                .and_then(|v| v.as_str())
                .map(|c| format!("bash: {}", truncate(c, 80)))
                .unwrap_or_else(|| format!("{} executing…", tool)),
            "file_read" => obj
                .get("path")
                .and_then(|v| v.as_str())
                .map(|p| format!("read: {}", truncate(p, 80)))
                .unwrap_or_else(|| format!("{} …", tool)),
            "file_write" => obj
                .get("path")
                .and_then(|v| v.as_str())
                .map(|p| format!("write: {}", truncate(p, 80)))
                .unwrap_or_else(|| format!("{} …", tool)),
            "grep" => obj
                .get("pattern")
                .and_then(|v| v.as_str())
                .map(|p| format!("grep: {}", truncate(p, 80)))
                .unwrap_or_else(|| format!("{} …", tool)),
            "web_fetch" => obj
                .get("url")
                .and_then(|v| v.as_str())
                .map(|u| format!("fetch: {}", truncate(u, 80)))
                .unwrap_or_else(|| format!("{} …", tool)),
            _ => format!("{} …", tool),
        }
    } else {
        format!("{}: {}", tool, truncate(args, 60))
    }
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

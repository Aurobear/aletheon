//! Turn context assembly — inspection and introspection query handlers.
//!
//! Contains the handlers that build context views for external debug access:
//! memory, self-field, dasein-module, snapshot, and LLM-based ask.

use serde_json::{json, Value};
use tracing::{info, warn};

use base::message::{ContentBlock, Message, Role};

use super::gateway::SessionGateway;
use super::snapshot::SnapshotBuilder;

impl SessionGateway {
    // ── Phase B: Snapshot ────────────────────────────────────────────────

    pub(crate) async fn handle_snapshot(&self, id: &Value) -> Value {
        let state = self.state.lock().await;
        let messages = self.session_manager.lock().await;
        let perf = self.debug_handler.perf_counter();

        let markdown = SnapshotBuilder::build(
            &self.session_id,
            &state.goal_tracker,
            perf,
            &self.runtime_config,
            self.started_at,
            state.circuit_breaker_status.clone(),
            state.tool_budget_remaining,
            state.tool_budget_max,
            &state.recent_tools,
            state.consecutive_errors,
            state.iteration,
            state.plan_mode,
            messages.message_count(),
            state.storm_breaker_failure_count,
        );

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "session_id": self.session_id,
                "snapshot": markdown,
            }
        })
    }

    // ── Phase C: Subsystem queries ─────────────────────────────────────

    pub(crate) async fn handle_memory(&self, id: &Value, params: &Value) -> Value {
        let memory_type = params.get("type").and_then(|v| v.as_str()).unwrap_or("all");

        let mut md = String::from("# Memory\n\n");

        if memory_type == "all" || memory_type == "core" {
            let cm = self.core_memory.lock().await;
            md.push_str("## Core Memory Blocks\n\n");
            for (label, block) in cm.blocks() {
                md.push_str(&format!(
                    "### {}\n- char_limit: {}\n- read_only: {}\n\n{}\n\n",
                    label, block.char_limit, block.read_only, block.value
                ));
            }
        }

        if memory_type == "all" || memory_type == "recall" {
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            let rm = self.recall_memory.lock().await;
            md.push_str("## Recall Memory (Recent)\n\n");
            match rm.recent(limit) {
                Ok(entries) if !entries.is_empty() => {
                    for entry in &entries {
                        md.push_str(&format!("- **[{}]** {}\n", entry.entry_type, entry.content));
                    }
                }
                _ => {
                    md.push_str("*(no entries)*\n");
                }
            }
            md.push('\n');
        }

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "memory_type": memory_type, "content": md }
        })
    }

    pub(crate) async fn handle_self(&self, id: &Value, params: &Value) -> Value {
        let layer = params
            .get("layer")
            .and_then(|v| v.as_str())
            .unwrap_or("all");
        let sf = self.self_field.lock().await;

        let mut md = String::from("# SelfField State\n\n");

        if layer == "all" || layer == "identity" {
            md.push_str("## Identity\n");
            use base::Subsystem;
            md.push_str(&format!("- Name: {}\n", sf.name()));
            md.push_str(&format!("- Version: {}\n\n", sf.version()));
        }

        if layer == "all" || layer == "boundary" {
            md.push_str("## Boundary\n");
            let _boundary = sf.boundary();
            md.push_str("- Boundary layer active\n\n");
        }

        if layer == "all" || layer == "dasein" {
            md.push_str("## DaseinModule\n");
            if let Some(d) = sf.dasein() {
                let m = d.mood();
                md.push_str(&format!("- Mood: {:?}\n", m));
                md.push_str(&format!("- Sorge alive: {}\n", d.is_alive()));

                let ts = d.temporality();
                let tss = ts.to_snapshot();
                md.push_str(&format!(
                    "- Retention depth: {}\n",
                    tss.recent_retentions.len()
                ));
                md.push_str(&format!("- Tempo: {:.2}\n", tss.tempo));

                let w = d.world();
                md.push_str(&format!(
                    "- Bewandtnis nodes: {} nodes, {} edges\n",
                    w.node_count(),
                    w.edge_count()
                ));

                let sm = d.self_model();
                md.push_str(&format!("- Self-assertions: {}\n", sm.assertion_count()));

                let cs = d.care();
                let css = cs.to_snapshot();
                md.push_str(&format!("- Concerns: {}\n", css.concerns.len()));
                md.push_str(&format!(
                    "- Rhythm interval: {}ms\n",
                    css.rhythm_interval_ms
                ));
            } else {
                md.push_str("*(DaseinModule not enabled)*\n");
            }
            md.push('\n');
        }

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "layer": layer, "content": md }
        })
    }

    pub(crate) async fn handle_dasein(&self, id: &Value) -> Value {
        let sf = self.self_field.lock().await;
        let mut md = String::from("# DaseinModule State\n\n");

        if let Some(d) = sf.dasein() {
            md.push_str("## Stimmung (Mood)\n");
            md.push_str(&format!("- {:?}\n\n", d.mood()));

            md.push_str("## TemporalStream\n");
            let tss = d.temporality().to_snapshot();
            md.push_str(&format!(
                "- Recent retentions: {}\n",
                tss.recent_retentions.len()
            ));
            md.push_str(&format!(
                "- Present: action={:?}, mood_tone={:?}\n",
                tss.present.action, tss.present.mood_tone
            ));
            md.push_str(&format!("- Protentions: {}\n", tss.protentions.len()));
            md.push_str(&format!("- Tempo: {:.2}\n\n", tss.tempo));

            md.push_str("## Bewandtnisganzheit (World)\n");
            let ws = d.world().to_snapshot();
            md.push_str(&format!(
                "- Ready-to-hand: {} | Present-at-hand: {} | Unavailable: {}\n",
                ws.ready_to_hand.len(),
                ws.present_at_hand.len(),
                ws.unavailable.len()
            ));
            md.push_str(&format!(
                "- Ultimate concern: {:?}\n\n",
                ws.ultimate_concern
            ));

            md.push_str("## MutableSelfModel\n");
            let sms = d.self_model().to_snapshot();
            md.push_str(&format!(
                "- Current assertions: {}\n",
                sms.current_assertions.len()
            ));
            for a in &sms.current_assertions {
                md.push_str(&format!(
                    "  - \"{}\" (stability: {:.2})\n",
                    a.content, a.stability
                ));
            }
            md.push_str(&format!(
                "- Negated assertions: {}\n",
                sms.negated_assertions.len()
            ));
            md.push_str(&format!("- Possibilities: {}\n\n", sms.possibilities.len()));

            md.push_str("## CareStructure\n");
            let css = d.care().to_snapshot();
            md.push_str(&format!("- Projection: {:?}\n", css.projection));
            md.push_str(&format!("- Concerns: {}\n", css.concerns.len()));
            for c in &css.concerns {
                md.push_str(&format!(
                    "  - \"{}\" (urgency: {:.2})\n",
                    c.purpose, c.urgency
                ));
            }
            md.push_str(&format!(
                "- Rhythm interval: {}ms\n",
                css.rhythm_interval_ms
            ));
            md.push_str(&format!(
                "- Fallenness depth: {:.2}\n\n",
                css.fallenness_depth
            ));

            md.push_str("## SorgeLoop\n");
            md.push_str(&format!("- Alive: {}\n", d.is_alive()));
        } else {
            md.push_str("*(DaseinModule not enabled)*\n");
        }

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "content": md }
        })
    }

    // ── Phase E: Ask ───────────────────────────────────────────────────────

    pub(crate) async fn handle_ask(&self, id: &Value, params: &Value) -> Value {
        let question = params
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if question.is_empty() {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": "Missing required param: question" }
            });
        }

        // Build context: goal, memory, recent messages
        let state = self.state.lock().await;
        let sm = self.session_manager.lock().await;
        let cm = self.core_memory.lock().await;

        let goal_desc = state
            .goal_tracker
            .current_goal_description()
            .unwrap_or_else(|| "(no goal set)".into());

        let mut context_parts = vec![
            format!("# Current Session Context"),
            format!("## Goal\n{}", goal_desc),
            format!(
                "## Loop State\n- Iteration: {}\n- Plan mode: {}\n- Consecutive errors: {}",
                state.iteration,
                if state.plan_mode { "yes" } else { "no" },
                state.consecutive_errors,
            ),
        ];

        // Core memory blocks
        let blocks = cm.blocks();
        if !blocks.is_empty() {
            let mut mem_section = String::from("## Core Memory\n");
            for (label, block) in blocks {
                if !block.value.is_empty() {
                    mem_section.push_str(&format!("- **{}**: {}\n", label, block.value));
                }
            }
            context_parts.push(mem_section);
        }

        // Recent messages (last 10)
        let history = sm.history();
        if !history.is_empty() {
            let mut msg_section = String::from("## Recent Messages\n");
            for msg in history.iter().rev().take(10).rev() {
                let role_str = match msg.role {
                    Role::User => "User",
                    Role::Assistant => "Assistant",
                    Role::System => "System",
                };
                let content_str: String = msg
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.clone()),
                        ContentBlock::ToolUse { name, .. } => Some(format!("[tool: {}]", name)),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                let preview: String = if content_str.len() > 200 {
                    format!("{}...", &content_str[..200])
                } else {
                    content_str.to_string()
                };
                msg_section.push_str(&format!("- [{}] {}\n", role_str, preview));
            }
            context_parts.push(msg_section);
        }

        let context = context_parts.join("\n\n");

        // Build the ask message
        let system_message = format!(
            "You are an introspection query handler for a running AI agent session. \
             Below is the current session context. Answer the user's question based \
             ONLY on the information provided. If the information is insufficient, say so.\n\n\
             {}\n\n\
             ## Question\n{}",
            context, question
        );

        info!(
            question_len = question.len(),
            "session.ask: sending query to LLM"
        );

        // Call LLM with NO tools (read-only introspection, no tool execution)
        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: system_message,
            }],
        }];

        match self.llm.complete(&messages, &[]).await {
            Ok(response) => {
                let answer = response
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "question": question,
                        "answer": answer,
                    }
                })
            }
            Err(e) => {
                warn!(error = %e, "session.ask: LLM call failed");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32053, "message": format!("LLM query failed: {}", e) }
                })
            }
        }
    }
}

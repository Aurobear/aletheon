use super::RequestHandler;

use serde_json::json;
use tracing::{info, warn};

use base::hook::{HookContext, HookPoint};
use base::ui_event::{CollaborationMode, InterruptReason};
use base::ReflectionTrigger;
use std::collections::HashMap;

use super::format::expand_tilde;
use crate::r#impl::daemon::prefix_builder::PrefixBuilder;
use crate::r#impl::orchestration::store::WorkflowStore;
use crate::r#impl::orchestration::digraph::graph::{DiGraph, WorkflowDef};
use crate::r#impl::orchestration::digraph::state::GraphState;
use crate::session::store::SessionStore;
use crate::r#impl::daemon::session_manager::SessionManager;
use corpus::security::security::approval::ApprovalDecision;

impl RequestHandler {
    pub(super) async fn handle_rpc(
        &self,
        method: &str,
        id: serde_json::Value,
        request: serde_json::Value,
    ) -> serde_json::Value {
        match method {
            "clear" => {
                // Fire OnSessionEnd hook before clearing
                {
                    let (session_id, turn_count) = {
                        let sm = self.session_manager.lock().await;
                        (sm.session_id.clone(), sm.turn_count())
                    };
                    let hr = self.hook_registry.lock().await;
                    let ctx = HookContext {
                        point: HookPoint::OnSessionEnd,
                        session_id,
                        turn_count,
                        tool_name: None,
                        tool_input: None,
                        tool_result: None,
                        message: None,
                        metadata: HashMap::new(),
                    };
                    hr.execute(&ctx).await;
                }
                // Run configured on_session_end hook scripts
                if !self.hooks_config.on_session_end.is_empty() {
                    let hook_session_id = self.session_manager.lock().await.session_id.clone();
                    let hook_input = serde_json::json!({
                        "session_id": hook_session_id,
                        "cwd": std::env::current_dir().unwrap_or_default()
                    });
                    let _ = self
                        .run_hook_scripts(
                            &self.hooks_config.on_session_end,
                            &hook_input.to_string(),
                        )
                        .await;
                }
                // Distill session facts into FactStore
                {
                    let fs = self.fact_store.lock().await;
                    let sm = self.session_manager.lock().await;
                    let recent: Vec<_> = sm.history().iter().rev().take(10).collect();
                    for msg in &recent {
                        if matches!(msg.role, base::Role::User) {
                            for block in &msg.content {
                                if let base::ContentBlock::Text { text } = block {
                                    if text.len() > 20 {
                                        let lower = text.to_lowercase();
                                        if lower.contains("prefer") || lower.contains("always")
                                            || lower.contains("never") || lower.contains("remember")
                                        {
                                            let _ = fs.add_fact(text, "session", "", "", 0.6, "episodic", 14);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let _ = fs.decay_stale();
                }
                // Clear cancel token
                {
                    let mut ct = self.cancel_token.lock().await;
                    *ct = None;
                }
                let mut state = self.state.lock().await;
                state.pending_input = None;
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "status": "ok" }
                })
            }
            "reflect" => {
                let reflections = self.episodic_memory.lock().await.recall_reflections(10);
                match reflections {
                    Ok(entries) => {
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "reflections": entries }
                        })
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to recall reflections");
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32001, "message": format!("Reflection recall error: {}", e) }
                        })
                    }
                }
            }
            "status" => {
                let state = self.state.lock().await;
                let session_id = state.runtime.config().session_id.clone();
                let iteration = state.runtime.iteration();
                drop(state);
                let turn_count = self.session_manager.lock().await.turn_count();

                // Reflection and evolution counts from episodic memory
                let reflection_count = self
                    .episodic_memory
                    .lock()
                    .await
                    .reflection_count()
                    .unwrap_or(0);
                let evolution_count = self
                    .episodic_memory
                    .lock()
                    .await
                    .evolution_log_count()
                    .unwrap_or(0);

                // Care weights, boundary rules, and attention from SelfField
                let sf = self.self_field.lock().await;
                let care_weights: Vec<serde_json::Value> = sf
                    .care()
                    .all_cares()
                    .into_iter()
                    .map(|c| json!({ "topic": c.topic, "weight": c.weight }))
                    .collect();
                let boundary_total = sf.boundary().rule_count();
                let boundary_immutable = sf.boundary().immutable_rule_count();
                let attention_focus = sf
                    .attention()
                    .current_focus()
                    .map(|f| f.topic)
                    .unwrap_or_default();
                drop(sf);

                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "status": {
                            "session_id": session_id,
                            "turn_count": turn_count,
                            "iteration": iteration,
                            "reflection_count": reflection_count,
                            "evolution_count": evolution_count,
                            "care_weights": care_weights,
                            "boundary_rules": boundary_total,
                            "boundary_immutable": boundary_immutable,
                            "attention_focus": attention_focus,
                        }
                    }
                })
            }
            "genome" => {
                // Read the genome dynamically from SelfField using SelfReader.
                let self_field = self.self_field.lock().await;
                let reader = metacog::r#impl::meta_runtime::self_reader::SelfReader::new();
                match reader.read_genome(&*self_field).await {
                    Ok(genome) => match serde_yaml::to_string(&genome) {
                        Ok(yaml) => {
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": { "genome": yaml }
                            })
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to serialize genome to YAML");
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "error": { "code": -32004, "message": format!("Genome serialization error: {}", e) }
                            })
                        }
                    },
                    Err(e) => {
                        warn!(error = %e, "Failed to read genome from SelfField");
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32004, "message": format!("Genome read error: {}", e) }
                        })
                    }
                }
            }
            "evolution" => {
                // Return recent evolution log entries from episodic memory.
                match self.episodic_memory.lock().await.recall_evolution_logs(20) {
                    Ok(entries) => {
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "evolution": entries,
                                "current_version": "0.1.0"
                            }
                        })
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to recall evolution logs");
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32002, "message": format!("Evolution recall error: {}", e) }
                        })
                    }
                }
            }
            "reflect_now" => {
                // Run an immediate reflection on the current session state
                let (turn, session_id, iteration) = {
                    let state = self.state.lock().await;
                    let session_id = state.runtime.config().session_id.clone();
                    let iteration = state.runtime.iteration();
                    drop(state);
                    let turn = self.session_manager.lock().await.turn_count();
                    (turn, session_id, iteration)
                };

                let task_summary = format!(
                    "Session {} after {} turns (iteration {})",
                    session_id, turn, iteration
                );

                let mut what_worked = Vec::new();
                let mut what_failed = Vec::new();
                let mut learned = Vec::new();

                what_worked.push(format!("Session is active with {} turns", turn));
                what_worked.push(format!("Runtime iteration count: {}", iteration));

                if turn == 0 {
                    what_failed.push("No chat turns recorded yet".to_string());
                }

                // Check if there are recent reflections to draw from
                match self.episodic_memory.lock().await.recall_reflections(5) {
                    Ok(recent) if !recent.is_empty() => {
                        learned.push(format!("Reviewed {} recent reflections", recent.len()));
                        // Aggregate failure patterns
                        let failure_count: usize = recent.iter().map(|r| r.what_failed.len()).sum();
                        if failure_count > 0 {
                            what_failed.push(format!(
                                "{} failure items across recent reflections",
                                failure_count
                            ));
                        }
                    }
                    Ok(_) => {
                        learned.push("No prior reflections available for context".to_string());
                    }
                    Err(e) => {
                        what_failed.push(format!("Could not recall reflections: {}", e));
                    }
                }

                let has_failures = !what_failed.is_empty() || turn == 0;
                let entry = self.reflector.reflect_conversation(
                    &task_summary,
                    ReflectionTrigger::Manual,
                    !has_failures,
                    what_worked,
                    what_failed,
                    learned,
                );
                if let Err(e) = self.episodic_memory.lock().await.store_reflection(&entry) {
                    warn!(error = %e, "Failed to store manual reflection");
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32003, "message": format!("Reflect now error: {}", e) }
                    })
                } else {
                    info!(id = %entry.id, "Manual reflection stored via reflect_now");
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "reflection": {
                                "id": entry.id,
                                "timestamp": entry.timestamp.to_rfc3339(),
                                "task_summary": entry.task_summary,
                                "outcome": entry.outcome.to_string(),
                                "what_worked": entry.what_worked,
                                "what_failed": entry.what_failed,
                                "learned": entry.learned,
                                "confidence": entry.confidence,
                                "turn_count": turn,
                            }
                        }
                    })
                }
            }
            "sessions" => match SessionStore::new(&self.data_dir) {
                Ok(store) => match store.list_sessions() {
                    Ok(ids) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "sessions": ids }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32020, "message": format!("Session list error: {}", e) }
                    }),
                },
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32020, "message": format!("SessionStore init error: {}", e) }
                }),
            },
            "resume" => {
                let target_session_id = request["params"]["session_id"].as_str().unwrap_or("");
                if target_session_id.is_empty() {
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32021, "message": "Missing session_id parameter" }
                    })
                } else {
                    match SessionManager::recover(&self.data_dir, target_session_id).await {
                        Some(msgs) => {
                            match SessionManager::new(
                                &self.data_dir,
                                target_session_id.to_string(),
                                self.context_window,
                            )
                            .await
                            {
                                Ok(new_sm) => {
                                    let msg_count = msgs.len();
                                    *self.session_manager.lock().await = new_sm;
                                    info!(
                                        session_id = target_session_id,
                                        messages = msg_count,
                                        "Session resumed"
                                    );
                                    json!({
                                        "jsonrpc": "2.0",
                                        "id": id,
                                        "result": {
                                            "session_id": target_session_id,
                                            "recovered_messages": msg_count,
                                        }
                                    })
                                }
                                Err(e) => json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "error": { "code": -32021, "message": format!("SessionManager init error: {}", e) }
                                }),
                            }
                        }
                        None => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32021, "message": format!("No recoverable session: {}", target_session_id) }
                        }),
                    }
                }
            }
            "compact" => {
                let did_compact = {
                    let mut sm = self.session_manager.lock().await;
                    // Force compaction by temporarily lowering threshold
                    sm.force_compact(&*self.llm).await
                };
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "compacted": did_compact }
                })
            }
            "reload_skills" => {
                let count = {
                    let mut loader = self.skill_loader.lock().await;
                    loader.reload()
                };
                info!(count = count, "Skills reloaded via reload_skills RPC");

                // Rebuild the cached prefix with updated skills.
                // Note: core_memory snapshot is from boot; mid-session memory
                // changes ride the memory_queue, not the prefix.
                {
                    let loader = self.skill_loader.lock().await;
                    let cm = self.core_memory.lock().await;
                    let old_prefix = self.cached_prefix.lock().await;
                    let new_prefix =
                        PrefixBuilder::build(&self.config_prompt, loader.skills(), &cm);
                    if let Some(reason) = PrefixBuilder::diff_reason(&old_prefix, &new_prefix) {
                        info!(reason = %reason, "Prefix changed after skill reload (cache will miss)");
                    }
                    drop(old_prefix);
                    drop(cm);
                    drop(loader);
                    *self.cached_prefix.lock().await = new_prefix;
                }

                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "skills_loaded": count }
                })
            }
            "approval_response" => {
                // Resolve a pending approval request. The client sends this
                // in response to an "approval_request" notification.
                // Supports: "once" (approve this time), "always" (approve for session),
                //           "reject" (deny).
                let aid = request["params"]["approval_id"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let action = request["params"]["decision"]
                    .as_str()
                    .unwrap_or("reject")
                    .to_string();
                let tool_name = request["params"]["tool"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();

                let decision = match action.as_str() {
                    "once" => ApprovalDecision::Approve,
                    "always" => {
                        // Cache approval for this tool for the rest of the session
                        if !tool_name.is_empty() {
                            let mut approvals = self.session_approvals.lock().await;
                            approvals.insert(tool_name.clone(), true);
                            info!(tool = %tool_name, "Tool approved for session (always)");
                        }
                        ApprovalDecision::ApproveForSession
                    }
                    _ => ApprovalDecision::Deny,
                };

                if let Some(tx) = self.pending_approvals.lock().await.remove(&aid) {
                    let _ = tx.send(decision);
                    info!(approval_id = %aid, action = %action, "Approval resolved");
                } else {
                    warn!(approval_id = %aid, "No pending approval found for id");
                }
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "ok": true }
                })
            }
            "new_session" => {
                let new_id = uuid::Uuid::new_v4().to_string();
                // Fire OnSessionEnd for the outgoing session
                {
                    let (old_id, turn_count) = {
                        let sm = self.session_manager.lock().await;
                        (sm.session_id.clone(), sm.turn_count())
                    };
                    let hr = self.hook_registry.lock().await;
                    let ctx = HookContext {
                        point: HookPoint::OnSessionEnd,
                        session_id: old_id,
                        turn_count,
                        tool_name: None,
                        tool_input: None,
                        tool_result: None,
                        message: None,
                        metadata: HashMap::new(),
                    };
                    hr.execute(&ctx).await;
                }
                // Run configured on_session_end hook scripts
                if !self.hooks_config.on_session_end.is_empty() {
                    let hook_input = serde_json::json!({
                        "session_id": self.session_manager.lock().await.session_id.clone(),
                        "cwd": std::env::current_dir().unwrap_or_default()
                    });
                    let _ = self
                        .run_hook_scripts(
                            &self.hooks_config.on_session_end,
                            &hook_input.to_string(),
                        )
                        .await;
                }
                // Create new session and replace SessionManager
                match SessionManager::new(&self.data_dir, new_id.clone(), self.context_window).await {
                    Ok(new_sm) => {
                        // Register session in store
                        if let Ok(store) = SessionStore::new(&self.data_dir) {
                            let _ = store.create_session(&new_id);
                        }
                        *self.session_manager.lock().await = new_sm;
                        // Clear per-session approval cache
                        self.session_approvals.lock().await.clear();
                        // Fire OnSessionStart for the new session
                        {
                            let hr = self.hook_registry.lock().await;
                            let ctx = HookContext {
                                point: HookPoint::OnSessionStart,
                                session_id: new_id.clone(),
                                turn_count: 0,
                                tool_name: None,
                                tool_input: None,
                                tool_result: None,
                                message: None,
                                metadata: HashMap::new(),
                            };
                            hr.execute(&ctx).await;
                        }
                        info!(session_id = %new_id, "New session created");
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "session_id": new_id }
                        })
                    }
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32030, "message": format!("Failed to create session: {}", e) }
                    }),
                }
            }
            "load_recent" => {
                match SessionStore::new(&self.data_dir) {
                    Ok(store) => match store.most_recent() {
                        Ok(Some(recent_id)) => {
                            match SessionManager::recover(&self.data_dir, &recent_id).await {
                                Some(msgs) => {
                                    match SessionManager::new(
                                        &self.data_dir,
                                        recent_id.clone(),
                                        self.context_window,
                                    )
                                    .await
                                    {
                                        Ok(new_sm) => {
                                            let msg_count = msgs.len();
                                            *self.session_manager.lock().await = new_sm;
                                            info!(
                                                session_id = %recent_id,
                                                messages = msg_count,
                                                "Loaded most recent session"
                                            );
                                            json!({
                                                "jsonrpc": "2.0",
                                                "id": id,
                                                "result": {
                                                    "session_id": recent_id,
                                                    "recovered_messages": msg_count,
                                                }
                                            })
                                        }
                                        Err(e) => json!({
                                            "jsonrpc": "2.0",
                                            "id": id,
                                            "error": { "code": -32031, "message": format!("SessionManager init error: {}", e) }
                                        }),
                                    }
                                }
                                None => {
                                    // No recoverable journal — create fresh session with this id
                                    match SessionManager::new(
                                        &self.data_dir,
                                        recent_id.clone(),
                                        self.context_window,
                                    )
                                    .await
                                    {
                                        Ok(new_sm) => {
                                            *self.session_manager.lock().await = new_sm;
                                            info!(session_id = %recent_id, "Loaded recent session (no journal, fresh)");
                                            json!({
                                                "jsonrpc": "2.0",
                                                "id": id,
                                                "result": {
                                                    "session_id": recent_id,
                                                    "recovered_messages": 0,
                                                }
                                            })
                                        }
                                        Err(e) => json!({
                                            "jsonrpc": "2.0",
                                            "id": id,
                                            "error": { "code": -32031, "message": format!("SessionManager init error: {}", e) }
                                        }),
                                    }
                                }
                            }
                        }
                        Ok(None) => {
                            // No sessions exist at all — create a new one
                            let new_id = uuid::Uuid::new_v4().to_string();
                            match SessionManager::new(&self.data_dir, new_id.clone(), self.context_window).await
                            {
                                Ok(new_sm) => {
                                    if let Ok(store) = SessionStore::new(&self.data_dir) {
                                        let _ = store.create_session(&new_id);
                                    }
                                    *self.session_manager.lock().await = new_sm;
                                    json!({
                                        "jsonrpc": "2.0",
                                        "id": id,
                                        "result": { "session_id": new_id, "recovered_messages": 0 }
                                    })
                                }
                                Err(e) => json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "error": { "code": -32031, "message": format!("SessionManager init error: {}", e) }
                                }),
                            }
                        }
                        Err(e) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32031, "message": format!("SessionStore query error: {}", e) }
                        }),
                    },
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32031, "message": format!("SessionStore init error: {}", e) }
                    }),
                }
            }
            "model_list" => {
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "models": [
                            {"name": "default", "description": "Default model from config"},
                            {"name": "sonnet", "description": "Claude Sonnet"},
                            {"name": "opus", "description": "Claude Opus"},
                            {"name": "haiku", "description": "Claude Haiku"}
                        ],
                        "current": "default"
                    }
                })
            }
            "model_switch" => {
                let model = request["params"]["model"].as_str().unwrap_or("");
                info!(model = %model, "Model switch requested");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "status": "ok", "model": model }
                })
            }
            "interrupt" => {
                let reason = match request.get("params")
                    .and_then(|p| p.get("reason"))
                    .and_then(|r| r.as_str())
                    .unwrap_or("user_cancelled")
                {
                    "user_cancelled" => InterruptReason::UserCancelled,
                    "timeout" => InterruptReason::Timeout,
                    "budget_exceeded" => InterruptReason::BudgetExceeded,
                    _ => InterruptReason::UserCancelled,
                };
                {
                    let state = self.state.lock().await;
                    state.runtime.interrupt_flag().request(reason);
                }
                info!(reason = ?reason, "Interrupt requested");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "status": "interrupt_requested", "reason": format!("{:?}", reason) }
                })
            }
            "mode_switch" => {
                let mode_str = request.get("params")
                    .and_then(|p| p.get("mode"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("default");
                let mode = match mode_str {
                    "plan" => CollaborationMode::Plan,
                    "auto" => CollaborationMode::Auto,
                    "sandbox" => CollaborationMode::Sandbox,
                    _ => CollaborationMode::Default,
                };
                let old_mode;
                {
                    let mut state = self.state.lock().await;
                    old_mode = state.runtime.mode_router().current_mode();
                    state.runtime.mode_router_mut().set_mode(mode);
                }
                info!(old = ?old_mode, new = ?mode, "Collaboration mode switched");
                // Notify all connected clients about the mode change
                if let Some(ref tx) = self.notify_tx {
                    let notification = serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "event",
                        "params": {
                            "type": "mode_changed",
                            "mode": mode.display_name(),
                        }
                    });
                    let _ = tx.send(notification.to_string()).await;
                }
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "status": "mode_switched",
                        "old": old_mode.display_name(),
                        "new": mode.display_name()
                    }
                })
            }
            "sub_agents" => {
                let state = self.state.lock().await;
                let agents: Vec<_> = state.runtime.sub_agent_spawner().list().iter().map(|a| {
                    serde_json::json!({
                        "id": a.id,
                        "task": a.task,
                        "status": format!("{:?}", a.status),
                    })
                }).collect();
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "agents": agents }
                })
            }
            "hooks_list" => {
                let hr = self.hook_registry.lock().await;
                let hooks: Vec<serde_json::Value> = hr.list().iter().map(|h| {
                    serde_json::json!({
                        "name": h.name,
                        "source": h.source,
                        "point": format!("{:?}", h.point),
                        "priority": h.priority,
                        "script_path": h.script_path,
                    })
                }).collect();
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "hooks": hooks }
                })
            }
            "tools/list" => {
                let tools_arc = self.tools.clone();
                let reg = tools_arc.lock().await;
                let tools: Vec<serde_json::Value> = reg.definitions().iter().map(|d| {
                    serde_json::json!({
                        "name": d.name,
                        "description": d.description,
                        "input_schema": d.input_schema,
                    })
                }).collect();
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": tools }
                })
            }
            "memory.add" => {
                let p = &request["params"];
                let content = p["content"].as_str().unwrap_or("");
                let scope = p["scope"].as_str().unwrap_or("session");
                let subject = p["subject"].as_str().unwrap_or("");
                let tags = p["tags"].as_str().unwrap_or("");
                let fs = self.fact_store.lock().await;
                match fs.add_fact_governed(
                    content, "general", tags, scope, "explicit", subject, 0.7, "semantic", 0,
                ) {
                    Ok(fact_id) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "fact_id": fact_id }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32010, "message": e.to_string() }
                    }),
                }
            }
            "memory.list" => {
                let p = &request["params"];
                let scope = p["scope"].as_str();
                let all = p["all"].as_bool().unwrap_or(false);
                let fs = self.fact_store.lock().await;
                match fs.list_facts(scope, all, 50) {
                    Ok(rows) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "facts": rows }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32010, "message": e.to_string() }
                    }),
                }
            }
            "memory.search" => {
                let p = &request["params"];
                let query = p["query"].as_str().unwrap_or("");
                let scope = p["scope"].as_str();
                let fs = self.fact_store.lock().await;
                match fs.search_facts_governed(query, scope, false, 0.15, 20) {
                    Ok(rows) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "facts": rows }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32010, "message": e.to_string() }
                    }),
                }
            }
            "memory.show" => {
                let fact_id = request["params"]["id"].as_i64().unwrap_or(0);
                let fs = self.fact_store.lock().await;
                match fs.get_fact(fact_id) {
                    Ok(Some(row)) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "fact": row }
                    }),
                    Ok(None) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32011, "message": "fact not found" }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32010, "message": e.to_string() }
                    }),
                }
            }
            "memory.forget" => {
                let p = &request["params"];
                let fact_id = p["id"].as_i64().unwrap_or(0);
                let hard = p["hard"].as_bool().unwrap_or(false);
                let fs = self.fact_store.lock().await;
                let res = if hard {
                    fs.delete_fact(fact_id)
                } else {
                    fs.set_status(fact_id, "archived")
                };
                match res {
                    Ok(ok) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "ok": ok }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32010, "message": e.to_string() }
                    }),
                }
            }
            "memory.pin" | "memory.unpin" => {
                let fact_id = request["params"]["id"].as_i64().unwrap_or(0);
                let pin = method == "memory.pin";
                let fs = self.fact_store.lock().await;
                match fs.set_pinned(fact_id, pin) {
                    Ok(ok) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "ok": ok }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32010, "message": e.to_string() }
                    }),
                }
            }
            // ── Workflow persistence ────────────────────────────────────
            "workflow.save" => {
                let name = request["params"]["name"].as_str().unwrap_or("");
                if name.is_empty() {
                    return json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32602, "message": "Missing 'name' parameter" }
                    });
                }
                match serde_json::from_value::<WorkflowDef>(request["params"]["def"].clone()) {
                    Ok(def) => {
                        let graph = DiGraph::from_def(&def);
                        match WorkflowStore::new(WorkflowStore::default_dir()) {
                            Ok(store) => match store.save(name, &graph) {
                                Ok(()) => json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": { "ok": true, "name": name }
                                }),
                                Err(e) => json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "error": { "code": -32040, "message": format!("Save error: {e}") }
                                }),
                            },
                            Err(e) => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "error": { "code": -32040, "message": format!("Store init error: {e}") }
                            }),
                        }
                    }
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32602, "message": format!("Invalid WorkflowDef: {e}") }
                    }),
                }
            }
            "workflow.load" => {
                let name = request["params"]["name"].as_str().unwrap_or("");
                if name.is_empty() {
                    return json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32602, "message": "Missing 'name' parameter" }
                    });
                }
                match WorkflowStore::new(WorkflowStore::default_dir()) {
                    Ok(store) => match store.load(name) {
                        Ok(graph) => {
                            let def = graph.to_def();
                            match serde_json::to_value(&def) {
                                Ok(v) => json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": { "def": v, "name": name }
                                }),
                                Err(e) => json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "error": { "code": -32040, "message": format!("Serialize error: {e}") }
                                }),
                            }
                        }
                        Err(e) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32041, "message": format!("Load error: {e}") }
                        }),
                    },
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32040, "message": format!("Store init error: {e}") }
                    }),
                }
            }
            "workflow.list" => {
                match WorkflowStore::new(WorkflowStore::default_dir()) {
                    Ok(store) => match store.list() {
                        Ok(names) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "names": names }
                        }),
                        Err(e) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32040, "message": format!("List error: {e}") }
                        }),
                    },
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32040, "message": format!("Store init error: {e}") }
                    }),
                }
            }
            "workflow.delete" => {
                let name = request["params"]["name"].as_str().unwrap_or("");
                if name.is_empty() {
                    return json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32602, "message": "Missing 'name' parameter" }
                    });
                }
                match WorkflowStore::new(WorkflowStore::default_dir()) {
                    Ok(store) => match store.delete(name) {
                        Ok(()) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "ok": true, "name": name }
                        }),
                        Err(e) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32040, "message": format!("Delete error: {e}") }
                        }),
                    },
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32040, "message": format!("Store init error: {e}") }
                    }),
                }
            }
            "workflow.run" => {
                let name = request["params"]["name"].as_str().unwrap_or("");
                if name.is_empty() {
                    return json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32602, "message": "Missing 'name' parameter" }
                    });
                }
                match WorkflowStore::new(WorkflowStore::default_dir()) {
                    Ok(store) => {
                        let registry = &*self.agent_registry;
                        match store.run(name, registry, GraphState::new()).await {
                            Ok(state) => match serde_json::to_value(&state) {
                                Ok(v) => json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": { "state": v, "name": name }
                                }),
                                Err(e) => json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "error": { "code": -32040, "message": format!("Serialize error: {e}") }
                                }),
                            },
                            Err(e) => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "error": { "code": -32042, "message": format!("Run error: {e}") }
                            }),
                        }
                    }
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32040, "message": format!("Store init error: {e}") }
                    }),
                }
            }
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Unknown method: {}", method) }
            }),
        }
    }
}

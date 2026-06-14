use std::path::PathBuf;
use std::sync::Arc;

use crate::core::orchestrator::AletheonRuntime;
use crate::core::config::RuntimeConfig;
use crate::r#impl::orchestration::registry::AgentRegistry;
use crate::r#impl::session::journal::EventJournal;
use crate::CoreMemory;
use crate::RecallMemory;
use crate::memory_tools::{CoreMemoryAppendTool, CoreMemoryReplaceTool, MemorySearchTool};
use crate::r#impl::orchestration::builtin::{FsAgent, NetAgent, CodeAgent};
use crate::ProviderRegistry;
use crate::session::store::SessionStore;
use aletheon_self::r#impl::perception::bridge::PerceptionInjection;
use aletheon_self::{SelfField, SelfFieldConfig};
use aletheon_meta::r#impl::meta_runtime::self_reader::SelfReader;
use aletheon_body::r#impl::sandbox::executor::{SandboxExecutor, SandboxPreference};
use aletheon_body::r#impl::security::audit::AuditLogger;
use aletheon_body::r#impl::security::runner::ToolRunnerWithGuard;
use aletheon_body::r#impl::tools::Tool;
use aletheon_body::r#impl::tools::ToolRegistry;
use aletheon_brain::r#impl::llm::LlmProvider;
use aletheon_brain::core::reflector::Reflector;
use aletheon_brain::core::ExperienceSummarizer;
use aletheon_abi::{Message, Role, ContentBlock, ReflectionTrigger, Subsystem, SubsystemContext,
    SelfFieldOps, Intent, IntentSource, Verdict, Context as AbiContext};
use aletheon_memory::episodic::EpisodicMemory;
use serde_json::json;
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

use super::DaemonConfig;

/// Session state wrapping the new AletheonRuntime.
///
/// NOTE: The old Engine god-object has been replaced by AletheonRuntime.
/// Methods like `run_turn`, `messages`, `set_perception_rx` etc. no longer
/// exist on the runtime.  This handler exposes a thin shim that delegates
/// to `AletheonRuntime::process()`.  A full migration of the daemon to the
/// new intent/plan/execute pipeline is tracked separately.
struct SessionState {
    runtime: AletheonRuntime,
    /// Pending input waiting to be processed via the cognitive pipeline.
    pending_input: Option<String>,
    /// Number of chat turns in this session.
    turn_count: usize,
}

#[derive(Clone)]
pub struct RequestHandler {
    state: Arc<Mutex<SessionState>>,
    llm: Arc<dyn LlmProvider>,
    /// Retained for future use; currently unused after Engine removal.
    #[allow(dead_code)]
    agent_registry: Arc<AgentRegistry>,
    reflector: Reflector,
    episodic_memory: Arc<Mutex<EpisodicMemory>>,
    /// SelfField — the policy engine that provides identity, cares, and boundary data.
    self_field: Arc<Mutex<SelfField>>,
}

impl RequestHandler {
    pub async fn new(config: &DaemonConfig, registry: &ProviderRegistry, _perception_rx: mpsc::Receiver<PerceptionInjection>) -> anyhow::Result<Self> {
        let llm: Arc<dyn LlmProvider> = Arc::from(registry.resolve_and_create("")?);
        info!(provider = llm.name(), "LLM provider initialized");

        // Create session and journal
        let session_id = uuid::Uuid::new_v4().to_string();
        let data_dir = PathBuf::from(&config.data_dir);
        let _journal = EventJournal::create(&session_id, &data_dir).await?;
        let session_store = SessionStore::new(&data_dir)?;
        session_store.create_session(&session_id)?;

        info!(session_id = %session_id, "Created new session with journal");

        // Create memory instances
        let core_memory = Arc::new(Mutex::new(CoreMemory::with_defaults()));
        let recall_db_path = data_dir.join("recall_memory.db");
        let recall_memory = Arc::new(Mutex::new(RecallMemory::new(&recall_db_path)?));

        // Register tools including memory tools
        let mut tools = ToolRegistry::default();
        tools.register(Arc::new(CoreMemoryAppendTool { memory: core_memory.clone() }));
        tools.register(Arc::new(CoreMemoryReplaceTool { memory: core_memory.clone() }));
        tools.register(Arc::new(MemorySearchTool { recall: recall_memory.clone() }));

        // Create security components
        let sandbox_pref = SandboxPreference::from_str(&config.sandbox_preference);
        let sandbox = SandboxExecutor::new(sandbox_pref);
        let audit_path = data_dir.join("audit.jsonl");
        let audit_logger = AuditLogger::new(audit_path)?;
        let _tool_runner = ToolRunnerWithGuard::new(sandbox, audit_logger);

        let runtime_config = RuntimeConfig {
            session_id: session_id.clone(),
            ..Default::default()
        };

        // Create agent registry: try config-based loading first, fall back to builtins
        let agents_dir = PathBuf::from("agents");

        // Collect default tools for config-based agent loading
        let default_tools: Vec<Box<dyn Tool>> = vec![
            Box::new(aletheon_body::r#impl::tools::file_read::FileReadTool),
            Box::new(aletheon_body::r#impl::tools::file_write::FileWriteTool),
            Box::new(aletheon_body::r#impl::tools::bash_exec::BashExecTool),
            Box::new(aletheon_body::r#impl::tools::system_status::SystemStatusTool),
            Box::new(aletheon_body::r#impl::tools::process_list::ProcessListTool),
        ];

        // Each config-loaded agent needs its own LLM instance; factory creates fresh ones
        let llm_factory = || registry.resolve_and_create("");
        let agent_registry = Arc::new(
            AgentRegistry::load_from_config(&agents_dir, &default_tools, &llm_factory).await
        );

        // Register built-in agents as fallbacks if no config agents were loaded
        if agent_registry.count().await == 0 {
            info!("No config agents found, registering built-in agents");
            agent_registry.register(Arc::new(tokio::sync::RwLock::new(
                FsAgent::new(registry.resolve_and_create("")?),
            ))).await;
            agent_registry.register(Arc::new(tokio::sync::RwLock::new(
                NetAgent::new(registry.resolve_and_create("")?),
            ))).await;
            agent_registry.register(Arc::new(tokio::sync::RwLock::new(
                CodeAgent::new(registry.resolve_and_create("")?),
            ))).await;
        }

        let runtime = AletheonRuntime::new(runtime_config);

        // Create reflector and episodic memory for post-chat reflection
        let reflector = Reflector::new();
        let episodic_db_path = data_dir.join("episodic.db");
        let mut episodic_memory = EpisodicMemory::new(episodic_db_path);
        let ctx = SubsystemContext {
            name: "episodic_memory".into(),
            working_dir: data_dir.clone(),
            config: serde_json::Value::Null,
        };
        episodic_memory.init(&ctx).await?;
        let episodic_memory = Arc::new(Mutex::new(episodic_memory));

        // Create SelfField for genome reads and policy engine
        let self_field_config = SelfFieldConfig {
            db_path: Some(data_dir.join("self_field.db")),
            ..Default::default()
        };
        let self_field = Arc::new(Mutex::new(SelfField::new(self_field_config)));

        Ok(Self {
            state: Arc::new(Mutex::new(SessionState {
                runtime,
                pending_input: None,
                turn_count: 0,
            })),
            llm,
            agent_registry,
            reflector,
            episodic_memory,
            self_field,
        })
    }

    pub async fn handle(&self, request: serde_json::Value) -> serde_json::Value {
        let method = request["method"].as_str().unwrap_or("");
        let id = request.get("id").cloned().unwrap_or(serde_json::Value::Null);

        match method {
            "chat" => {
                let message = request["params"]["message"].as_str().unwrap_or("");
                info!(message = %message, "Chat request received");

                // --- SelfField review: gate the user message before LLM ---
                let intent = Intent {
                    action: "chat".to_string(),
                    parameters: serde_json::json!({ "message": message }),
                    source: IntentSource::User,
                    description: format!("User chat message: {}", &message[..message.len().min(80)]),
                };
                let sf_ctx = AbiContext::new(
                    &self.state.lock().await.runtime.config().session_id,
                    std::env::current_dir().unwrap_or_default(),
                );

                let verdict = {
                    let sf = self.self_field.lock().await;
                    sf.review(&intent, &sf_ctx).await
                };

                let mut system_prompt = "You are Aletheon, an AI agent running on the user's machine. Be helpful, concise, and friendly.".to_string();

                match verdict {
                    Ok(Verdict::Deny { ref reason }) => {
                        warn!(reason = %reason, "SelfField denied chat intent");
                        let sf = self.self_field.lock().await;
                        let _ = sf.narrate("chat_denied", reason).await;
                        return json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32010, "message": format!("Intent denied by SelfField: {}", reason) }
                        });
                    }
                    Ok(Verdict::SandboxFirst { ref reason }) => {
                        info!(reason = %reason, "SelfField flagged chat for sandbox");
                        system_prompt.push_str(&format!(
                            "\n\n[SelfField note: This interaction has been flagged for sandbox review. Reason: {}]",
                            reason
                        ));
                    }
                    Ok(ref v) => {
                        info!(verdict = ?v, "SelfField approved chat intent");
                    }
                    Err(ref e) => {
                        warn!(error = %e, "SelfField review error, proceeding with caution");
                    }
                }

                let messages = vec![
                    Message {
                        role: Role::System,
                        content: vec![ContentBlock::Text {
                            text: system_prompt,
                        }],
                    },
                    Message::user(message),
                ];

                match self.llm.complete(&messages, &[]).await {
                    Ok(response) => {
                        let text = response.content.iter()
                            .filter_map(|block| match block {
                                ContentBlock::Text { text } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("");
                        info!(tokens = response.usage.output_tokens, "Chat response generated");

                        // Narrate the completed interaction in the SelfField narrative layer
                        {
                            let sf = self.self_field.lock().await;
                            let _ = sf.narrate(
                                "chat_completed",
                                &format!("User asked: '{}...' | Response: {} chars, {} tokens",
                                    &message[..message.len().min(60)],
                                    text.len(),
                                    response.usage.output_tokens,
                                ),
                            ).await;
                        }

                        // Increment turn count
                        let turn = {
                            let mut state = self.state.lock().await;
                            state.turn_count += 1;
                            state.turn_count
                        };

                        // Enhanced reflection: analyze question and response quality
                        let task_summary = if message.len() > 100 {
                            format!("{}...", &message[..100])
                        } else {
                            message.to_string()
                        };

                        let mut what_worked = Vec::new();
                        let mut what_failed = Vec::new();
                        let mut learned = Vec::new();

                        // Response length as a quality indicator
                        let resp_len = text.len();
                        if resp_len > 500 {
                            what_worked.push(format!("Detailed response ({} chars)", resp_len));
                        } else if resp_len > 100 {
                            what_worked.push(format!("Concise response ({} chars)", resp_len));
                        } else {
                            what_worked.push(format!("Brief response ({} chars)", resp_len));
                        }

                        // Token efficiency
                        if response.usage.output_tokens > 0 {
                            let chars_per_token = resp_len as f64 / response.usage.output_tokens as f64;
                            what_worked.push(format!(
                                "Token efficiency: {:.1} chars/token ({} output tokens)",
                                chars_per_token, response.usage.output_tokens
                            ));
                        }

                        // Detect error indicators in response
                        let text_lower = text.to_lowercase();
                        let error_indicators = ["error", "failed", "unable", "cannot", "couldn't", "sorry, i", "i don't know"];
                        for indicator in &error_indicators {
                            if text_lower.contains(indicator) {
                                what_failed.push(format!("Response contains '{}'", indicator));
                            }
                        }

                        // Detect learning/self-correction indicators
                        let learning_indicators = ["i learned", "i now understand", "i realize", "correction:", "actually,"];
                        for indicator in &learning_indicators {
                            if text_lower.contains(indicator) {
                                learned.push(format!("Self-correction detected: '{}'", indicator));
                            }
                        }

                        // Track turn context
                        what_worked.push(format!("Conversation turn #{}", turn));

                        let has_failures = !what_failed.is_empty();
                        let entry = self.reflector.reflect_conversation(
                            &task_summary,
                            ReflectionTrigger::TaskComplete,
                            !has_failures,
                            what_worked,
                            what_failed,
                            learned,
                        );
                        if let Err(e) = self.episodic_memory.lock().await.store_reflection(&entry) {
                            warn!(error = %e, "Failed to store chat reflection");
                        } else {
                            info!(id = %entry.id, task = %task_summary, "Chat reflection stored");

                            // Periodic evolution trigger: every 10 reflections, run ExperienceSummarizer
                            let mem = self.episodic_memory.lock().await;
                            if let Ok(count) = mem.reflection_count() {
                                if count > 0 && count % 10 == 0 {
                                    info!(count = count, "Running ExperienceSummarizer (periodic trigger)");
                                    if let Ok(recent) = mem.recall_reflections(20) {
                                        if let Some(evo_entry) = ExperienceSummarizer::summarize(&recent) {
                                            if let Err(e) = mem.store_evolution_log(&evo_entry) {
                                                warn!(error = %e, "Failed to store evolution log");
                                            } else {
                                                info!(id = %evo_entry.id, patterns = evo_entry.patterns_detected.len(), "Evolution log stored");
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "response": text, "turn": turn }
                        })
                    }
                    Err(e) => {
                        warn!(error = %e, "LLM call failed");
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32000, "message": format!("LLM error: {}", e) }
                        })
                    }
                }
            }
            "clear" => {
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
                let turn_count = state.turn_count;
                let iteration = state.runtime.iteration();
                drop(state);

                // Reflection and evolution counts from episodic memory
                let reflection_count = self.episodic_memory.lock().await
                    .reflection_count()
                    .unwrap_or(0);
                let evolution_count = self.episodic_memory.lock().await
                    .evolution_log_count()
                    .unwrap_or(0);

                // Care weights, boundary rules, and attention from SelfField
                let sf = self.self_field.lock().await;
                let care_weights: Vec<serde_json::Value> = sf.care().all_cares().into_iter().map(|c| {
                    json!({ "topic": c.topic, "weight": c.weight })
                }).collect();
                let boundary_total = sf.boundary().rule_count();
                let boundary_immutable = sf.boundary().immutable_rule_count();
                let attention_focus = sf.attention().current_focus()
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
                let reader = SelfReader::new();
                match reader.read_genome(&*self_field).await {
                    Ok(genome) => {
                        match serde_yaml::to_string(&genome) {
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
                        }
                    }
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
                    (state.turn_count, state.runtime.config().session_id.clone(), state.runtime.iteration())
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
                        learned.push(format!(
                            "Reviewed {} recent reflections",
                            recent.len()
                        ));
                        // Aggregate failure patterns
                        let failure_count: usize = recent.iter()
                            .map(|r| r.what_failed.len())
                            .sum();
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
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Unknown method: {}", method) }
            }),
        }
    }
}

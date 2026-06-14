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
use aletheon_body::r#impl::sandbox::executor::{SandboxExecutor, SandboxPreference};
use aletheon_body::r#impl::security::audit::AuditLogger;
use aletheon_body::r#impl::security::runner::ToolRunnerWithGuard;
use aletheon_body::r#impl::tools::Tool;
use aletheon_body::r#impl::tools::ToolRegistry;
use aletheon_brain::r#impl::llm::LlmProvider;
use aletheon_brain::core::reflector::Reflector;
use aletheon_brain::core::ExperienceSummarizer;
use aletheon_abi::{Message, Role, ContentBlock, ReflectionTrigger, Subsystem, SubsystemContext};
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

        Ok(Self {
            state: Arc::new(Mutex::new(SessionState {
                runtime,
                pending_input: None,
            })),
            llm,
            agent_registry,
            reflector,
            episodic_memory,
        })
    }

    pub async fn handle(&self, request: serde_json::Value) -> serde_json::Value {
        let method = request["method"].as_str().unwrap_or("");
        let id = request.get("id").cloned().unwrap_or(serde_json::Value::Null);

        match method {
            "chat" => {
                let message = request["params"]["message"].as_str().unwrap_or("");
                info!(message = %message, "Chat request received");

                let messages = vec![
                    Message {
                        role: Role::System,
                        content: vec![ContentBlock::Text {
                            text: "You are Aletheon, an AI agent running on the user's machine. Be helpful, concise, and friendly.".to_string(),
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

                        // Trigger post-chat reflection
                        let task_summary = &message[..message.len().min(100)];
                        let entry = self.reflector.reflect_conversation(
                            task_summary,
                            ReflectionTrigger::TaskComplete,
                            true,
                            vec!["LLM responded successfully".to_string()],
                            vec![],
                            vec![],
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
                            "result": { "response": text }
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
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "iteration": state.runtime.iteration(),
                        "config": state.runtime.config().session_id,
                    }
                })
            }
            "genome" => {
                // Return the agent's genome (self-description).
                // Currently returns a static initial genome; MetaRuntime integration pending.
                let genome = json!({
                    "topology": {
                        "subsystems": [
                            {"name": "self_field", "subsystem_type": "Policy", "version": "0.1.0", "dependencies": [], "config": {}},
                            {"name": "brain_core", "subsystem_type": "Cognitive", "version": "0.1.0", "dependencies": ["self_field"], "config": {}},
                            {"name": "body_runtime", "subsystem_type": "Execution", "version": "0.1.0", "dependencies": ["brain_core"], "config": {}},
                            {"name": "memory", "subsystem_type": "Storage", "version": "0.1.0", "dependencies": [], "config": {}},
                            {"name": "meta_runtime", "subsystem_type": "Evolution", "version": "0.1.0", "dependencies": ["memory"], "config": {}}
                        ]
                    },
                    "identity": {
                        "name": "aletheon",
                        "description": "Autonomous cognitive agent",
                        "self_model": "4-layer architecture: self-field -> brain-core -> body-runtime -> memory"
                    },
                    "boundary": {"rules": []},
                    "care": {"priorities": [{"topic": "user_safety", "weight": 1.0}, {"topic": "self_coherence", "weight": 0.8}]},
                    "memory": {"backends": ["episodic", "core"], "compaction_strategy": "age_based"},
                    "mutation": {"allowed_targets": ["config", "prompts"], "require_sandbox": true, "require_self_field_approval": true},
                    "lifecycle": {"auto_compact": true, "health_check_interval_secs": 300, "max_idle_time_secs": 3600}
                });
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "genome": genome }
                })
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
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Unknown method: {}", method) }
            }),
        }
    }
}

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
use aletheon_self_field::r#impl::perception::bridge::PerceptionInjection;
use aletheon_body::r#impl::sandbox::executor::{SandboxExecutor, SandboxPreference};
use aletheon_body::r#impl::security::audit::AuditLogger;
use aletheon_body::r#impl::security::runner::ToolRunnerWithGuard;
use aletheon_body::r#impl::tools::Tool;
use aletheon_body::r#impl::tools::ToolRegistry;
use serde_json::json;
use tokio::sync::{mpsc, Mutex};
use tracing::info;

use super::DaemonConfig;

/// Session state wrapping the new AletheonRuntime.
///
/// NOTE: The old Engine god-object has been replaced by AletheonRuntime.
/// Methods like `run_turn`, `messages`, `set_perception_rx` etc. no longer
/// exist on the runtime.  This handler exposes a thin shim that delegates
/// to `AletheonRuntime::process()`.  A full migration of argosd to the
/// new intent/plan/execute pipeline is tracked separately.
struct SessionState {
    runtime: AletheonRuntime,
    /// Pending input waiting to be processed via the cognitive pipeline.
    pending_input: Option<String>,
}

#[derive(Clone)]
pub struct RequestHandler {
    state: Arc<Mutex<SessionState>>,
    /// Retained for future use; currently unused after Engine removal.
    #[allow(dead_code)]
    agent_registry: Arc<AgentRegistry>,
}

impl RequestHandler {
    pub async fn new(config: &DaemonConfig, registry: &ProviderRegistry, _perception_rx: mpsc::Receiver<PerceptionInjection>) -> anyhow::Result<Self> {
        let _llm = registry.resolve_and_create("")?;

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

        Ok(Self {
            state: Arc::new(Mutex::new(SessionState {
                runtime,
                pending_input: None,
            })),
            agent_registry,
        })
    }

    pub async fn handle(&self, request: serde_json::Value) -> serde_json::Value {
        let method = request["method"].as_str().unwrap_or("");
        let id = request.get("id").cloned().unwrap_or(serde_json::Value::Null);

        match method {
            "chat" => {
                let message = request["params"]["message"].as_str().unwrap_or("");
                let mut state = self.state.lock().await;
                // Store input for processing; actual LLM invocation requires
                // injecting review/think/execute callbacks which will be wired
                // up in a follow-up migration.
                state.pending_input = Some(message.to_string());
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "response": format!("[pending] {}", message) }
                })
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
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Unknown method: {}", method) }
            }),
        }
    }
}

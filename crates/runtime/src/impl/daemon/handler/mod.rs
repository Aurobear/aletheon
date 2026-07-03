#![allow(deprecated)]
// TODO(P1-A): Migrate event_bus field from Arc<dyn EventBus> to Arc<CommunicationBus>.
// Blocked by DaseinEventBridge (crates/dasein/src/dasein/event_bridge.rs) which uses
// EventBus::subscribe callback pattern. The event_bus is passed through to
// wire_dasein_event_bridge() at boot time. Once DaseinEventBridge is migrated to
// channel-based CommunicationBus topic subscriptions, this field can be changed.
//
// Additionally, the file uses `use base::envelope::*;` which re-exports types
// that use deprecated Event/Payload. These need to be migrated separately.

mod chat;
mod connection;
mod format;
mod init;
mod rpc;
mod session_routing;
mod turn_handler;

use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use super::model_router::ModelRouter;
use super::session_manager::SessionManager;
use crate::core::orchestrator::AletheonRuntime;
use crate::r#impl::orchestration::registry::AgentRegistry;
use crate::CoreMemory;
use crate::RecallMemory;
use base::envelope::Payload;
use base::envelope::*;
use base::CommunicationBus;
use base::{Context as AbiContext, Intent, SelfFieldOps, Verdict};
use cognit::core::reflector::Reflector;
use cognit::r#impl::llm::LlmProvider;
use corpus::security::security::approval::ApprovalDecision;
use corpus::security::security::runner::ToolRunnerWithGuard;
use corpus::security::security::socket_approval::PendingApproval;
use corpus::tools::tools::ToolRegistry;
use dasein::SelfField;
use memory::episodic::EpisodicMemory;
use metacog::{DefaultMetaRuntime, MorphogenesisPipeline};
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::warn;

use crate::core::checkpoint::CheckpointStore;
use crate::core::config::HooksConfig;
use crate::core::storm_breaker::StormBreaker;
use crate::r#impl::agent_loader::AgentLoader;
use crate::r#impl::engine::modules::{SelfFieldRequest, SelfFieldResponse};
use crate::r#impl::goal::ObjectiveStore;
use crate::r#impl::hooks::registry::HookRegistry;
use crate::r#impl::memory::auto_memory::AutoMemory;
use crate::r#impl::memory::fact_store::FactStore;
use crate::r#impl::skill_router::SkillRouter;
use crate::r#impl::skills::loader::SkillLoader;

use super::debug_handler::DebugHandler;
use crate::core::session_gateway::SessionGateway;
use base::kernel::debug_bus::PerfCounter;

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
    model_router: Arc<ModelRouter>,
    /// Multi-session registry: session_id -> SessionManager.
    sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
    /// Default session ID for requests that do not specify one.
    default_session_id: Arc<tokio::sync::Mutex<String>>,
    /// Per-session creation timestamps (keyed by session_id).
    session_created_at: Arc<Mutex<HashMap<String, Instant>>>,
    recall_memory: Arc<Mutex<RecallMemory>>,
    data_dir: PathBuf,
    /// The LLM's context window size, used for SessionManager creation.
    context_window: usize,
    /// Daemon start time for uptime calculation.
    started_at: Instant,
    /// Active connection count.
    active_connections: Arc<AtomicUsize>,
    /// Retained for future use; currently unused after Engine removal.
    #[allow(dead_code)]
    agent_registry: Arc<AgentRegistry>,
    reflector: Reflector,
    episodic_memory: Arc<Mutex<EpisodicMemory>>,
    /// SelfField — the policy engine that provides identity, cares, and boundary data.
    self_field: Arc<Mutex<SelfField>>,
    /// Loads skill markdown files from the skills directory and caches them.
    skill_loader: Arc<Mutex<SkillLoader>>,
    /// Cache-stable system prompt prefix, built once at boot.
    /// Same inputs = same bytes = cache hit on DeepSeek/Mimo.
    cached_prefix: Arc<Mutex<String>>,
    /// Queue for memory updates that arrive mid-session.
    /// Drained into user turns as `<memory-update>` XML blocks
    /// so the system prompt prefix stays byte-stable.
    memory_queue: Arc<Mutex<Vec<String>>>,
    /// The base system prompt from config, retained for prefix rebuilds.
    config_prompt: String,
    /// Core memory reference, retained for prefix rebuilds on skill reload.
    core_memory: Arc<Mutex<CoreMemory>>,
    /// Lifecycle hook registry.
    hook_registry: Arc<Mutex<HookRegistry>>,
    /// CommunicationBus for inter-module communication.
    /// When `Some`, SelfField review/narrate calls go through the bus.
    bus: Option<Arc<CommunicationBus>>,
    /// Guarded tool runner (policy -> approval -> loop detector -> sandbox -> audit).
    /// Wired to the SocketApprovalGate so L2+ requests are forwarded to the client.
    tool_runner: Arc<Mutex<ToolRunnerWithGuard>>,
    /// Tool registry shared with BodyModule; kept here for ReAct loop tool execution.
    tools: Arc<Mutex<ToolRegistry>>,
    /// Receiver for pending approval requests from the SocketApprovalGate.
    /// Drained during chat turns to relay approval requests to the client.
    approval_rx: Arc<Mutex<mpsc::Receiver<PendingApproval>>>,
    /// Map from approval_id to the oneshot sender that resolves the pending approval.
    pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    /// Channel to send out-of-band JSON-RPC notifications to the connected client.
    /// Used to push `approval_request` notifications during a chat turn.
    notify_tx: Option<mpsc::Sender<String>>,
    /// SQLite-backed fact store with trust scoring and FTS5 search.
    fact_store: Arc<Mutex<FactStore>>,
    /// SQLite-backed objective store for persistent goal tracking.
    objective_store: Arc<Mutex<ObjectiveStore>>,
    /// Cached active objective + sub-goals for resume-on-start.
    /// Applied once to GoalTracker before the first chat turn.
    resumed_objective: Option<(String, Vec<String>)>,
    /// Loop detector: tracks consecutive tool failures/successes.
    storm_breaker: Arc<Mutex<StormBreaker>>,
    /// Per-session checkpoint store for file-edit rewind.
    #[allow(dead_code)]
    checkpoint_store: Arc<Mutex<CheckpointStore>>,
    /// Keyword-based skill router for prompt-to-skill matching.
    skill_router: Arc<Mutex<SkillRouter>>,
    /// Agent role loader — loads agent markdown definitions from ~/.aletheon/agents/.
    #[allow(dead_code)]
    agent_loader: Arc<Mutex<AgentLoader>>,
    /// Configured hook scripts from the [hooks] config section.
    hooks_config: HooksConfig,
    /// Per-session "always approve" cache: tool_name -> approved.
    /// Populated when user responds with "always" to an approval request.
    session_approvals: Arc<Mutex<HashMap<String, bool>>>,
    /// Morphogenesis pipeline for post-turn evolution.
    pipeline: Arc<MorphogenesisPipeline<DefaultMetaRuntime>>,
    /// Automatic memory extraction — uses a cheap LLM to extract and store
    /// important facts from each conversation turn.
    auto_memory: Arc<Mutex<AutoMemory>>,
    /// Debug handler — exposes debug.* JSON-RPC methods for tracing, perf, and bag recording.
    debug_handler: Arc<DebugHandler>,
    /// Performance counter — shared with DebugHandler, also used by the ReAct loop.
    debug_perf: Arc<PerfCounter>,
    /// Cancellation token for the current chat turn.
    cancel_token: Arc<Mutex<Option<CancellationToken>>>,
    /// EventBus for cross-subsystem event routing (DaseinEventBridge, etc.).
    event_bus: Option<Arc<dyn base::EventBus>>,
    /// Daemon-level cancellation token for graceful shutdown via daemon.shutdown RPC.
    daemon_cancel_token: Option<CancellationToken>,
    /// Session Gateway — unified facade for external agent debug access.
    session_gateway: Arc<SessionGateway>,
}

impl RequestHandler {
    /// Review an intent through SelfField via CommunicationBus.
    /// Falls back to direct lock if bus is not configured.
    pub(crate) async fn sf_review(
        &self,
        intent: &Intent,
        ctx: &AbiContext,
    ) -> anyhow::Result<Verdict> {
        if let Some(ref bus) = self.bus {
            let req = SelfFieldRequest::Review {
                intent: intent.clone(),
                ctx: serde_json::to_value(ctx).unwrap_or_default(),
            };
            let envelope = Envelope::request(
                Endpoint::Module(ModuleId::Runtime),
                Target::Module(ModuleId::SelfField),
                Payload::Json(serde_json::to_value(&req).unwrap_or_default()),
                Duration::from_secs(5),
            );
            match bus.request(envelope).await {
                Ok(resp_envelope) => {
                    if let Payload::Json(val) = &resp_envelope.payload {
                        match serde_json::from_value::<SelfFieldResponse>(val.clone()) {
                            Ok(SelfFieldResponse::Verdict { verdict }) => return Ok(verdict),
                            Ok(SelfFieldResponse::Error { message }) => {
                                return Err(anyhow::anyhow!("SelfField review error: {}", message));
                            }
                            Ok(other) => {
                                return Err(anyhow::anyhow!(
                                    "Unexpected SelfField response: {:?}",
                                    other
                                ));
                            }
                            Err(e) => {
                                return Err(anyhow::anyhow!(
                                    "Failed to deserialize SelfFieldResponse: {}",
                                    e
                                ));
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Bus request for SelfField review failed, falling back to direct");
                }
            }
        }
        // Fallback: direct lock
        let sf = self.self_field.lock().await;
        sf.review(intent, ctx).await
    }

    /// Record a narrative entry in SelfField via CommunicationBus.
    /// Falls back to direct lock if bus is not configured.
    pub(crate) async fn sf_narrate(&self, event: &str, reason: &str) {
        if let Some(ref bus) = self.bus {
            let req = SelfFieldRequest::Narrate {
                event: event.to_string(),
                reason: reason.to_string(),
            };
            let envelope = Envelope::request(
                Endpoint::Module(ModuleId::Runtime),
                Target::Module(ModuleId::SelfField),
                Payload::Json(serde_json::to_value(&req).unwrap_or_default()),
                Duration::from_secs(5),
            );
            match bus.request(envelope).await {
                Ok(resp_envelope) => {
                    if let Payload::Json(val) = &resp_envelope.payload {
                        match serde_json::from_value::<SelfFieldResponse>(val.clone()) {
                            Ok(SelfFieldResponse::Narrated) => return,
                            Ok(SelfFieldResponse::Error { message }) => {
                                warn!(error = %message, "SelfField narrate error via bus");
                                return;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Bus request for SelfField narrate failed, falling back to direct");
                }
            }
        }
        // Fallback: direct lock
        let sf = self.self_field.lock().await;
        let _ = sf.narrate(event, reason).await;
    }

    /// Post-turn coordination: update Dasein mood from turn output.
    pub(crate) async fn coordinate(&self, turn: &usize, turn_text: &str) {
        let sf = self.self_field.lock().await;
        if let Some(dasein) = sf.dasein() {
            let _mood = dasein.quick_mood_update(turn_text);
            tracing::info!(turn = turn, "Dasein mood updated via coordinator");
        }
    }

    /// Compose the user message with mid-session injections from the memory queue.
    ///
    /// Drains all pending memory updates and prepends them as a `<memory-update>`
    /// XML block before the raw user input.  This is the same pattern as
    /// `ReActLoop::compose_user_message()` and `Controller::compose_user_message()`
    /// — changes ride the user message tail so the system prompt prefix stays
    /// byte-stable for provider cache hits.
    ///
    /// Returns empty string if the queue is empty (no injections needed).
    pub(crate) async fn compose_memory_block(&self) -> String {
        let mut queue = self.memory_queue.lock().await;
        if queue.is_empty() {
            return String::new();
        }
        let updates: Vec<String> = queue.drain(..).collect();
        drop(queue);

        let items: Vec<String> = updates.iter().map(|m| format!("- {}", m)).collect();
        format!("<memory-update>\n{}\n</memory-update>", items.join("\n"))
    }

    /// Execute configured hook scripts at a lifecycle point.
    ///
    /// Each script is spawned as a subprocess with `input_json` piped to stdin.
    /// Stdout from successful scripts is collected and returned.
    /// Each script has a 30-second timeout.
    pub(crate) async fn run_hook_scripts(
        &self,
        scripts: &[String],
        input_json: &str,
    ) -> Vec<String> {
        let mut outputs = Vec::new();
        for script_path in scripts {
            let path = format::expand_tilde(script_path);
            if !std::path::Path::new(&path).exists() {
                tracing::warn!(path = %path, "Hook script not found, skipping");
                continue;
            }
            let spawn_result = tokio::process::Command::new(&path)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn();

            match spawn_result {
                Ok(mut child) => {
                    // Write input to stdin
                    if let Some(stdin) = child.stdin.take() {
                        let input = input_json.to_string();
                        tokio::spawn(async move {
                            use tokio::io::AsyncWriteExt;
                            let mut stdin = stdin;
                            let _ = stdin.write_all(input.as_bytes()).await;
                        });
                    }
                    // Capture stdout before waiting
                    let mut stdout_pipe = child.stdout.take();
                    // Wait with 30-second timeout
                    match tokio::time::timeout(Duration::from_secs(30), child.wait()).await {
                        Ok(Ok(status)) if status.success() => {
                            // Read captured stdout
                            if let Some(ref mut stdout) = stdout_pipe {
                                use tokio::io::AsyncReadExt;
                                let mut buf = String::new();
                                if stdout.read_to_string(&mut buf).await.is_ok() && !buf.is_empty()
                                {
                                    outputs.push(buf);
                                }
                            }
                        }
                        Ok(Ok(status)) => {
                            tracing::warn!(
                                path = %path,
                                code = status.code(),
                                "Hook script exited with non-zero status"
                            );
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(path = %path, error = %e, "Hook script I/O error");
                        }
                        Err(_) => {
                            tracing::warn!(path = %path, "Hook script timed out (30s)");
                            child.kill().await.ok();
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(path = %path, error = %e, "Failed to spawn hook script");
                }
            }
        }
        outputs
    }

    pub async fn handle(&self, request: serde_json::Value) -> serde_json::Value {
        let method = request["method"].as_str().unwrap_or("").to_string();
        let id = request
            .get("id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let params = request
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        // Route session.* methods to the Session Gateway (new unified facade).
        if method.starts_with("session.") {
            if let Some(response) = self
                .session_gateway
                .handle_method(&method, &id, &params)
                .await
            {
                return response;
            }
        }

        // Route debug.* methods to the debug handler (backward compat).
        if method.starts_with("debug.") {
            if let Some(response) = self
                .debug_handler
                .handle_method(&method, &id, &params)
                .await
            {
                return response;
            }
        }

        match method.as_str() {
            "chat" => self.handle_chat(id, request).await,
            _ => self.handle_rpc(&method, id, request).await,
        }
    }
}

//! Daemon request handler — JSON-RPC dispatcher for the Unix socket server.
//! Handles chat, RPC, session management, and lifecycle events.
//!
// TODO(P1-A): Migrate event_bus field from Arc<dyn EventBus> to Arc<CommunicationBus>.
// DONE: event_bus is now Arc<CommunicationBus>. DaseinEventBridge has been updated
// to accept CommunicationBus. The underlying EventBus::subscribe calls remain
// (accessible via communication_bus.event_bus()) until a true topic-based migration.
//
// Additionally, the file uses `use fabric::envelope::*;` which re-exports types
// that use deprecated Event/Payload. These need to be migrated separately.

mod chat;
mod connection;
mod format;
mod init;
mod rpc;
mod session_routing;
mod tool_executor;
mod turn_handler;

use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use super::model_router::ModelRouter;
use super::session_manager::SessionManager;
use crate::core::core_systems::CoreSystems;
use fabric::envelope::Payload;
use fabric::envelope::*;
use fabric::CommunicationBus;
use fabric::LlmProvider;
use fabric::{Context as AbiContext, Intent, SelfFieldOps, Verdict};

use std::collections::HashMap;
use tokio::sync::{mpsc, Mutex};
use tracing::warn;

use crate::r#impl::engine::modules::{SelfFieldRequest, SelfFieldResponse};

use crate::core::session_gateway::SessionGateway;

#[derive(Clone)]
pub struct RequestHandler {
    /// All subsystem types — becomes `Arc<dyn TraitOps>` in Group B.
    pub(crate) subsystems: Arc<CoreSystems>,
    /// Multi-session registry.
    pub(crate) sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
    /// Session gateway for external agent debug access.
    pub(crate) session_gateway: Arc<SessionGateway>,
    /// Communication bus (always available after init).
    pub(crate) bus: Arc<CommunicationBus>,
    /// Default LLM provider.
    pub(crate) llm: Arc<dyn LlmProvider>,
    /// Model router for per-task-type model selection.
    pub(crate) model_router: Arc<ModelRouter>,
    /// Per-connection notification channel for JSON-RPC push.
    pub(crate) notify_tx: Option<mpsc::Sender<String>>,
    /// Active connection count.
    pub(crate) active_connections: Arc<AtomicUsize>,
    /// Daemon start time.
    pub(crate) started_at: Instant,
    /// Daemon-level cancellation token for graceful shutdown.
    pub(crate) daemon_cancel_token: Option<CancellationToken>,
}

impl RequestHandler {
    /// Review an intent through SelfField via CommunicationBus.
    /// Falls back to direct lock if bus is not configured.
    pub(crate) async fn sf_review(
        &self,
        intent: &Intent,
        ctx: &AbiContext,
    ) -> anyhow::Result<Verdict> {
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
        match self.bus.request(envelope).await {
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
        // Fallback: direct lock
        let sf = self.subsystems.self_field.lock().await;
        sf.review(intent, ctx).await
    }

    /// Record a narrative entry in SelfField via CommunicationBus.
    /// Falls back to direct lock if bus is not configured.
    pub(crate) async fn sf_narrate(&self, event: &str, reason: &str) {
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
        match self.bus.request(envelope).await {
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
        // Fallback: direct lock
        let sf = self.subsystems.self_field.lock().await;
        let _ = sf.narrate(event, reason).await;
    }

    /// Post-turn coordination: update Dasein mood from turn output.
    pub(crate) async fn coordinate(&self, turn: &usize, turn_text: &str) {
        let sf = self.subsystems.self_field.lock().await;
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
        let mut queue = self.subsystems.memory_queue.lock().await;
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
                .subsystems
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

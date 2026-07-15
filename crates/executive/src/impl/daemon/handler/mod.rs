//! Daemon request handler — JSON-RPC dispatcher for the Unix socket server.
//! Handles chat, RPC, session management, and lifecycle events.

mod connection;
pub(crate) mod format;
mod init;
mod ports;
mod rpc;
pub(crate) mod tool_executor;
mod turn_handler;

use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Duration;

use aletheon_kernel::chronos::SystemTimer;
use fabric::Timer;
use tokio_util::sync::CancellationToken;

use super::model_router::ModelRouter;
use crate::core::core_systems::CoreSystems;
use crate::service::DaemonTurnOrchestrator;
use fabric::envelope::Payload;
use fabric::envelope::*;
use fabric::CommunicationBus;
use fabric::LlmProvider;
use fabric::{Context as AbiContext, Intent, SelfFieldOps, Verdict};

use std::path::Path;
use tokio::sync::{mpsc, Mutex};
use tracing::warn;

use crate::r#impl::engine::modules::{SelfFieldRequest, SelfFieldResponse};

use crate::core::session_gateway::SessionGateway;

#[derive(Clone)]
pub struct RequestHandler {
    /// Narrow application use cases available to protocol handlers.
    pub(crate) ports: Arc<ports::HandlerPorts>,
    /// All subsystem types — becomes `Arc<dyn TraitOps>` in Group B.
    pub(crate) subsystems: Arc<CoreSystems>,
    /// Session gateway for external agent debug access.
    pub(crate) session_gateway: Arc<SessionGateway>,
    /// Communication bus (always available after init).
    /// Only used by the currently-dead `sf_review`/`sf_narrate` bus fallback paths;
    /// will become live when those methods are re-plumbed.
    #[allow(dead_code)]
    pub(crate) bus: Arc<CommunicationBus>,
    /// Default LLM provider.
    pub(crate) llm: Arc<dyn LlmProvider>,
    /// Model router for per-task-type model selection.
    /// Stored for future handler-level routing; the live path passes a clone
    /// directly to the orchestrator at construction time (`init.rs`).
    #[allow(dead_code)]
    pub(crate) model_router: Arc<ModelRouter>,
    /// Per-connection notification channel for JSON-RPC push.
    pub(crate) notify_tx: Option<mpsc::Sender<String>>,
    /// Active connection count.
    pub(crate) active_connections: Arc<AtomicUsize>,
    /// Daemon start time.
    pub(crate) started_at: fabric::MonoTime,
    /// Daemon-level cancellation token for graceful shutdown.
    pub(crate) daemon_cancel_token: Option<CancellationToken>,
    /// Macro-kernel turn orchestrator — handles the full pre/core/post turn pipeline.
    pub(crate) turn_orchestrator: Arc<DaemonTurnOrchestrator>,
    /// Telegram poll-loop task handle — stored so daemon shutdown can await
    /// graceful termination. `None` when Telegram is disabled or not yet started.
    pub(crate) telegram_task: Option<Arc<tokio::task::JoinHandle<()>>>,
    /// Durable Goal scheduler, awaited with a bound during shutdown.
    pub(crate) goal_worker_task: Option<Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>>,
    /// Google poll-loop supervisor handle, retained for graceful shutdown.
    pub(crate) google_sync: Option<Arc<Mutex<Option<crate::r#impl::google::GoogleSyncHandle>>>>,
    /// Configured Google control plane. Credentials remain behind its vault-owning boundary.
    pub(crate) google: Option<Arc<crate::r#impl::external::GoogleIntegration>>,
    /// Sanitized readiness state; payload data and error bodies never enter it.
    pub(crate) health: Arc<crate::r#impl::health::HealthRegistry>,
}

impl RequestHandler {
    /// Review an intent through SelfField via CommunicationBus.
    /// Falls back to direct lock if bus is not configured.
    /// Reserved for future bus-based SelfField integration; not yet called.
    #[allow(dead_code)]
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
            Endpoint::Module(ModuleId::Executive),
            Target::Module(ModuleId::Dasein),
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
    /// Reserved for future bus-based SelfField integration; not yet called.
    #[allow(dead_code)]
    pub(crate) async fn sf_narrate(&self, event: &str, reason: &str) {
        let req = SelfFieldRequest::Narrate {
            event: event.to_string(),
            reason: reason.to_string(),
        };
        let envelope = Envelope::request(
            Endpoint::Module(ModuleId::Executive),
            Target::Module(ModuleId::Dasein),
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
    /// Reserved for future Dasein mood-feedback integration; not yet called.
    #[allow(dead_code)]
    pub(crate) async fn coordinate(
        &self,
        turn: &usize,
        turn_text: &str,
        status: fabric::dasein::OutcomeStatus,
    ) {
        let sf = self.subsystems.self_field.lock().await;
        if let Some(dasein) = sf.dasein() {
            match dasein
                .record_outcome(turn_text, status, "legacy-daemon-handler")
                .await
            {
                Ok(receipt) => tracing::info!(
                    turn,
                    version = receipt.current_version.0,
                    "Dasein outcome transition accepted"
                ),
                Err(error) => tracing::warn!(turn, %error, "Dasein outcome transition rejected"),
            }
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
    /// Reserved for future handler-based memory injection; the live path uses
    /// `TurnPipeline::compose_memory_block` instead.
    #[allow(dead_code)]
    pub(crate) async fn compose_memory_block(&self) -> String {
        let mut queue = self.subsystems.session.memory_queue.lock().await;
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
                    match SystemTimer
                        .timeout(Duration::from_secs(30), child.wait())
                        .await
                    {
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

        if matches!(
            method.as_str(),
            "session.resume" | "session.fork" | "session.interrupt" | "session.replay"
        ) {
            let service = &self.turn_orchestrator.session_service;
            let session_id = fabric::SessionId(
                params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            );
            let result: anyhow::Result<serde_json::Value> = match method.as_str() {
                "session.resume" => service.resume(&session_id).await.map(|resume| serde_json::json!({
                    "session": resume.session, "next_sequence": resume.next_sequence, "messages": resume.messages,
                })),
                "session.fork" => service.fork(
                    &session_id,
                    params.get("through_sequence").and_then(|v| v.as_u64()).unwrap_or(0),
                ).await.and_then(|record| serde_json::to_value(record).map_err(Into::into)),
                "session.interrupt" => service.interrupt(&session_id).await.map(|outcome| serde_json::json!({
                    "outcome": format!("{outcome:?}").to_lowercase(),
                })),
                "session.replay" => service.replay(
                    &session_id,
                    params.get("after_sequence").and_then(|v| v.as_u64()),
                ).await.map(|messages| serde_json::json!({"messages": messages})),
                _ => unreachable!(),
            };
            return match result {
                Ok(result) => serde_json::json!({"jsonrpc":"2.0","id":id,"result":result}),
                Err(error) => {
                    serde_json::json!({"jsonrpc":"2.0","id":id,"error":{"code":-32020,"message":error.to_string()}})
                }
            };
        }

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

    /// Thin delegation to the macro-kernel turn orchestrator.
    pub(super) async fn handle_chat(
        &self,
        id: serde_json::Value,
        request: serde_json::Value,
    ) -> serde_json::Value {
        let message = request["params"]["message"].as_str().unwrap_or("");
        let working_dir =
            match validate_local_working_dir(request["params"]["working_dir"].as_str()) {
                Ok(path) => path,
                Err(error) => {
                    return serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32602, "message": error }
                    });
                }
            };
        if let Err(error) = self.select_workspace_session(&working_dir).await {
            return serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32603, "message": error.to_string() }
            });
        }
        tracing::info!(message = %message, "Chat request received");
        self.turn_orchestrator
            .execute_turn(id, message, working_dir)
            .await
    }

    /// Keep local conversation history scoped to its canonical workspace.
    /// Without this, a TUI launched in one checkout inherits tool paths from
    /// the last TUI that happened to use the daemon's global default session.
    async fn select_workspace_session(&self, working_dir: &Path) -> anyhow::Result<()> {
        let session_id = self
            .ports
            .sessions
            .route_workspace(working_dir.to_path_buf())
            .await?;
        tracing::info!(%session_id, cwd = %working_dir.display(), "Selected new workspace session");
        Ok(())
    }
}

const LOCAL_WORKSPACE_ROOT: &str = "/home/aurobear/Bear-ws";
const LEGACY_WORKING_DIR: &str = "/var/lib/aletheon";

fn validate_local_working_dir(value: Option<&str>) -> Result<std::path::PathBuf, String> {
    validate_working_dir_against_roots(
        value.unwrap_or(LEGACY_WORKING_DIR),
        std::path::Path::new(LOCAL_WORKSPACE_ROOT),
        std::path::Path::new(LEGACY_WORKING_DIR),
    )
}

fn validate_working_dir_against_roots(
    requested: &str,
    workspace_root: &std::path::Path,
    legacy_root: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let canonical = std::fs::canonicalize(requested)
        .map_err(|error| format!("invalid working_dir '{requested}': {error}"))?;
    let workspace_root =
        std::fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    if canonical.starts_with(&workspace_root) || canonical.starts_with(legacy_root) {
        Ok(canonical)
    } else {
        Err(format!(
            "working_dir '{}' is outside allowed roots '{}' and '{}'",
            canonical.display(),
            workspace_root.display(),
            legacy_root.display()
        ))
    }
}

#[cfg(test)]
mod working_dir_tests {
    #[test]
    fn rejects_root_as_local_working_directory() {
        assert!(super::validate_local_working_dir(Some("/")).is_err());
    }

    #[test]
    fn rejects_missing_local_working_directory() {
        assert!(
            super::validate_local_working_dir(Some("/home/aurobear/Bear-ws/does-not-exist"))
                .is_err()
        );
    }

    #[test]
    fn accepts_a_canonical_bear_workspace_directory() {
        let root = std::env::temp_dir().join(format!("aletheon-cwd-test-{}", std::process::id()));
        let project = root.join("aletheon");
        std::fs::create_dir_all(&project).unwrap();
        let path = super::validate_working_dir_against_roots(
            project.to_str().unwrap(),
            &root,
            std::path::Path::new("/var/lib/aletheon"),
        )
        .unwrap();
        assert!(path.starts_with(std::fs::canonicalize(&root).unwrap()));
        std::fs::remove_dir_all(root).unwrap();
    }
}

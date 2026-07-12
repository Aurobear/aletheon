//! `TurnToolExecutor` — the per-tool execution pipeline for a chat turn.
//!
//! Extracted from the former inline `execute_tool` closure (previously in chat.rs, now deleted)
//! (RFC-018 D5 seam 3 / issue #4). Runs one tool through the full pipeline:
//! PreTool hook → OnMemoryRecall hook → session-approval check → SelfField
//! review → registry lookup → `ExecutionPermit`-guarded
//! `ToolRunnerWithGuard::run` → PerfCounter → StormBreaker → PostTool hook,
//! returning `(content, is_error)`.
//!
//! Behaviour is identical to the previous closure; this only gives the pipeline
//! a name and a home. It is adapted to the harness's
//! `Fn(&str, &str, &Value) -> Future<Output=(String, bool)>` executor parameter
//! by a thin `Arc<Self>` closure wrapper.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::info;

use corpus::security::runner::ToolRunnerWithGuard;
use corpus::security::storm_breaker::StormBreaker;
use corpus::tools::tools::ToolRegistry;
use corpus::HookRegistry;
use dasein::SelfField;
use fabric::hook::{HookContext, HookPoint, HookResult};
use fabric::kernel::debug_bus::PerfCounter;
use fabric::types::admission::ExecutionPermit;
use fabric::types::operation::{OperationId, ProcessId};
use fabric::{Context as AbiContext, Intent, IntentSource, SelfFieldOps, Verdict};

use crate::core::core_systems::CoreSystems;

/// Executes a single tool through the full guarded/hooked pipeline for one turn.
///
/// Holds the same subsystem handles the former `execute_tool` closure captured;
/// cheap to wrap in `Arc` and clone per tool call.
pub(crate) struct TurnToolExecutor {
    tool_runner: Arc<Mutex<ToolRunnerWithGuard>>,
    tools: Arc<Mutex<ToolRegistry>>,
    hook_registry: Arc<Mutex<HookRegistry>>,
    storm_breaker: Arc<Mutex<StormBreaker>>,
    memory_queue: Arc<Mutex<Vec<String>>>,
    session_approvals: Arc<Mutex<HashMap<String, bool>>>,
    debug_perf: Arc<PerfCounter>,
    self_field: Arc<Mutex<SelfField>>,
    working_dir: PathBuf,
    session_id: String,
    turn_count: usize,
    /// Kernel operation id for this turn (used by admission controller).
    operation_id: OperationId,
    /// Kernel process id for the main agent (used by admission controller).
    process_id: ProcessId,
}

impl TurnToolExecutor {
    /// Build an executor for one turn, cloning the needed subsystem handles.
    pub(crate) fn new(
        subsystems: &CoreSystems,
        session_id: String,
        turn_count: usize,
        working_dir: PathBuf,
        operation_id: OperationId,
        process_id: ProcessId,
    ) -> Self {
        Self {
            tool_runner: subsystems.security.tool_runner.clone(),
            tools: subsystems.corpus.tools.clone(),
            hook_registry: subsystems.corpus.hook_registry.clone(),
            storm_breaker: subsystems.security.storm_breaker.clone(),
            memory_queue: subsystems.session.memory_queue.clone(),
            session_approvals: subsystems.security.session_approvals.clone(),
            debug_perf: subsystems.debug_perf.clone(),
            self_field: subsystems.self_field.clone(),
            working_dir,
            session_id,
            turn_count,
            operation_id,
            process_id,
        }
    }

    /// Return the kernel operation id for this turn.
    #[allow(dead_code)]
    pub(crate) fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    /// Return the kernel process id for the main agent.
    #[allow(dead_code)]
    pub(crate) fn process_id(&self) -> ProcessId {
        self.process_id
    }

    /// Run one tool call with an already-granted execution permit.
    ///
    /// No `ExecutionPermit` means no side-effecting tool execution.
    /// Returns `(content, is_error)`.`
    pub(crate) async fn execute(
        &self,
        permit: &ExecutionPermit,
        _id: &str,
        name: &str,
        input: &serde_json::Value,
    ) -> (String, bool) {
        if permit.operation_id != self.operation_id
            || permit.process_id != self.process_id
            || permit.capability.0 != name
        {
            return (
                format!("admission permit does not match tool '{name}'"),
                true,
            );
        }

        // Rebind captured handles/values so the pipeline body below is identical
        // to the former `execute_tool` closure.
        let hook_registry_arc = &self.hook_registry;
        let session_approvals_arc = &self.session_approvals;
        let self_field_arc = &self.self_field;
        let tools_arc = &self.tools;
        let runner = &self.tool_runner;
        let debug_perf = &self.debug_perf;
        let storm_breaker_arc = &self.storm_breaker;
        let memory_queue_arc = &self.memory_queue;
        let name = name.to_string();
        let input = input.clone();
        let working_dir = self.working_dir.clone();
        let session_id = self.session_id.clone();
        let turn_count = self.turn_count;

        // --- PreTool hook ---
        {
            let hr = hook_registry_arc.lock().await;
            let ctx = HookContext {
                point: HookPoint::PreTool,
                session_id: session_id.clone(),
                turn_count,
                tool_name: Some(name.clone()),
                tool_input: Some(input.clone()),
                tool_result: None,
                message: None,
                metadata: HashMap::new(),
            };
            if let HookResult::Block { reason } = hr.execute(&ctx).await {
                return (format!("Blocked by hook: {}", reason), true);
            }
        }

        // --- OnMemoryRecall hook (when memory_search tool is invoked) ---
        if name == "memory_search" {
            let hr = hook_registry_arc.lock().await;
            let ctx = HookContext {
                point: HookPoint::OnMemoryRecall,
                session_id: session_id.clone(),
                turn_count,
                tool_name: Some(name.clone()),
                tool_input: Some(input.clone()),
                tool_result: None,
                message: None,
                metadata: HashMap::new(),
            };
            hr.execute(&ctx).await;
        }

        // --- Check session approvals (auto-approve if "always" was used) ---
        {
            let approvals = session_approvals_arc.lock().await;
            if let Some(&approved) = approvals.get(&name) {
                if approved {
                    info!(tool = %name, "Auto-approving tool from session approval cache");
                }
            }
        }

        // SelfField review per-tool
        {
            let tool_intent = Intent {
                action: name.clone(),
                parameters: input.clone(),
                source: IntentSource::Body,
                description: format!("Tool call: {}", name),
            };
            let sf_ctx = AbiContext::new(&session_id, working_dir.clone());
            let sf = self_field_arc.lock().await;
            match sf.review(&tool_intent, &sf_ctx).await {
                Ok(Verdict::Deny { reason }) => {
                    let _ = sf
                        .narrate("tool_blocked", &format!("{}: {}", name, reason))
                        .await;
                    return (format!("Tool blocked by SelfField: {}", reason), true);
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        tool = %name,
                        "SelfField review error, proceeding"
                    );
                }
                _ => {}
            }
        }

        let tool = {
            let reg = tools_arc.lock().await;
            reg.get(&name).cloned()
        };
        let exec_ctx = fabric::tool::ToolContext {
            working_dir,
            session_id: session_id.clone(),
            clock: std::sync::Arc::new(aletheon_kernel::chronos::SystemClock::new()),
        };
        let (content, is_error) = match tool {
            Some(t) => {
                let mut r = runner.lock().await;
                let res = r
                    .run(t.as_ref(), input.clone(), &exec_ctx, "chat-turn")
                    .await;
                (res.content, res.is_error)
            }
            None => (format!("Unknown tool: {}", name), true),
        };

        // --- PerfCounter: record tool call and errors ---
        debug_perf.record_tool_call(&name).await;
        if is_error {
            debug_perf.record_error();
        }

        // --- StormBreaker: track consecutive failures ---
        {
            let mut sb = storm_breaker_arc.lock().await;
            if let Some(directive) = sb.record(&name, is_error, &content) {
                let mut mq = memory_queue_arc.lock().await;
                mq.push(format!("\n[Storm Breaker] {}\n", directive));
            }
        }

        // --- PostTool hook ---
        {
            let hr = hook_registry_arc.lock().await;
            let ctx = HookContext {
                point: HookPoint::PostTool,
                session_id,
                turn_count,
                tool_name: Some(name.clone()),
                tool_input: None,
                tool_result: Some(fabric::hook::HookToolResult {
                    content: content.clone(),
                    is_error,
                    execution_time_ms: 0,
                }),
                message: None,
                metadata: HashMap::new(),
            };
            hr.execute(&ctx).await;
        }

        // tool_call_result is emitted via EventSink in ReActLoop (single source of truth).
        (content, is_error)
    }
}

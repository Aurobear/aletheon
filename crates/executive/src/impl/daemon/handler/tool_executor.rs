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

use aletheon_kernel::capability::ToolExecutor;
use corpus::security::storm_breaker::StormBreaker;
use corpus::CorpusToolExecutor;
use corpus::HookRegistry;
use dasein::SelfField;
use fabric::hook::{HookContext, HookPoint, HookResult};
use fabric::kernel::debug_bus::PerfCounter;
use fabric::types::admission::{ExecutionPermit, UsageReport};
use fabric::types::operation::{OperationId, ProcessId};
use fabric::{
    AgentProfileId, AuditEventId, CapabilityCall, CapabilityRequest, CapabilityResult,
    Context as AbiContext, ExitReason, Intent, IntentSource, NamespaceId, OperationKind,
    OperationRequest, ProcessSignal, SelfFieldOps, SpawnSpec, Verdict,
};

use crate::service::{
    CapabilityExecutionContext, CapabilityRuntimeFactory, CapabilityService,
    RegistryAuthorityProvider,
};

#[derive(Clone)]
pub(crate) struct CapabilityResources {
    pub(crate) kernel: Arc<aletheon_kernel::KernelRuntime>,
    pub(crate) tools: crate::core::corpus_group::ToolRegistryHandle,
    pub(crate) runner: crate::core::security_group::ToolRunnerHandle,
    pub(crate) hooks: crate::core::corpus_group::HookRegistryHandle,
    pub(crate) storm: Arc<Mutex<StormBreaker>>,
    pub(crate) memory_queue: Arc<Mutex<Vec<String>>>,
    pub(crate) approvals: Arc<Mutex<HashMap<String, bool>>>,
    pub(crate) perf: Arc<PerfCounter>,
    pub(crate) self_field: Arc<Mutex<SelfField>>,
}

/// Executes a single tool through the full guarded/hooked pipeline for one turn.
///
/// Holds the same subsystem handles the former `execute_tool` closure captured;
/// cheap to wrap in `Arc` and clone per tool call.
pub(crate) struct TurnToolExecutor {
    inner: Arc<CorpusToolExecutor>,
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
        resources: &CapabilityResources,
        session_id: String,
        turn_count: usize,
        working_dir: PathBuf,
        operation_id: OperationId,
        process_id: ProcessId,
    ) -> Self {
        let inner = Arc::new(CorpusToolExecutor::new(
            resources.tools.clone(),
            resources.runner.clone(),
            resources.kernel.clock(),
        ));
        Self {
            inner,
            hook_registry: resources.hooks.clone(),
            storm_breaker: resources.storm.clone(),
            memory_queue: resources.memory_queue.clone(),
            session_approvals: resources.approvals.clone(),
            debug_perf: resources.perf.clone(),
            self_field: resources.self_field.clone(),
            working_dir,
            session_id,
            turn_count,
            operation_id,
            process_id,
        }
    }

    /// Return the kernel operation id for this turn.
    /// Reserved for future admission-controller diagnostics; not yet called.
    #[allow(dead_code)]
    pub(crate) fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    /// Return the kernel process id for the main agent.
    /// Reserved for future admission-controller diagnostics; not yet called.
    #[allow(dead_code)]
    pub(crate) fn process_id(&self) -> ProcessId {
        self.process_id
    }

    /// Run one tool call with an already-granted execution permit.
    ///
    /// No `ExecutionPermit` means no side-effecting tool execution.
    /// Returns `(content, is_error)`.`
    async fn execute(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
    ) -> CapabilityResult {
        let name = &request.call.name;
        let input = &request.call.input;
        if permit.operation_id != self.operation_id
            || permit.process_id != self.process_id
            || permit.capability.0 != *name
        {
            return self.error_result(
                request,
                permit,
                format!("admission permit does not match tool '{name}'"),
            );
        }

        // Rebind captured handles/values so the pipeline body below is identical
        // to the former `execute_tool` closure.
        let hook_registry_arc = &self.hook_registry;
        let session_approvals_arc = &self.session_approvals;
        let self_field_arc = &self.self_field;
        let inner = &self.inner;
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
                return self.error_result(request, permit, format!("Blocked by hook: {reason}"));
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
                    return self.error_result(
                        request,
                        permit,
                        format!("Tool blocked by SelfField: {reason}"),
                    );
                }
                Err(e) => {
                    return self.error_result(
                        request,
                        permit,
                        format!("SelfField review failed: {e}"),
                    );
                }
                _ => {}
            }
        }

        let mut result = inner.execute_with_permit(request, permit).await;
        let content = result.output.clone();
        let is_error = result.is_error;

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
        result.output = content;
        result.is_error = is_error;
        result
    }

    fn error_result(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
        output: String,
    ) -> CapabilityResult {
        CapabilityResult {
            call_id: request.call.call_id.clone(),
            output,
            is_error: true,
            usage: UsageReport {
                permit_id: permit.id,
                exit_code: Some(1),
                ..Default::default()
            },
            audit_id: Some(AuditEventId::new()),
        }
    }
}

#[async_trait::async_trait]
impl ToolExecutor for TurnToolExecutor {
    async fn execute_with_permit(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
    ) -> CapabilityResult {
        self.execute(request, permit).await
    }
}

/// Executive composition adapter for every non-turn capability caller.
pub(crate) struct ProductionCapabilityService {
    resources: CapabilityResources,
}

impl ProductionCapabilityService {
    pub(crate) fn new(resources: CapabilityResources) -> Self {
        Self { resources }
    }

    async fn invoke_existing(
        resources: &CapabilityResources,
        context: CapabilityExecutionContext,
        call: CapabilityCall,
    ) -> CapabilityResult {
        let executor = Arc::new(TurnToolExecutor::new(
            resources,
            context.session_id.clone(),
            context.turn_count,
            context.working_dir.clone(),
            context.operation_id,
            context.process_id,
        ));
        let authority = Arc::new(
            RegistryAuthorityProvider::new(
                corpus::tool_risk_levels(&resources.tools).await,
                context.principal,
                context.session_id,
                context.working_dir,
                context.sandbox,
                context.cancel,
            )
            .with_agent_context(context.agent),
        );
        CapabilityRuntimeFactory::build(resources.kernel.admission(), executor, authority)
            .invoke(call)
            .await
    }

    fn unavailable(call: &CapabilityCall, message: impl Into<String>) -> CapabilityResult {
        CapabilityResult {
            call_id: call.call_id.clone(),
            output: message.into(),
            is_error: true,
            usage: UsageReport::default(),
            audit_id: None,
        }
    }
}

#[async_trait::async_trait]
impl CapabilityService for ProductionCapabilityService {
    async fn invoke(
        &self,
        context: Option<CapabilityExecutionContext>,
        mut call: CapabilityCall,
        cancel: tokio_util::sync::CancellationToken,
    ) -> CapabilityResult {
        if let Some(mut context) = context {
            context.cancel = cancel;
            call.process_id = context.process_id;
            call.operation_id = context.operation_id;
            return Self::invoke_existing(&self.resources, context, call).await;
        }

        // External/provider callers without a parent lifecycle receive a
        // bounded transient lifecycle owned and settled entirely here.
        let kernel = &self.resources.kernel;
        let process = match kernel
            .spawn_process(SpawnSpec {
                profile: AgentProfileId("capability-client".into()),
                namespace: NamespaceId("external-capability".into()),
                initial_operation: None,
                ..SpawnSpec::default()
            })
            .await
        {
            Ok(process) => process,
            Err(error) => return Self::unavailable(&call, format!("kernel spawn failed: {error}")),
        };
        if let Err(error) = kernel
            .signal_process(process.id, ProcessSignal::Start)
            .await
        {
            let _ = kernel
                .terminate_process(process.id, ExitReason::Failed(error.to_string()))
                .await;
            return Self::unavailable(&call, format!("kernel start failed: {error}"));
        }
        let operation = match kernel
            .submit_operation(OperationRequest {
                owner: process.id,
                parent: None,
                kind: OperationKind::CapabilityCall,
                deadline: None,
            })
            .await
        {
            Ok(operation) => operation,
            Err(error) => {
                let _ = kernel
                    .terminate_process(process.id, ExitReason::Failed(error.to_string()))
                    .await;
                return Self::unavailable(&call, format!("operation submit failed: {error}"));
            }
        };
        if let Err(error) = kernel.start_operation(operation.id).await {
            let _ = kernel
                .terminate_process(process.id, ExitReason::Failed(error.to_string()))
                .await;
            return Self::unavailable(&call, format!("operation start failed: {error}"));
        }
        call.process_id = process.id;
        call.operation_id = operation.id;
        let context = CapabilityExecutionContext {
            agent: None,
            process_id: process.id,
            operation_id: operation.id,
            principal: fabric::PrincipalId("external-capability".into()),
            session_id: "external-capability".into(),
            working_dir: std::env::current_dir().unwrap_or_default(),
            sandbox: fabric::SandboxRequirement::NotRequired,
            cancel,
            turn_count: 0,
            action_loop: None,
        };
        let result = Self::invoke_existing(&self.resources, context, call).await;
        if result.is_error {
            let _ = kernel
                .fail_operation(operation.id, result.output.clone())
                .await;
        } else {
            let _ = kernel.succeed_operation(operation.id).await;
        }
        let exit = if result.is_error {
            ExitReason::Failed("capability failed".into())
        } else {
            ExitReason::Completed
        };
        let _ = kernel.terminate_process(process.id, exit).await;
        result
    }
}

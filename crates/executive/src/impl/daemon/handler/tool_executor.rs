//! `TurnToolExecutor` — the per-tool execution pipeline for a chat turn.
//!
//! Extracted from the former inline `execute_tool` closure (previously in chat.rs, now deleted)
//! (RFC-018 D5 seam 3 / issue #4). Runs one tool through the full pipeline:
//! PreTool hook → OnMemoryRecall hook → session-approval check → SelfField
//! review → scoped Corpus activation → `ExecutionPermit`-guarded invocation →
//! PerfCounter → StormBreaker → PostTool hook,
//! returning `(content, is_error)`.
//!
//! Behaviour is identical to the previous closure; this only gives the pipeline
//! a name and a home. It is adapted to the harness's
//! `Fn(&str, &str, &Value) -> Future<Output=(String, bool)>` executor parameter
//! by a thin `Arc<Self>` closure wrapper.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};
use tracing::info;

use aletheon_kernel::capability::ToolExecutor;
use corpus::security::storm_breaker::StormBreaker;
use corpus::{ActivatedCorpusExecutor, CorpusService, ExtensionGrant, ExtensionSnapshot};
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

use crate::service::admin_service::ScopedApprovalCache;
use crate::service::{
    CapabilityExecutionContext, CapabilityRuntimeFactory, CapabilityService,
    RegistryAuthorityProvider,
};

#[derive(Clone)]
pub(crate) struct CapabilityResources {
    pub(crate) kernel: Arc<aletheon_kernel::KernelRuntime>,
    pub(crate) corpus: Arc<dyn CorpusService>,
    pub(crate) capabilities: Arc<RwLock<Vec<fabric::CapabilityId>>>,
    pub(crate) storm: Arc<Mutex<StormBreaker>>,
    pub(crate) memory_queue: Arc<Mutex<Vec<String>>>,
    pub(crate) approvals: ScopedApprovalCache,
    pub(crate) perf: Arc<PerfCounter>,
    pub(crate) self_field: Arc<Mutex<SelfField>>,
    pub(crate) extension_decisions:
        Arc<dyn crate::service::extension_service::ExtensionDecisionSink>,
}

/// Executes a single tool through the full guarded/hooked pipeline for one turn.
///
/// Holds the same subsystem handles the former `execute_tool` closure captured;
/// cheap to wrap in `Arc` and clone per tool call.
pub(crate) struct TurnToolExecutor {
    inner: Arc<dyn ToolExecutor>,
    corpus: Arc<dyn CorpusService>,
    storm_breaker: Arc<Mutex<StormBreaker>>,
    memory_queue: Arc<Mutex<Vec<String>>>,
    session_approvals: ScopedApprovalCache,
    debug_perf: Arc<PerfCounter>,
    self_field: Arc<Mutex<SelfField>>,
    working_dir: PathBuf,
    session_id: String,
    turn_count: usize,
    repo_hooks_trusted: bool,
    /// Kernel operation id for this turn (used by admission controller).
    operation_id: OperationId,
    /// Kernel process id for the main agent (used by admission controller).
    process_id: ProcessId,
}

impl TurnToolExecutor {
    /// Build an executor for one turn, cloning the needed subsystem handles.
    pub(crate) fn new(
        resources: &CapabilityResources,
        inner: Arc<dyn ToolExecutor>,
        session_id: String,
        turn_count: usize,
        repo_hooks_trusted: bool,
        working_dir: PathBuf,
        operation_id: OperationId,
        process_id: ProcessId,
    ) -> Self {
        Self {
            inner,
            corpus: resources.corpus.clone(),
            storm_breaker: resources.storm.clone(),
            memory_queue: resources.memory_queue.clone(),
            session_approvals: resources.approvals.clone(),
            debug_perf: resources.perf.clone(),
            self_field: resources.self_field.clone(),
            working_dir,
            session_id,
            turn_count,
            repo_hooks_trusted,
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
        sink: Option<&mut fabric::ToolEventSink>,
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
        let corpus = &self.corpus;
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
        let repo_hooks_trusted = self.repo_hooks_trusted;

        // --- PreTool hook ---
        {
            let ctx = HookContext {
                point: HookPoint::PreTool,
                session_id: session_id.clone(),
                turn_count,
                tool_name: Some(name.clone()),
                tool_input: Some(input.clone()),
                tool_result: None,
                message: None,
                metadata: HashMap::from([
                    ("workspace_root".into(), working_dir.display().to_string()),
                    ("repo_hooks_trusted".into(), repo_hooks_trusted.to_string()),
                ]),
            };
            if let HookResult::Block { reason } = corpus.execute_hook(&ctx).await {
                corpus
                    .execute_hook(&HookContext {
                        point: HookPoint::PermissionDenied,
                        session_id: session_id.clone(),
                        turn_count,
                        tool_name: Some(name.clone()),
                        tool_input: Some(input.clone()),
                        tool_result: None,
                        message: Some(reason.clone()),
                        metadata: ctx.metadata.clone(),
                    })
                    .await;
                return self.error_result(request, permit, format!("Blocked by hook: {reason}"));
            }
        }

        // --- OnMemoryRecall hook (when memory_search tool is invoked) ---
        if name == "memory_search" {
            let ctx = HookContext {
                point: HookPoint::OnMemoryRecall,
                session_id: session_id.clone(),
                turn_count,
                tool_name: Some(name.clone()),
                tool_input: Some(input.clone()),
                tool_result: None,
                message: None,
                metadata: HashMap::from([
                    ("workspace_root".into(), working_dir.display().to_string()),
                    ("repo_hooks_trusted".into(), repo_hooks_trusted.to_string()),
                ]),
            };
            corpus.execute_hook(&ctx).await;
        }

        // --- Check session approvals (auto-approve if "always" was used) ---
        {
            if session_approvals_arc
                .is_allowed(
                    &request.authority.principal,
                    &request.authority.thread_id,
                    &name,
                )
                .await
            {
                info!(tool = %name, "Auto-approving tool from scoped thread approval cache");
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

        let mut result = match sink {
            Some(sink) => {
                inner
                    .execute_streaming_with_permit(request, permit, sink)
                    .await
            }
            None => inner.execute_with_permit(request, permit).await,
        };
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
            let ctx = HookContext {
                point: if is_error {
                    HookPoint::PostToolFailure
                } else {
                    HookPoint::PostTool
                },
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
                metadata: HashMap::from([
                    ("workspace_root".into(), working_dir.display().to_string()),
                    ("repo_hooks_trusted".into(), repo_hooks_trusted.to_string()),
                ]),
            };
            corpus.execute_hook(&ctx).await;
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
            patch_delta: None,
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
        self.execute(request, permit, None).await
    }

    async fn execute_streaming_with_permit(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
        sink: &mut fabric::ToolEventSink,
    ) -> CapabilityResult {
        self.execute(request, permit, Some(sink)).await
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
        let prepared = match prepare_corpus(resources, &context).await {
            Ok(prepared) => prepared,
            Err(error) => return Self::unavailable(&call, error.to_string()),
        };
        let executor = Arc::new(TurnToolExecutor::new(
            resources,
            prepared.executor,
            context.session_id.clone(),
            context.turn_count,
            context.repo_hooks_trusted,
            context.workspace.cwd().to_path_buf(),
            context.operation_id,
            context.process_id,
        ));
        let authority_working_dir = context.workspace.cwd().to_path_buf();
        let turn_event_sender = context.turn_event_sender.clone();
        let authority = Arc::new(
            RegistryAuthorityProvider::new(
                prepared.risk_by_tool,
                context.principal,
                context.connection_id,
                context.thread_id,
                context.turn_id,
                context.workspace,
                context.session_id,
                authority_working_dir,
                context.sandbox,
                context.cancel,
            )
            .with_agent_context(context.agent)
            .with_turn_event_sender(turn_event_sender),
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
            patch_delta: None,
        }
    }
}

pub(crate) struct PreparedCorpus {
    pub(crate) snapshot: ExtensionSnapshot,
    pub(crate) risk_by_tool: HashMap<String, fabric::types::admission::RiskLevel>,
    pub(crate) executor: Arc<dyn ToolExecutor>,
}

pub(crate) async fn prepare_corpus(
    resources: &CapabilityResources,
    context: &CapabilityExecutionContext,
) -> anyhow::Result<PreparedCorpus> {
    let grant = ExtensionGrant {
        grant_id: uuid::Uuid::new_v4().to_string(),
        principal: context.principal.clone(),
        session_id: context.session_id.clone(),
        agent_id: context
            .agent
            .as_ref()
            .map(|agent| agent.caller_root_agent_id),
        capabilities: resources.capabilities.read().await.clone(),
        resources: fabric::CapabilityScope::default(),
    };
    let snapshot = resources.corpus.catalog(&grant).await?;
    let activated = crate::service::ExtensionService::new(
        resources.corpus.clone(),
        resources.extension_decisions.clone(),
    )
    .activate(
        grant,
        snapshot
            .entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect(),
        &crate::service::SessionExtensionPolicy::default(),
    )
    .await?;
    let snapshot = activated.snapshot;
    let risk_by_tool = snapshot
        .entries
        .iter()
        .filter_map(|entry| {
            entry
                .primary_capability()
                .map(|capability| (capability.0.clone(), entry.risk))
        })
        .collect();
    let executor = Arc::new(ActivatedCorpusExecutor::new(
        resources.corpus.clone(),
        activated.receipt.id,
    ));
    Ok(PreparedCorpus {
        snapshot,
        risk_by_tool,
        executor,
    })
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
            connection_id: fabric::ConnectionId::new(),
            thread_id: fabric::ThreadId("external-capability".into()),
            turn_id: fabric::TurnId::new(),
            workspace: fabric::WorkspacePolicy::from_resolved_roots(
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/tmp")),
                Vec::new(),
            )
            .expect("external capability workspace is absolute"),
            session_id: "external-capability".into(),
            working_dir: std::env::current_dir().unwrap_or_default(),
            sandbox: fabric::SandboxRequirement::NotRequired,
            cancel,
            turn_count: 0,
            repo_hooks_trusted: false,
            action_loop: None,
            streaming_tools: false,
            turn_event_sender: None,
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

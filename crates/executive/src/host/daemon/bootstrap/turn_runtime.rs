//! Private production adapters for turn-blocking runtime ports.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use async_trait::async_trait;
use fabric::hook::{HookContext, HookResult};
use fabric::{AdmissionController, Clock, ContentBlock, LlmProvider, Message, Role};
use tokio::sync::{mpsc, Mutex};

use crate::application::governed_capability::{
    CapabilityExecutionContext, CapabilityRuntimeFactory, RegistryAuthorityProvider,
};
use crate::application::turn_runtime_ports::{
    ActiveAgentProfilePort, ApprovalNotice, GovernedTurnCapabilityPort, ModelSelectionPort,
    PreparedCapabilities, ResolvedTurnProfile, SelfPolicyPort, StormStatePort, TurnApprovalPort,
    TurnConfigPort, TurnHookPort, TurnObservabilityPort, TurnRuntimePorts, TurnSessionStatePort,
};
use crate::host::daemon::handler::tool_executor::{prepare_corpus, TurnToolExecutor};
use crate::host::daemon::model_router::ModelRouter;

pub(super) fn register_configured_hooks(
    registry: &mut corpus::HookRegistry,
    config: &crate::composition::config::HooksConfig,
) {
    for (point, scripts) in [
        (
            fabric::hook::HookPoint::OnSessionStart,
            &config.on_session_start,
        ),
        (fabric::hook::HookPoint::PreTurn, &config.pre_turn),
        (fabric::hook::HookPoint::PreTool, &config.pre_tool),
        (fabric::hook::HookPoint::PostTool, &config.post_tool),
        (
            fabric::hook::HookPoint::PostToolFailure,
            &config.post_tool_failure,
        ),
        (
            fabric::hook::HookPoint::PermissionDenied,
            &config.permission_denied,
        ),
        (
            fabric::hook::HookPoint::UserPromptSubmit,
            &config.user_prompt_submit,
        ),
        (fabric::hook::HookPoint::Notification, &config.notification),
        (
            fabric::hook::HookPoint::SubagentStart,
            &config.subagent_start,
        ),
        (fabric::hook::HookPoint::SubagentStop, &config.subagent_stop),
        (fabric::hook::HookPoint::PreCompact, &config.pre_compact),
        (fabric::hook::HookPoint::PostCompact, &config.post_compact),
        (
            fabric::hook::HookPoint::OnSessionEnd,
            &config.on_session_end,
        ),
    ] {
        for (index, script) in scripts.iter().enumerate() {
            registry.register(corpus::hook::registry::RegisteredHook {
                name: format!("config:{point:?}:{index}"),
                source: "config".into(),
                script_path: Some(
                    crate::host::daemon::handler::format::expand_tilde(script).into(),
                ),
                point,
                priority: 100,
            });
        }
    }
}

pub(super) struct TurnRuntimeResources {
    pub(crate) corpus: Arc<dyn corpus::CorpusService>,
    pub(crate) storm: Arc<Mutex<corpus::security::storm_breaker::StormBreaker>>,
    pub(crate) model_router: Arc<ModelRouter>,
    pub(crate) default_llm: Arc<dyn LlmProvider>,
    pub(crate) self_policy: Arc<dyn SelfPolicyPort>,
    pub(crate) approval_rx:
        Arc<Mutex<mpsc::Receiver<corpus::security::socket_approval::PendingApproval>>>,
    pub(crate) pending_approvals: crate::application::admin_service::PendingApprovals,
    pub(crate) capabilities: crate::host::daemon::handler::tool_executor::CapabilityResources,
    pub(crate) admission: Arc<dyn AdmissionController>,
    pub(crate) sessions: Arc<
        Mutex<HashMap<String, Arc<Mutex<crate::host::daemon::session_manager::SessionManager>>>>,
    >,
    pub(crate) default_session_id: Arc<Mutex<String>>,
    pub(crate) session_created_at: Arc<Mutex<HashMap<String, fabric::MonoTime>>>,
    pub(crate) data_dir: std::path::PathBuf,
    pub(crate) context_window: usize,
    pub(crate) cached_prefix: Arc<Mutex<String>>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) memory: Arc<dyn mnemosyne::MemoryService>,
    pub(crate) config: Arc<dyn TurnConfigPort>,
    pub(crate) performance: Arc<fabric::kernel::debug_bus::PerfCounter>,
    pub(crate) active_profile: Arc<dyn ActiveAgentProfilePort>,
}

pub(super) fn compose_turn_runtime(resources: TurnRuntimeResources) -> TurnRuntimePorts {
    let corpus = resources.corpus;
    let execute_hook: Arc<HookExecutionFn> = Arc::new(move |context| {
        let corpus = corpus.clone();
        Box::pin(async move { corpus.execute_hook(&context).await })
    });
    let hooks: Arc<dyn TurnHookPort> = Arc::new(ProductionTurnHooks { execute_hook });
    let active_profile = resources.active_profile.clone();
    TurnRuntimePorts {
        hooks: hooks.clone(),
        storm: Arc::new(ProductionStormState {
            state: resources.storm,
        }),
        models: Arc::new(ProductionModelSelection {
            router: resources.model_router,
            default_llm: resources.default_llm.clone(),
        }),
        self_policy: resources.self_policy,
        approvals: Arc::new(ProductionTurnApprovals {
            receiver: resources.approval_rx,
            pending: resources.pending_approvals,
        }),
        capabilities: Arc::new(ProductionGovernedCapabilities {
            resources: resources.capabilities,
            admission: resources.admission,
            hooks: hooks.clone(),
            active_profile: active_profile.clone(),
        }),
        sessions: Arc::new(ProductionTurnSessions {
            registry: resources.sessions,
            default_id: resources.default_session_id,
            created_at: resources.session_created_at,
            data_dir: resources.data_dir,
            context_window: resources.context_window,
            cached_prefix: resources.cached_prefix,
            active_profile,
            clock: resources.clock,
            llm: resources.default_llm,
            memory_service: resources.memory,
            hooks,
        }),
        config: resources.config,
        observability: Arc::new(ProductionTurnObservability {
            performance: resources.performance,
        }),
    }
}

struct ProductionTurnHooks {
    execute_hook: Arc<HookExecutionFn>,
}

type HookExecutionFn =
    dyn Fn(HookContext) -> Pin<Box<dyn Future<Output = HookResult> + Send>> + Send + Sync;

#[async_trait]
impl TurnHookPort for ProductionTurnHooks {
    async fn execute(&self, context: HookContext) -> HookResult {
        (self.execute_hook)(context).await
    }
}

struct ProductionStormState {
    state: Arc<Mutex<corpus::security::storm_breaker::StormBreaker>>,
}

#[async_trait]
impl StormStatePort for ProductionStormState {
    async fn reset(&self) {
        self.state.lock().await.reset();
    }

    async fn failure_count(&self) -> usize {
        self.state.lock().await.failure_count()
    }
}

struct ProductionModelSelection {
    router: Arc<ModelRouter>,
    default_llm: Arc<dyn LlmProvider>,
}

impl ModelSelectionPort for ProductionModelSelection {
    fn select(&self, message: &str) -> Arc<dyn LlmProvider> {
        let task = self.router.classify_message(message);
        match self.router.create_provider(task) {
            Ok(provider) => {
                tracing::info!(task=?task, model=provider.name(), "Model selected by router");
                Arc::from(provider)
            }
            Err(error) => {
                tracing::warn!(%error, task=?task, "ModelRouter failed, using default");
                self.default_llm.clone()
            }
        }
    }
}

struct ProductionTurnSessions {
    registry: Arc<
        Mutex<HashMap<String, Arc<Mutex<crate::host::daemon::session_manager::SessionManager>>>>,
    >,
    default_id: Arc<Mutex<String>>,
    created_at: Arc<Mutex<HashMap<String, fabric::MonoTime>>>,
    data_dir: std::path::PathBuf,
    context_window: usize,
    cached_prefix: Arc<Mutex<String>>,
    active_profile: Arc<dyn ActiveAgentProfilePort>,
    clock: Arc<dyn Clock>,
    llm: Arc<dyn LlmProvider>,
    memory_service: Arc<dyn mnemosyne::MemoryService>,
    hooks: Arc<dyn TurnHookPort>,
}

impl ProductionTurnSessions {
    async fn manager(
        &self,
        requested_session_id: &str,
    ) -> anyhow::Result<(
        String,
        Arc<Mutex<crate::host::daemon::session_manager::SessionManager>>,
    )> {
        let session_id = if requested_session_id.trim().is_empty() {
            self.default_id.lock().await.clone()
        } else {
            requested_session_id.to_owned()
        };
        if let Some(manager) = self.registry.lock().await.get(&session_id).cloned() {
            return Ok((session_id, manager));
        }
        let manager = crate::host::daemon::session_manager::SessionManager::new(
            &self.data_dir,
            session_id.clone(),
            self.context_window,
            self.clock.clone(),
        )
        .await?;
        let manager = Arc::new(Mutex::new(manager));
        self.registry
            .lock()
            .await
            .insert(session_id.clone(), manager.clone());
        self.created_at
            .lock()
            .await
            .insert(session_id.clone(), self.clock.mono_now());
        Ok((session_id, manager))
    }

    async fn budget_plan(
        &self,
        manager: &crate::host::daemon::session_manager::SessionManager,
        pending_user: &str,
    ) -> anyhow::Result<mnemosyne::runtime::ContextBudgetPlan> {
        use mnemosyne::runtime::{ContextBudgetInput, ContextBudgetPlanner};

        let profile = self.active_profile.snapshot().await?;
        let prefix_tokens = Message::system(self.cached_prefix.lock().await.clone())
            .estimate_tokens()
            .saturating_add(Message::system(profile.system_prompt).estimate_tokens());
        // Real per-tool schema token estimate (name + description +
        // serialized JSON input_schema), not a fixed ~74 tokens/tool
        // placeholder — the provider sends the full schema for every
        // allowed tool, which for large toolsets is thousands of tokens.
        let tool_schema_tokens = self.active_profile.tool_schema_tokens().await?;
        let pending_user_input_tokens = if pending_user.is_empty() {
            0
        } else {
            Message::user(pending_user).estimate_tokens()
        };
        let effective_context_window = self.context_window.min(profile.max_input_tokens as usize);
        Ok(ContextBudgetPlanner::plan(ContextBudgetInput {
            model_context_window: self.context_window,
            profile_input_limit: profile.max_input_tokens as usize,
            system_and_skill_prefix_tokens: prefix_tokens,
            tool_schema_tokens,
            reserved_output_tokens: profile.max_output_tokens as usize,
            pending_user_input_tokens,
            safety_margin_tokens: (effective_context_window / 20).max(1_024),
            current_history_tokens: manager.estimate_tokens(),
        }))
    }
}

#[async_trait]
impl TurnSessionStatePort for ProductionTurnSessions {
    async fn current(&self, requested_session_id: &str) -> anyhow::Result<(String, usize)> {
        let (session_id, manager) = self.manager(requested_session_id).await?;
        let turn_count = manager.lock().await.turn_count();
        Ok((session_id, turn_count))
    }

    async fn begin_user(
        &self,
        requested_session_id: &str,
        message: &str,
    ) -> anyhow::Result<(String, usize)> {
        let (session_id, manager) = self.manager(requested_session_id).await?;
        let turn_count = {
            let mut manager = manager.lock().await;
            let mut plan = self.budget_plan(&manager, message).await?;
            if plan.action == mnemosyne::runtime::BudgetAction::HardCompact {
                tracing::warn!(
                    projected_tokens = plan.projected_history_tokens,
                    history_budget = plan.history_budget,
                    "Hard watermark exceeded — compacting before model call"
                );
                let first_applied = match manager.compact_to_budget(&*self.llm, &plan, false).await
                {
                    Ok(applied) => applied,
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            "First hard-watermark compaction attempt was rejected; retrying aggressively"
                        );
                        false
                    }
                };
                plan = self.budget_plan(&manager, message).await?;
                if !first_applied || plan.action == mnemosyne::runtime::BudgetAction::HardCompact {
                    if let Err(error) = manager.compact_to_budget(&*self.llm, &plan, true).await {
                        tracing::warn!(
                            error = %error,
                            "Aggressive hard-watermark compaction attempt was rejected"
                        );
                    }
                    plan = self.budget_plan(&manager, message).await?;
                }
                if plan.action == mnemosyne::runtime::BudgetAction::HardCompact {
                    return Err(anyhow::anyhow!(
                        "Context cannot safely fit the request after two validated compaction \
                         attempts. Try /new, switch to a larger-window model, or export diagnostics."
                    ));
                }
            }

            // The pending input becomes part of the projection only after the
            // hard-watermark gate proves that the request can be submitted.
            manager.push_user(message).await;
            manager.turn_count()
        };
        if let Err(error) = self
            .memory_service
            .record(mnemosyne::ExperienceEvent::Message {
                session: session_id.clone(),
                role: "user".into(),
                content: message.to_owned(),
                metadata: mnemosyne::MemoryMetadata::local(
                    format!("message:{session_id}:user:{turn_count}"),
                    format!("{session_id}:user:{turn_count}"),
                    fabric::wall_to_datetime(self.clock.wall_now()),
                ),
            })
            .await
        {
            tracing::warn!(%error, "user memory projection failed");
        }
        self.hooks
            .execute(HookContext {
                point: fabric::hook::HookPoint::OnMemoryStore,
                session_id: session_id.clone(),
                turn_count,
                tool_name: None,
                tool_input: None,
                tool_result: None,
                message: Some(message.to_owned()),
                metadata: HashMap::new(),
            })
            .await;
        Ok((session_id, turn_count))
    }

    async fn finish(
        &self,
        requested_session_id: &str,
        succeeded: bool,
        tool_calls: &[(String, String, serde_json::Value)],
        tool_results: &[(String, String, bool)],
        output: &str,
    ) -> anyhow::Result<usize> {
        let (session_id, manager_handle) = self.manager(requested_session_id).await?;
        let mut manager = manager_handle.lock().await;
        if succeeded {
            if !tool_calls.is_empty() {
                manager
                    .push_message(Message {
                        role: Role::Assistant,
                        content: tool_calls
                            .iter()
                            .map(|(id, name, input)| ContentBlock::ToolUse {
                                id: id.clone(),
                                name: name.clone(),
                                input: input.clone(),
                            })
                            .collect(),
                    })
                    .await;
                manager
                    .push_message(Message {
                        role: Role::User,
                        content: tool_results
                            .iter()
                            .map(|(id, content, is_error)| ContentBlock::ToolResult {
                                tool_use_id: id.clone(),
                                content: content.clone(),
                                is_error: *is_error,
                            })
                            .collect(),
                    })
                    .await;
            }
            manager.push_assistant(output).await;
            let turn_count = manager.turn_count();

            let plan = self.budget_plan(&manager, "").await?;
            let tokens_before = manager.estimate_tokens();
            if plan.action == mnemosyne::runtime::BudgetAction::None
                || !manager.compaction_needed_for(&plan)
            {
                return Ok(turn_count);
            }
            drop(manager);
            self.hooks
                .execute(HookContext {
                    point: fabric::hook::HookPoint::PreCompact,
                    session_id: session_id.clone(),
                    turn_count,
                    tool_name: None,
                    tool_input: None,
                    tool_result: None,
                    message: None,
                    metadata: HashMap::from([
                        ("mode".into(), "automatic".into()),
                        ("tokens_before".into(), tokens_before.to_string()),
                    ]),
                })
                .await;
            let mut manager = manager_handle.lock().await;
            let compacted = match manager.compact_to_budget(&*self.llm, &plan, false).await {
                Ok(compacted) => compacted,
                Err(error) => {
                    // A soft-watermark pass is opportunistic. Validation
                    // rejection leaves the projection untouched and must not
                    // turn an otherwise successful model response into a
                    // failed turn.
                    tracing::warn!(
                        error = %error,
                        "Soft-watermark compaction was rejected; preserving current projection"
                    );
                    false
                }
            };
            if compacted {
                let tokens_after = manager.estimate_tokens();
                drop(manager);
                self.hooks
                    .execute(HookContext {
                        point: fabric::hook::HookPoint::PostCompact,
                        session_id,
                        turn_count,
                        tool_name: None,
                        tool_input: None,
                        tool_result: None,
                        message: None,
                        metadata: HashMap::from([
                            ("mode".into(), "automatic".into()),
                            ("tokens_before".into(), tokens_before.to_string()),
                            ("tokens_after".into(), tokens_after.to_string()),
                        ]),
                    })
                    .await;
                return Ok(turn_count);
            }
            return Ok(manager.turn_count());
        }
        Ok(manager.turn_count())
    }
}

struct ProductionTurnObservability {
    performance: Arc<fabric::kernel::debug_bus::PerfCounter>,
}

impl TurnObservabilityPort for ProductionTurnObservability {
    fn record_turn(&self, tokens_in: u64, tokens_out: u64) {
        self.performance.record_turn(tokens_in, tokens_out);
    }
}

struct ProductionTurnApprovals {
    receiver: Arc<Mutex<mpsc::Receiver<corpus::security::socket_approval::PendingApproval>>>,
    pending: crate::application::admin_service::PendingApprovals,
}

#[async_trait]
impl TurnApprovalPort for ProductionTurnApprovals {
    async fn next(&self) -> Option<ApprovalNotice> {
        let pending = self.receiver.lock().await.recv().await?;
        let approval_id = self
            .pending
            .insert(
                pending.request.owner.clone(),
                pending.request.turn_id,
                pending.request.call_id.clone(),
                pending.request.tool.clone(),
                pending.request.connection_id.clone(),
                pending.respond,
            )
            .await;
        let notice = ApprovalNotice {
            approval_id: approval_id.clone(),
            tool: pending.request.tool,
            action_summary: pending.request.action_summary,
            risk_level: pending.request.risk_level,
            detail: pending.request.detail,
        };
        Some(notice)
    }
}

struct ProductionGovernedCapabilities {
    resources: crate::host::daemon::handler::tool_executor::CapabilityResources,
    admission: Arc<dyn AdmissionController>,
    hooks: Arc<dyn TurnHookPort>,
    active_profile: Arc<dyn ActiveAgentProfilePort>,
}

pub(super) struct ProductionActiveAgentProfile {
    current: Arc<Mutex<String>>,
    profiles: Arc<crate::adapters::runtime::AgentProfileRegistry>,
}

impl ProductionActiveAgentProfile {
    pub(super) fn new(
        current: Arc<Mutex<String>>,
        profiles: Arc<crate::adapters::runtime::AgentProfileRegistry>,
    ) -> Self {
        Self { current, profiles }
    }
}

#[async_trait]
impl ActiveAgentProfilePort for ProductionActiveAgentProfile {
    async fn snapshot(&self) -> anyhow::Result<ResolvedTurnProfile> {
        // Clone the name under the switch lock, then resolve an owned profile.
        // No live mutable profile state survives this call.
        let profile_name = self.current.lock().await.clone();
        let resolved = self
            .profiles
            .resolve_by_name(&profile_name)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let p = &resolved.profile;
        Ok(ResolvedTurnProfile {
            profile_name: p.profile_name.clone(),
            allowed_tools: p.allowed_tools.iter().cloned().collect(),
            system_prompt: p.system_prompt.clone(),
            model_policy: Some(p.model.clone()),
            max_iterations: p.max_iterations,
            max_input_tokens: p.max_input_tokens,
            max_output_tokens: p.max_output_tokens,
            max_tool_calls: p.max_tool_calls,
            max_elapsed_ms: p.max_elapsed_ms,
            approval_policy: p.approval_policy,
            tool_timeout_ms: p.tool_timeout_ms,
        })
    }

    async fn tool_schema_tokens(&self) -> anyhow::Result<usize> {
        let profile_name = self.current.lock().await.clone();
        let resolved = self
            .profiles
            .resolve_by_name(&profile_name)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        // Estimate the real serialized cost of each tool's schema (name +
        // description + JSON input_schema) using the same chars/4 + overhead
        // heuristic as `Message::estimate_tokens`, instead of a fixed
        // per-tool constant that ignores actual schema size.
        Ok(resolved
            .tools
            .iter()
            .map(|tool| {
                let serialized_chars =
                    tool.name.len() + tool.description.len() + tool.input_schema.to_string().len();
                serialized_chars / 4 + 10
            })
            .sum())
    }
}

fn constrain_profile_capabilities(
    snapshot: &ResolvedTurnProfile,
    definitions: &mut Vec<fabric::ToolDefinition>,
    risk_by_tool: &mut HashMap<String, fabric::types::admission::RiskLevel>,
) {
    definitions.retain(|definition| snapshot.allowed_tools.contains(&definition.name));
    risk_by_tool.retain(|name, _| snapshot.allowed_tools.contains(name));
}

#[async_trait]
impl GovernedTurnCapabilityPort for ProductionGovernedCapabilities {
    async fn prepare(
        &self,
        context: CapabilityExecutionContext,
    ) -> anyhow::Result<PreparedCapabilities> {
        let stream_sender = context
            .streaming_tools
            .then(|| context.turn_event_sender.clone())
            .flatten();
        // One immutable snapshot controls both disclosure and execution for
        // the entire turn. A concurrent profile switch is observed next turn.
        let profile = self.active_profile.snapshot().await?;
        let mut prepared = prepare_corpus(&self.resources, &context).await?;
        let mut definitions = prepared
            .snapshot
            .entries
            .iter()
            .filter_map(|entry| entry.tool_definition.clone())
            .collect();
        constrain_profile_capabilities(&profile, &mut definitions, &mut prepared.risk_by_tool);
        let executor = Arc::new(TurnToolExecutor::new(
            &self.resources,
            prepared.executor,
            context.session_id.clone(),
            context.turn_count,
            context.repo_hooks_trusted,
            context.working_dir.clone(),
            context.operation_id,
            context.process_id,
        ));
        let notification_session_id = context.session_id.clone();
        let notification_turn_count = context.turn_count;
        let authority = Arc::new(
            RegistryAuthorityProvider::new(
                prepared.risk_by_tool,
                context.principal,
                context.connection_id,
                context.thread_id,
                context.turn_id,
                context.workspace,
                context.session_id,
                context.working_dir,
                context.sandbox,
                context.cancel,
            )
            .with_agent_context(context.agent)
            .with_turn_event_sender(stream_sender.clone()),
        );
        let action_loop = context.action_loop;
        let notification_observer: crate::application::tool_stream_bridge::ToolNotificationObserver = {
            let hooks = self.hooks.clone();
            let session_id = notification_session_id;
            let turn_count = notification_turn_count;
            Arc::new(move |notification| {
                let hooks = hooks.clone();
                let session_id = session_id.clone();
                Box::pin(async move {
                    hooks
                        .execute(HookContext {
                            point: fabric::hook::HookPoint::Notification,
                            session_id,
                            turn_count,
                            tool_name: None,
                            tool_input: None,
                            tool_result: None,
                            message: None,
                            metadata: HashMap::from([(
                                "notification".into(),
                                serde_json::to_string(&notification)
                                    .unwrap_or_else(|_| "null".into()),
                            )]),
                        })
                        .await;
                })
            })
        };
        let invoker = match stream_sender {
            Some(sender) => CapabilityRuntimeFactory::build_streaming(
                self.admission.clone(),
                executor,
                authority,
                action_loop,
                sender,
                Some(notification_observer),
            ),
            None => match action_loop {
                Some(action_loop) => CapabilityRuntimeFactory::build_with_action_loop(
                    self.admission.clone(),
                    executor,
                    authority,
                    action_loop,
                ),
                None => {
                    CapabilityRuntimeFactory::build(self.admission.clone(), executor, authority)
                }
            },
        };
        Ok(PreparedCapabilities {
            definitions,
            invoker,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_hooks_join_the_authoritative_corpus_registry() {
        let mut registry =
            corpus::HookRegistry::new(Arc::new(kernel::chronos::TestClock::default()));
        let config = crate::composition::config::HooksConfig {
            on_session_start: vec!["session-start".into()],
            pre_turn: vec!["pre-turn".into()],
            pre_tool: vec!["pre-tool".into()],
            post_tool: vec!["post-tool".into()],
            on_session_end: vec!["session-end".into()],
            post_tool_failure: vec!["tool-failure".into()],
            ..Default::default()
        };

        register_configured_hooks(&mut registry, &config);

        assert_eq!(registry.count(&fabric::hook::HookPoint::PreTurn), 1);
        assert_eq!(registry.count(&fabric::hook::HookPoint::OnSessionStart), 1);
        assert_eq!(registry.count(&fabric::hook::HookPoint::PreTool), 1);
        assert_eq!(registry.count(&fabric::hook::HookPoint::PostTool), 1);
        assert_eq!(registry.count(&fabric::hook::HookPoint::PostToolFailure), 1);
        assert_eq!(registry.count(&fabric::hook::HookPoint::OnSessionEnd), 1);
    }

    #[test]
    fn one_profile_snapshot_constrains_disclosure_and_executor_gate() {
        let snapshot = ResolvedTurnProfile {
            profile_name: "safe".into(),
            allowed_tools: ["file_read".to_owned()].into_iter().collect(),
            system_prompt: String::new(),
            model_policy: None,
            max_iterations: 0,
            max_input_tokens: 0,
            max_output_tokens: 0,
            max_tool_calls: 0,
            max_elapsed_ms: 0,
            approval_policy: fabric::AgentApprovalPolicy::AutoApprove,
            tool_timeout_ms: 30_000,
        };
        let mut definitions = ["file_read", "file_write"]
            .into_iter()
            .map(|name| fabric::ToolDefinition {
                name: name.into(),
                description: name.into(),
                input_schema: serde_json::json!({"type": "object"}),
            })
            .collect::<Vec<_>>();
        let mut risks = HashMap::from([
            (
                "file_read".into(),
                fabric::types::admission::RiskLevel::ReadOnly,
            ),
            (
                "file_write".into(),
                fabric::types::admission::RiskLevel::Sandboxed,
            ),
        ]);

        constrain_profile_capabilities(&snapshot, &mut definitions, &mut risks);

        assert_eq!(
            definitions
                .iter()
                .map(|definition| definition.name.as_str())
                .collect::<Vec<_>>(),
            vec!["file_read"]
        );
        assert_eq!(
            risks.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["file_read"]
        );
    }

    #[test]
    fn completed_turn_snapshot_is_immutable_across_profile_switch() {
        let mut active_allowed: std::collections::HashSet<_> =
            ["file_read".to_owned()].into_iter().collect();
        let turn_snapshot = ResolvedTurnProfile {
            profile_name: "safe".into(),
            allowed_tools: active_allowed.clone(),
            system_prompt: String::new(),
            model_policy: None,
            max_iterations: 0,
            max_input_tokens: 0,
            max_output_tokens: 0,
            max_tool_calls: 0,
            max_elapsed_ms: 0,
            approval_policy: fabric::AgentApprovalPolicy::AutoApprove,
            tool_timeout_ms: 30_000,
        };
        active_allowed.clear();
        active_allowed.insert("file_write".to_owned());

        assert!(turn_snapshot.allowed_tools.contains("file_read"));
        assert!(!turn_snapshot.allowed_tools.contains("file_write"));
        assert!(active_allowed.contains("file_write"));
    }

    #[test]
    fn resolved_turn_profile_carries_behavior_and_authorization() {
        let profile = crate::application::turn_runtime_ports::ResolvedTurnProfile {
            profile_name: "test-code-agent".into(),
            allowed_tools: ["file_read".to_owned(), "bash_exec".to_owned()]
                .into_iter()
                .collect(),
            system_prompt: "You are a code agent. Write and test code.".into(),
            model_policy: Some("gpt-5-code".into()),
            max_iterations: 20,
            max_input_tokens: 100_000,
            max_output_tokens: 16_384,
            max_tool_calls: 64,
            max_elapsed_ms: 600_000,
            approval_policy: fabric::AgentApprovalPolicy::AutoApprove,
            tool_timeout_ms: 30_000,
        };

        assert_eq!(profile.profile_name, "test-code-agent");
        assert!(profile.allowed_tools.contains("bash_exec"));
        assert!(profile.system_prompt.contains("code agent"));
        assert_eq!(profile.model_policy.as_deref(), Some("gpt-5-code"));
        assert_eq!(profile.max_iterations, 20);
        assert_eq!(profile.max_input_tokens, 100_000);
        assert_eq!(profile.max_output_tokens, 16_384);
        assert_eq!(profile.max_tool_calls, 64);
        assert_eq!(profile.max_elapsed_ms, 600_000);
        assert_eq!(
            profile.approval_policy,
            fabric::AgentApprovalPolicy::AutoApprove
        );
        assert_eq!(profile.tool_timeout_ms, 30_000);
    }

    #[test]
    fn turn_profile_is_applied_different_profiles_yield_different_configs() {
        let code = ResolvedTurnProfile {
            profile_name: "code-agent".into(),
            allowed_tools: ["bash_exec".to_owned()].into_iter().collect(),
            system_prompt: "Write production code with tests.".into(),
            model_policy: Some("gpt-5-code".into()),
            max_iterations: 20,
            max_input_tokens: 100_000,
            max_output_tokens: 16_384,
            max_tool_calls: 64,
            max_elapsed_ms: 600_000,
            approval_policy: fabric::AgentApprovalPolicy::AutoApprove,
            tool_timeout_ms: 30_000,
        };

        let review = ResolvedTurnProfile {
            profile_name: "review-agent".into(),
            allowed_tools: ["file_read".to_owned()].into_iter().collect(),
            system_prompt: "Review code for bugs and security issues.".into(),
            model_policy: Some("claude-opus-review".into()),
            max_iterations: 10,
            max_input_tokens: 200_000,
            max_output_tokens: 8_192,
            max_tool_calls: 32,
            max_elapsed_ms: 300_000,
            approval_policy: fabric::AgentApprovalPolicy::AutoDeny,
            tool_timeout_ms: 60_000,
        };

        // Different profiles must produce observably different behavior configs.
        assert_ne!(code.model_policy, review.model_policy);
        assert_ne!(code.system_prompt, review.system_prompt);
        assert_ne!(code.max_iterations, review.max_iterations);
        assert_ne!(code.max_input_tokens, review.max_input_tokens);
        assert_ne!(code.max_tool_calls, review.max_tool_calls);
        assert_ne!(code.approval_policy, review.approval_policy);
        assert_ne!(code.tool_timeout_ms, review.tool_timeout_ms);
    }
}

//! Private production adapters for turn-blocking runtime ports.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use async_trait::async_trait;
use fabric::hook::{HookContext, HookResult};
use fabric::{AdmissionController, Clock, ContentBlock, LlmProvider, Message, Role};
use tokio::sync::{mpsc, Mutex};

use crate::r#impl::daemon::handler::tool_executor::{prepare_corpus, TurnToolExecutor};
use crate::r#impl::daemon::model_router::ModelRouter;
use crate::service::governed_capability::{
    CapabilityExecutionContext, CapabilityRuntimeFactory, RegistryAuthorityProvider,
};
use crate::service::turn_runtime_ports::{
    ApprovalNotice, GovernedTurnCapabilityPort, ModelSelectionPort, PreparedCapabilities,
    SelfPolicyPort, StormStatePort, TurnApprovalPort, TurnConfigPort, TurnHookPort,
    TurnObservabilityPort, TurnRuntimePorts, TurnSessionStatePort,
};

pub(super) fn register_configured_hooks(
    registry: &mut corpus::HookRegistry,
    config: &crate::core::config::HooksConfig,
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
            fabric::hook::HookPoint::OnSessionEnd,
            &config.on_session_end,
        ),
    ] {
        for (index, script) in scripts.iter().enumerate() {
            registry.register(corpus::hook::registry::RegisteredHook {
                name: format!("config:{point:?}:{index}"),
                source: "config".into(),
                script_path: Some(
                    crate::r#impl::daemon::handler::format::expand_tilde(script).into(),
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
    pub(crate) pending_approvals: crate::service::admin_service::PendingApprovals,
    pub(crate) capabilities: crate::r#impl::daemon::handler::tool_executor::CapabilityResources,
    pub(crate) admission: Arc<dyn AdmissionController>,
    pub(crate) sessions: Arc<
        Mutex<HashMap<String, Arc<Mutex<crate::r#impl::daemon::session_manager::SessionManager>>>>,
    >,
    pub(crate) default_session_id: Arc<Mutex<String>>,
    pub(crate) session_created_at: Arc<Mutex<HashMap<String, fabric::MonoTime>>>,
    pub(crate) data_dir: std::path::PathBuf,
    pub(crate) context_window: usize,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) memory: Arc<dyn mnemosyne::MemoryService>,
    pub(crate) config: Arc<dyn TurnConfigPort>,
    pub(crate) performance: Arc<fabric::kernel::debug_bus::PerfCounter>,
}

pub(super) fn compose_turn_runtime(resources: TurnRuntimeResources) -> TurnRuntimePorts {
    let corpus = resources.corpus;
    let execute_hook: Arc<HookExecutionFn> = Arc::new(move |context| {
        let corpus = corpus.clone();
        Box::pin(async move { corpus.execute_hook(&context).await })
    });
    let hooks: Arc<dyn TurnHookPort> = Arc::new(ProductionTurnHooks { execute_hook });
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
        }),
        sessions: Arc::new(ProductionTurnSessions {
            registry: resources.sessions,
            default_id: resources.default_session_id,
            created_at: resources.session_created_at,
            data_dir: resources.data_dir,
            context_window: resources.context_window,
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
        Mutex<HashMap<String, Arc<Mutex<crate::r#impl::daemon::session_manager::SessionManager>>>>,
    >,
    default_id: Arc<Mutex<String>>,
    created_at: Arc<Mutex<HashMap<String, fabric::MonoTime>>>,
    data_dir: std::path::PathBuf,
    context_window: usize,
    clock: Arc<dyn Clock>,
    llm: Arc<dyn LlmProvider>,
    memory_service: Arc<dyn mnemosyne::MemoryService>,
    hooks: Arc<dyn TurnHookPort>,
}

impl ProductionTurnSessions {
    async fn manager(
        &self,
    ) -> anyhow::Result<(
        String,
        Arc<Mutex<crate::r#impl::daemon::session_manager::SessionManager>>,
    )> {
        let session_id = self.default_id.lock().await.clone();
        if let Some(manager) = self.registry.lock().await.get(&session_id).cloned() {
            return Ok((session_id, manager));
        }
        let manager = crate::r#impl::daemon::session_manager::SessionManager::new(
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
}

#[async_trait]
impl TurnSessionStatePort for ProductionTurnSessions {
    async fn current(&self) -> anyhow::Result<(String, usize)> {
        let (session_id, manager) = self.manager().await?;
        let turn_count = manager.lock().await.turn_count();
        Ok((session_id, turn_count))
    }

    async fn begin_user(&self, message: &str) -> anyhow::Result<(String, usize)> {
        let (session_id, manager) = self.manager().await?;
        let turn_count = {
            let mut manager = manager.lock().await;
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
        succeeded: bool,
        tool_calls: &[(String, String, serde_json::Value)],
        tool_results: &[(String, String, bool)],
        output: &str,
    ) -> anyhow::Result<usize> {
        let (_, manager) = self.manager().await?;
        let mut manager = manager.lock().await;
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
            manager.compact_if_needed(&*self.llm).await?;
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
    pending: crate::service::admin_service::PendingApprovals,
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
    resources: crate::r#impl::daemon::handler::tool_executor::CapabilityResources,
    admission: Arc<dyn AdmissionController>,
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
        let prepared = prepare_corpus(&self.resources, &context).await?;
        let definitions = prepared
            .snapshot
            .entries
            .iter()
            .filter_map(|entry| entry.tool_definition.clone())
            .collect();
        let executor = Arc::new(TurnToolExecutor::new(
            &self.resources,
            prepared.executor,
            context.session_id.clone(),
            context.turn_count,
            context.working_dir.clone(),
            context.operation_id,
            context.process_id,
        ));
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
            .with_agent_context(context.agent),
        );
        let action_loop = context.action_loop;
        let invoker = match stream_sender {
            Some(sender) => CapabilityRuntimeFactory::build_streaming(
                self.admission.clone(),
                executor,
                authority,
                action_loop,
                sender,
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
            corpus::HookRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
        let config = crate::core::config::HooksConfig {
            on_session_start: vec!["session-start".into()],
            pre_turn: vec!["pre-turn".into()],
            pre_tool: vec!["pre-tool".into()],
            post_tool: vec!["post-tool".into()],
            on_session_end: vec!["session-end".into()],
        };

        register_configured_hooks(&mut registry, &config);

        assert_eq!(registry.count(&fabric::hook::HookPoint::PreTurn), 1);
        assert_eq!(registry.count(&fabric::hook::HookPoint::OnSessionStart), 1);
        assert_eq!(registry.count(&fabric::hook::HookPoint::PreTool), 1);
        assert_eq!(registry.count(&fabric::hook::HookPoint::PostTool), 1);
        assert_eq!(registry.count(&fabric::hook::HookPoint::OnSessionEnd), 1);
    }
}

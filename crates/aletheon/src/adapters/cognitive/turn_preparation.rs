//! Concrete capability/context preparation for the Application cognitive stage.

use std::sync::Arc;

pub struct PrepareCognitiveTurn<'a> {
    pub agora: Option<&'a Arc<dyn agora::contract::AgoraService>>,
    pub kernel: &'a kernel::KernelRuntime,
    pub conscious: Option<&'a Arc<dyn crate::composition::conscious_workspace::ConsciousTurnPort>>,
    pub capabilities: &'a dyn crate::adapters::capability::GovernedTurnCapabilityPort,
    pub context_assembler: &'a application::turn::context::ContextAssembler,
    pub lifecycle: &'a crate::daemon::turn_lifecycle_adapter::TurnLifecycleAdapter,
    pub lifecycle_context: &'a crate::daemon::turn_lifecycle_adapter::TurnLifecycleContext,
    pub prefix_trackers: &'a tokio::sync::Mutex<runtime::cache_shape::PrefixShapeTrackerStore>,
    pub session_input: Arc<application::session_input::SessionInputCoordinator>,
    pub request: &'a contracts::TurnRequest,
    pub context_request: &'a contracts::TurnRequest,
    pub existing_messages: &'a [contracts::Message],
    pub history_budget: contracts::HistoryBudgetTokens,
    pub prepared_context: application::turn::context::PreparedContext,
    pub model_facts: &'a contracts::ModelRuntimeFacts,
    pub profile: &'a application::turn::settings::ResolvedTurnProfile,
    pub settings: &'a application::turn::settings::TurnRuntimeSettings,
    pub principal: &'a contracts::PrincipalId,
    pub process_id: contracts::ProcessId,
    pub operation_id: contracts::OperationId,
    pub session_id: &'a str,
    pub working_dir: &'a std::path::Path,
    pub message: &'a str,
    pub sandbox: contracts::SandboxRequirement,
    pub turn_count: usize,
    pub rewrite_version: u64,
    pub cancel: tokio_util::sync::CancellationToken,
}

pub struct PreparedCognitiveTurn {
    pub agora: Option<Arc<dyn agora::contract::AgoraService>>,
    pub agora_version: u64,
    pub agora_start_version: u64,
    pub batch_planner: Option<Arc<dyn cognit::harness::BatchPlanner>>,
    pub turn_stream: contracts::ipc::TurnEventStream,
    pub turn_sender: contracts::ipc::TurnEventSender,
    pub tool_definitions: Vec<contracts::ToolDefinition>,
    pub tool_executor: crate::adapters::capability::TurnToolExecutor,
    pub context_projection: Option<contracts::ContextProjectionReceipt>,
    pub request_messages: Vec<contracts::Message>,
    pub prefix_shape_digest: Option<String>,
    pub local_cache_miss_reason: Option<runtime::cache_shape::LocalMissReason>,
    pub provider_miss_inference_allowed: bool,
    pub diff_tracker: Arc<tokio::sync::Mutex<runtime::turn_diff_tracker::TurnDiffTracker>>,
    pub capability_receipts: Arc<tokio::sync::Mutex<Vec<contracts::CapabilityTerminalReceipt>>>,
    pub inference_items: Arc<tokio::sync::Mutex<Vec<contracts::ItemPayload>>>,
}

pub async fn prepare(input: PrepareCognitiveTurn<'_>) -> anyhow::Result<PreparedCognitiveTurn> {
    let crate::adapters::conscious::turn_workspace::TurnSpaceSeed {
        agora,
        agora_version,
        main_agent_id,
    } = crate::adapters::conscious::turn_workspace::seed_turn_space(
        input.agora,
        input.kernel,
        input.process_id,
        input.session_id,
        input.message,
    )
    .await;
    let agora_start_version = agora_version;
    let resources = crate::adapters::conscious::turn_workspace::resolve_execution_resources(
        input.conscious,
        input.session_id,
        input.process_id,
    )
    .await?;
    let (turn_stream, turn_sender) = contracts::ipc::TurnEventStream::new();
    let projected = crate::adapters::capability::prepare_turn(
        input.capabilities,
        crate::adapters::capability::TurnCapabilityInput {
            main_agent_id,
            process_id: input.process_id,
            operation_id: input.operation_id,
            principal: input.principal,
            request: input.request,
            session_id: input.session_id,
            working_dir: input.working_dir,
            sandbox: input.sandbox,
            cancel: input.cancel.clone(),
            turn_count: input.turn_count,
            action_loop: resources.action_loop,
            streaming_tools: input.settings.streaming_tools,
            turn_event_sender: turn_sender.clone(),
            profile: input.profile.clone(),
            settings: input.settings,
        },
    )
    .await?;
    let assembled = input.context_assembler.assemble_prepared(
        input.context_request,
        input.existing_messages,
        input.history_budget,
        input.prepared_context,
        &projected.definitions,
    )?;
    let prompt_profile = assembled.diagnostic_profile(projected.definitions.len());
    tracing::debug!(
        prompt_construction_profile = %prompt_profile,
        "turn prompt construction profile"
    );
    let mut request_messages = assembled.messages;
    adapters_inference::runtime_facts::bind_runtime_facts(&mut request_messages, input.model_facts);
    input
        .lifecycle
        .before_tool_batch(
            input.lifecycle_context,
            projected.definitions.len(),
            prompt_profile,
            &input.cancel,
        )
        .await?;
    let (prefix_shape_digest, local_cache_miss_reason, provider_miss_inference_allowed) =
        adapters_inference::runtime_facts::track_prefix_shape(
            input.prefix_trackers,
            &input.request.context.thread_id.0,
            input.model_facts,
            &request_messages,
            &projected.definitions,
            &input.profile.profile_name,
            input.rewrite_version,
        )
        .await;
    let diff_tracker = Arc::new(tokio::sync::Mutex::new(
        runtime::turn_diff_tracker::TurnDiffTracker::default(),
    ));
    let capability_receipts = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let inference_items = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let tool_executor = crate::adapters::capability::TurnToolExecutor {
        invoker: projected.invoker,
        activation_catalog: projected.authorized_catalog,
        diff_tracker: diff_tracker.clone(),
        session_input: input.session_input,
        principal: input.principal.clone(),
        connection: input.request.context.connection_id.clone(),
        thread: input.request.context.thread_id.clone(),
        operation_id: input.operation_id,
        process_id: input.process_id,
    };
    Ok(PreparedCognitiveTurn {
        agora,
        agora_version,
        agora_start_version,
        batch_planner: resources.batch_planner,
        turn_stream,
        turn_sender,
        tool_definitions: projected.definitions,
        tool_executor,
        context_projection: assembled.projection_receipt,
        request_messages,
        prefix_shape_digest,
        local_cache_miss_reason,
        provider_miss_inference_allowed,
        diff_tracker,
        capability_receipts,
        inference_items,
    })
}

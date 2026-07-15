use std::future::Future;
use std::sync::Arc;

use cognit::harness::event_sink::ChannelEventSink;
use cognit::harness::linear::{DynLlmRef, TurnMetrics};
use fabric::{LlmProvider, Message, ToolDefinition, TurnRequest};
use tokio_util::sync::CancellationToken;

use crate::core::config::ExecutiveConfig;

pub struct DaemonStreamingTurnContext<F> {
    pub config: ExecutiveConfig,
    pub llm: Arc<dyn LlmProvider>,
    pub tool_defs: Vec<ToolDefinition>,
    pub execute_tool: F,
    pub event_sink: ChannelEventSink,
    pub request_messages: Vec<Message>,
    pub dasein_context: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    /// Per-turn cancellation token from the OperationScope (PR-3).
    ///
    /// Checked cooperatively by the execute_tool closure before each tool call.
    /// When cancelled by `cancel_turn()`, subsequent tool invocations return
    /// immediately with an error.
    pub cancel_token: CancellationToken,
    pub clock: Arc<dyn fabric::Clock>,
}

/// Submit the daemon's streaming ReAct turn through the service/composition seam.
///
/// This keeps daemon handler code focused on JSON-RPC/session/event pumping while
/// the concrete harness construction stays in `executive::service`.
pub async fn submit_streaming_daemon_turn<F, Fut>(
    request: TurnRequest,
    context: DaemonStreamingTurnContext<F>,
) -> anyhow::Result<(String, TurnMetrics)>
where
    F: Fn(&str, &str, &serde_json::Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = (String, bool)> + Send + 'static,
{
    let DaemonStreamingTurnContext {
        config,
        llm,
        tool_defs,
        execute_tool,
        event_sink,
        request_messages,
        dasein_context,
        cancel_token,
        clock,
    } = context;

    let mut react_loop =
        crate::service::harness_factory::build_configured_react_loop(&config, clock);
    react_loop.seed_messages(request_messages);
    react_loop.set_goal(request.input.clone());
    react_loop.set_dasein_context_provider(Box::new(move || dasein_context()));
    use cognit::harness::event_sink::{Event, EventSink};
    event_sink.emit(Event::GoalSet {
        goal: request.input,
        sub_goals: vec![],
    });
    let llm_ref = DynLlmRef(&*llm);
    tokio::select! {
        _ = cancel_token.cancelled() => anyhow::bail!("turn cancelled"),
        result = react_loop.run_streaming(&llm_ref, &tool_defs, execute_tool, &event_sink) => result,
    }
}

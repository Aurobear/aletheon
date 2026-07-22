mod support {
    pub mod mock_llm_provider;
    pub mod test_aletheon_builder;
}
mod turn_request_support;

use std::sync::Arc;
use std::time::Duration;

use cognit::{CognitError, CognitErrorKind};
use executive::application::daemon_react::{
    submit_streaming_daemon_turn, DaemonStreamingTurnContext,
};
use fabric::ipc::{StreamConfig, TurnEventStream, TurnEventV1};
use fabric::{Message, OperationId, SpawnSpec, TurnRequest, TurnStop};
use support::mock_llm_provider::{MockLlmProvider, MockTurnResponse, MockTurnSequence};
use support::test_aletheon_builder::{TestAletheon, TestAletheonBuilder};
use tokio_util::sync::CancellationToken;

fn request(process_id: fabric::ProcessId, thread: &str) -> TurnRequest {
    TurnRequest {
        operation_id: OperationId::default(),
        process_id,
        context: turn_request_support::context(thread, std::env::temp_dir()),
        input: "hello daemon".into(),
        model_policy: None,
        deadline: None,
    }
}

fn execute_noop(
    _call_id: &str,
    _name: &str,
    _input: &serde_json::Value,
) -> std::future::Ready<(String, bool)> {
    std::future::ready((String::new(), false))
}

fn context(
    test: &TestAletheon,
    llm: Arc<MockLlmProvider>,
    cancel_token: CancellationToken,
    sender: fabric::ipc::TurnEventSender,
) -> DaemonStreamingTurnContext<
    fn(&str, &str, &serde_json::Value) -> std::future::Ready<(String, bool)>,
> {
    DaemonStreamingTurnContext {
        config: executive::ExecutiveConfig::default(),
        llm,
        tool_defs: vec![],
        execute_tool: execute_noop,
        event_sink: cognit::CanonicalTurnEventSink::new(sender),
        request_messages: vec![Message::user("seed")],
        dasein_context: Arc::new(|| None),
        cancel_token,
        sessions: test.cognitive_sessions.clone(),
        batch_planner: None,
        session_input: test.session_input.clone(),
        prompt_queue_enabled: false,
    }
}

fn collected_events(stream: &mut TurnEventStream) -> Vec<TurnEventV1> {
    let mut events = Vec::new();
    while let Some(result) = stream.try_recv() {
        events.push(result.expect("canonical turn event must match its schema"));
    }
    events
}

#[tokio::test]
async fn streaming_daemon_turn_completes_through_real_session_factory() {
    let test = TestAletheonBuilder::new().build().await;
    let process = test
        .kernel
        .spawn_process(SpawnSpec::default())
        .await
        .unwrap();
    let llm = Arc::new(MockLlmProvider::single_text_response("done"));
    let (mut stream, sender) = TurnEventStream::new(StreamConfig::turn_events(64));

    let result = submit_streaming_daemon_turn(
        request(process.id, "streaming-success"),
        context(&test, llm.clone(), CancellationToken::new(), sender),
    )
    .await
    .unwrap();

    assert_eq!(result.output, "done");
    assert_eq!(result.stop, TurnStop::Completed);
    assert!(result.metrics.completed_normally);
    assert_eq!(llm.call_count(), 1);
    let events = collected_events(&mut stream);
    assert!(events.iter().any(|event| matches!(
        event,
        TurnEventV1::TurnDone { result: Some(text) } if text == "done"
    )));
}

#[tokio::test]
async fn streaming_daemon_turn_cancels_while_provider_is_running() {
    let test = TestAletheonBuilder::new().build().await;
    let process = test
        .kernel
        .spawn_process(SpawnSpec::default())
        .await
        .unwrap();
    let llm = Arc::new(MockLlmProvider::new(vec![MockTurnSequence {
        responses: vec![MockTurnResponse::Timeout],
    }]));
    let cancel = CancellationToken::new();
    let (mut stream, sender) = TurnEventStream::new(StreamConfig::turn_events(64));
    let task = tokio::spawn(submit_streaming_daemon_turn(
        request(process.id, "streaming-cancel"),
        context(&test, llm.clone(), cancel.clone(), sender),
    ));

    tokio::time::timeout(Duration::from_secs(1), async {
        while llm.call_count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider call should start");
    cancel.cancel();

    let error = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("cancelled turn should settle")
        .expect("turn task should not panic")
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<CognitError>().map(CognitError::kind),
        Some(CognitErrorKind::Cancelled)
    );
    assert!(!collected_events(&mut stream)
        .iter()
        .any(|event| matches!(event, TurnEventV1::TurnDone { .. })));
}

#[tokio::test]
async fn streaming_daemon_turn_propagates_provider_error_without_done_event() {
    let test = TestAletheonBuilder::new().build().await;
    let process = test
        .kernel
        .spawn_process(SpawnSpec::default())
        .await
        .unwrap();
    let llm = Arc::new(MockLlmProvider::always_error("boom"));
    let (mut stream, sender) = TurnEventStream::new(StreamConfig::turn_events(64));

    let error = submit_streaming_daemon_turn(
        request(process.id, "streaming-error"),
        context(&test, llm, CancellationToken::new(), sender),
    )
    .await
    .unwrap_err();

    assert_eq!(
        error.downcast_ref::<CognitError>().map(CognitError::kind),
        Some(CognitErrorKind::TerminalRuntime)
    );
    assert!(!collected_events(&mut stream)
        .iter()
        .any(|event| matches!(event, TurnEventV1::TurnDone { .. })));
}

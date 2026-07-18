//! Tool stream bridge — connects G2 ToolEventSink producers to
//! `TurnEventV1::ToolProgress` consumers without bypassing terminal governance.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use fabric::ipc::stream::{TurnEventSender, TurnEventV1};
use fabric::types::tool::ToolResult;
use fabric::types::tool_stream::{
    tool_event_channel, BoundToolEventReceiver, ToolEventSink, ToolExecutionError,
    ToolExecutionEvent, ToolProgress,
};
use tokio_util::sync::CancellationToken;

pub type ToolNotificationObserver = std::sync::Arc<
    dyn Fn(
            fabric::ToolNotification,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

/// Maximum coalesced text payload emitted at once.
pub const TOOL_PROGRESS_TEXT_BYTES: usize = 4 * 1024;
pub const TOOL_PROGRESS_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
/// Per-call protection against flooding the downstream turn stream.
pub const TOOL_PROGRESS_EVENT_LIMIT: usize = 64;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolStreamMetricSnapshot {
    pub tool_progress_dropped_total: u64,
    pub tool_no_terminal_total: u64,
}

static TOOL_STREAM_METRICS: LazyLock<Mutex<HashMap<String, ToolStreamMetricSnapshot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn tool_stream_metrics(tool_name: &str) -> ToolStreamMetricSnapshot {
    TOOL_STREAM_METRICS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(tool_name)
        .copied()
        .unwrap_or_default()
}

fn record_metrics(tool_name: &str, dropped: usize, no_terminal: bool) {
    let mut metrics = TOOL_STREAM_METRICS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let metric = metrics.entry(tool_name.to_owned()).or_default();
    metric.tool_progress_dropped_total = metric
        .tool_progress_dropped_total
        .saturating_add(dropped as u64);
    if no_terminal {
        metric.tool_no_terminal_total = metric.tool_no_terminal_total.saturating_add(1);
    }
}

/// The tool-side handle given to streaming tool implementations.
pub struct ToolStreamHandle {
    pub sink: ToolEventSink,
    pub event_rx: tokio::sync::mpsc::Receiver<ToolExecutionEvent>,
}

impl ToolStreamHandle {
    pub fn new() -> Self {
        let (sink, rx) = tool_event_channel();
        Self { sink, event_rx: rx }
    }
}

impl Default for ToolStreamHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// Non-authoritative bridge result. The terminal value must still pass through
/// the existing Executive settle/audit path before it becomes a ToolResult.
#[derive(Debug)]
pub struct ToolStreamOutcome {
    pub terminal: Result<ToolResult, ToolExecutionError>,
    pub progress_emitted: usize,
    pub progress_dropped: usize,
}

struct ProgressAccumulator {
    text: String,
    emitted: usize,
    dropped: usize,
}

impl ProgressAccumulator {
    fn new() -> Self {
        Self {
            text: String::new(),
            emitted: 0,
            dropped: 0,
        }
    }

    fn accept(
        &mut self,
        progress: ToolProgress,
        turn_events: &TurnEventSender,
        tool_name: &str,
        call_id: &str,
    ) {
        match progress {
            ToolProgress::Text(chunk) => {
                self.text.push_str(&chunk);
                while self.text.len() >= TOOL_PROGRESS_TEXT_BYTES {
                    let split = floor_char_boundary(&self.text, TOOL_PROGRESS_TEXT_BYTES);
                    let remainder = self.text.split_off(split);
                    let payload = std::mem::replace(&mut self.text, remainder);
                    self.emit(
                        "text",
                        serde_json::Value::String(payload),
                        turn_events,
                        tool_name,
                        call_id,
                    );
                }
            }
            other => {
                self.flush_text(turn_events, tool_name, call_id);
                self.emit(
                    other.kind(),
                    other.to_payload(),
                    turn_events,
                    tool_name,
                    call_id,
                );
            }
        }
    }

    fn flush_text(&mut self, turn_events: &TurnEventSender, tool_name: &str, call_id: &str) {
        if self.text.is_empty() {
            return;
        }
        let payload = serde_json::Value::String(std::mem::take(&mut self.text));
        self.emit("text", payload, turn_events, tool_name, call_id);
    }

    fn has_pending_text(&self) -> bool {
        !self.text.is_empty()
    }

    fn emit(
        &mut self,
        kind: &str,
        payload: serde_json::Value,
        turn_events: &TurnEventSender,
        tool_name: &str,
        call_id: &str,
    ) {
        if self.emitted >= TOOL_PROGRESS_EVENT_LIMIT {
            self.dropped += 1;
            return;
        }
        let event = TurnEventV1::ToolProgress {
            name: tool_name.to_owned(),
            call_id: call_id.to_owned(),
            kind: kind.to_owned(),
            payload,
        };
        match turn_events.send(&event) {
            Ok(()) => self.emitted += 1,
            Err(_) => self.dropped += 1,
        }
    }
}

fn floor_char_boundary(value: &str, requested: usize) -> usize {
    let mut split = requested.min(value.len());
    while !value.is_char_boundary(split) {
        split -= 1;
    }
    split
}

/// Drain one governed tool stream, forwarding only progress and returning the
/// unique terminal for settlement. Channel close synthesizes `NoTerminal`;
/// cancellation synthesizes `Cancelled` and prevents a later success terminal.
pub async fn bridge_tool_stream(
    event_rx: tokio::sync::mpsc::Receiver<ToolExecutionEvent>,
    turn_events: TurnEventSender,
    tool_name: String,
    call_id: String,
    cancel: CancellationToken,
) -> ToolStreamOutcome {
    bridge_tool_stream_observed(event_rx, turn_events, tool_name, call_id, cancel, None).await
}

async fn bridge_tool_stream_observed(
    mut event_rx: tokio::sync::mpsc::Receiver<ToolExecutionEvent>,
    turn_events: TurnEventSender,
    tool_name: String,
    call_id: String,
    cancel: CancellationToken,
    notification_observer: Option<ToolNotificationObserver>,
) -> ToolStreamOutcome {
    let mut accumulator = ProgressAccumulator::new();
    let mut flush_deadline = None;
    let terminal = loop {
        let flush_timer = async {
            match flush_deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                break Err(ToolExecutionError::Cancelled("turn cancelled".into()));
            }
            _ = flush_timer => {
                accumulator.flush_text(&turn_events, &tool_name, &call_id);
                flush_deadline = None;
            }
            event = event_rx.recv() => match event {
                Some(ToolExecutionEvent::Progress(progress)) => {
                    accumulator.accept(progress, &turn_events, &tool_name, &call_id);
                    if accumulator.has_pending_text() {
                        flush_deadline.get_or_insert_with(|| {
                            tokio::time::Instant::now() + TOOL_PROGRESS_FLUSH_INTERVAL
                        });
                    } else {
                        flush_deadline = None;
                    }
                }
                Some(ToolExecutionEvent::Notification(notification)) => {
                    // Notifications remain UI-only and are not model or terminal data.
                    if let Some(observer) = &notification_observer {
                        observer(notification).await;
                    }
                }
                Some(ToolExecutionEvent::Terminal(result)) => break result,
                None => break Err(ToolExecutionError::NoTerminal),
            }
        }
    };
    accumulator.flush_text(&turn_events, &tool_name, &call_id);
    record_metrics(
        &tool_name,
        accumulator.dropped,
        matches!(&terminal, Err(ToolExecutionError::NoTerminal)),
    );
    ToolStreamOutcome {
        terminal,
        progress_emitted: accumulator.emitted,
        progress_dropped: accumulator.dropped,
    }
}

/// Bridge a host-bound stream and reject accidental routing under another call
/// identifier. Producers cannot choose this id; it is minted at channel setup.
pub async fn bridge_bound_tool_stream(
    event_rx: BoundToolEventReceiver,
    turn_events: TurnEventSender,
    tool_name: String,
    call_id: String,
    cancel: CancellationToken,
) -> ToolStreamOutcome {
    bridge_bound_tool_stream_observed(event_rx, turn_events, tool_name, call_id, cancel, None).await
}

pub async fn bridge_bound_tool_stream_observed(
    event_rx: BoundToolEventReceiver,
    turn_events: TurnEventSender,
    tool_name: String,
    call_id: String,
    cancel: CancellationToken,
    notification_observer: Option<ToolNotificationObserver>,
) -> ToolStreamOutcome {
    let (bound_call_id, event_rx) = event_rx.into_parts();
    if bound_call_id != call_id {
        tracing::warn!(
            call_id,
            bound_call_id,
            "tool stream call_id mismatch; failing closed"
        );
        return ToolStreamOutcome {
            terminal: Err(ToolExecutionError::Protocol(
                "tool stream call_id mismatch".into(),
            )),
            progress_emitted: 0,
            progress_dropped: 0,
        };
    }
    bridge_tool_stream_observed(
        event_rx,
        turn_events,
        tool_name,
        call_id,
        cancel,
        notification_observer,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::ipc::stream::{OverflowPolicy, StreamConfig, TurnEventStream};
    use fabric::types::tool::{ToolResult, ToolResultMeta};
    use fabric::types::tool_stream::{ToolNotification, ToolNotificationKind};
    use proptest::prelude::*;

    fn turn_stream(capacity: usize) -> (TurnEventStream, TurnEventSender) {
        TurnEventStream::new(StreamConfig {
            capacity,
            overflow: OverflowPolicy::BlockProducer,
        })
    }

    fn result() -> ToolResult {
        ToolResult {
            content: "done".into(),
            is_error: false,
            metadata: ToolResultMeta::default(),
        }
    }

    #[tokio::test]
    async fn coalesces_progress_and_preserves_terminal() {
        let (mut sink, rx) = tool_event_channel();
        for _ in 0..10 {
            assert!(sink.progress(ToolProgress::Text("chunk".into())));
        }
        sink.terminal(Ok(result())).await;
        drop(sink);
        let (mut stream, sender) = turn_stream(8);

        let outcome = bridge_tool_stream(
            rx,
            sender,
            "bash".into(),
            "call-1".into(),
            CancellationToken::new(),
        )
        .await;

        assert!(outcome.terminal.is_ok());
        assert_eq!(outcome.progress_emitted, 1);
        assert!(matches!(
            stream.try_recv(),
            Some(Ok(TurnEventV1::ToolProgress { .. }))
        ));
    }

    #[tokio::test]
    async fn pending_text_flushes_before_terminal_after_time_window() {
        let (mut sink, rx) = tool_event_channel();
        assert!(sink.progress(ToolProgress::Text("still-running".into())));
        let (mut stream, sender) = turn_stream(8);
        let bridge = tokio::spawn(bridge_tool_stream(
            rx,
            sender,
            "slow-tool".into(),
            "call-window".into(),
            CancellationToken::new(),
        ));

        let event = tokio::time::timeout(std::time::Duration::from_millis(300), stream.recv())
            .await
            .expect("progress must flush while tool is still running")
            .unwrap();
        assert!(matches!(
            event,
            TurnEventV1::ToolProgress { payload, .. }
                if payload == serde_json::json!("still-running")
        ));

        sink.terminal(Ok(result())).await;
        assert!(bridge.await.unwrap().terminal.is_ok());
    }

    #[tokio::test]
    async fn progress_flood_is_bounded_and_terminal_survives() {
        let (mut sink, rx) = tool_event_channel();
        let producer = tokio::spawn(async move {
            for index in 0..1_000 {
                let _ = sink.progress(ToolProgress::Structured(serde_json::json!({"i": index})));
                tokio::task::yield_now().await;
            }
            sink.terminal(Ok(result())).await;
        });
        let (_stream, sender) = turn_stream(128);
        let outcome = bridge_tool_stream(
            rx,
            sender,
            "search".into(),
            "call-2".into(),
            CancellationToken::new(),
        )
        .await;
        producer.await.unwrap();

        assert!(outcome.terminal.is_ok());
        assert!(outcome.progress_emitted <= TOOL_PROGRESS_EVENT_LIMIT);
        assert!(outcome.progress_dropped > 0);
    }

    #[tokio::test]
    async fn producer_drop_synthesizes_no_terminal() {
        let (sink, rx) = tool_event_channel();
        drop(sink);
        let (_stream, sender) = turn_stream(1);
        let before = tool_stream_metrics("metric-no-terminal");
        let outcome = bridge_tool_stream(
            rx,
            sender,
            "metric-no-terminal".into(),
            "call-3".into(),
            CancellationToken::new(),
        )
        .await;
        assert!(matches!(
            outcome.terminal,
            Err(ToolExecutionError::NoTerminal)
        ));
        let after = tool_stream_metrics("metric-no-terminal");
        assert_eq!(
            after.tool_no_terminal_total,
            before.tool_no_terminal_total + 1
        );
    }

    #[tokio::test]
    async fn cancellation_wins_over_later_success_terminal() {
        let (mut sink, rx) = tool_event_channel();
        let (_stream, sender) = turn_stream(1);
        let cancel = CancellationToken::new();
        cancel.cancel();
        sink.terminal(Ok(result())).await;
        let outcome = bridge_tool_stream(rx, sender, "tool".into(), "call-4".into(), cancel).await;
        assert!(matches!(
            outcome.terminal,
            Err(ToolExecutionError::Cancelled(_))
        ));
    }

    #[tokio::test]
    async fn utf8_text_chunks_split_only_on_character_boundaries() {
        let (mut sink, rx) = tool_event_channel();
        assert!(sink.progress(ToolProgress::Text("界".repeat(2_000))));
        sink.terminal(Ok(result())).await;
        let (_stream, sender) = turn_stream(8);
        let outcome = bridge_tool_stream(
            rx,
            sender,
            "tool".into(),
            "call-5".into(),
            CancellationToken::new(),
        )
        .await;
        assert!(outcome.terminal.is_ok());
        assert_eq!(outcome.progress_emitted, 2);
    }

    #[tokio::test]
    async fn governed_notification_reaches_lifecycle_observer_once() {
        let (mut sink, rx) = fabric::tool_event_channel_for_call("call-notify");
        assert!(sink.notify(ToolNotification {
            kind: ToolNotificationKind::UiStatus,
            message: "working".into(),
        }));
        sink.terminal(Ok(result())).await;
        let (_stream, sender) = turn_stream(1);
        let observed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observer: ToolNotificationObserver = {
            let observed = observed.clone();
            std::sync::Arc::new(move |_| {
                let observed = observed.clone();
                Box::pin(async move {
                    observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                })
            })
        };
        let outcome = bridge_bound_tool_stream_observed(
            rx,
            sender,
            "tool".into(),
            "call-notify".into(),
            CancellationToken::new(),
            Some(observer),
        )
        .await;
        assert!(outcome.terminal.is_ok());
        assert_eq!(observed.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn bound_stream_call_id_mismatch_fails_closed() {
        let (mut sink, rx) = fabric::tool_event_channel_for_call("call-a");
        sink.terminal(Ok(result())).await;
        let (_stream, sender) = turn_stream(1);

        let outcome = bridge_bound_tool_stream(
            rx,
            sender,
            "tool".into(),
            "call-b".into(),
            CancellationToken::new(),
        )
        .await;

        assert!(matches!(
            outcome.terminal,
            Err(ToolExecutionError::Protocol(message)) if message.contains("call_id mismatch")
        ));
    }

    proptest! {
        #[test]
        fn arbitrary_valid_event_sequences_keep_the_terminal_unique_and_last(
            prefix in prop::collection::vec(any::<bool>(), 0..24),
            terminal_is_ok in any::<bool>(),
        ) {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .expect("test runtime");
            runtime.block_on(async move {
                let (mut sink, rx) = tool_event_channel();
                for (index, notification) in prefix.into_iter().enumerate() {
                    if notification {
                        let _ = sink.notify(ToolNotification {
                            kind: ToolNotificationKind::UiStatus,
                            message: format!("status-{index}"),
                        });
                    } else {
                        let _ = sink.progress(ToolProgress::Structured(
                            serde_json::json!({"index": index}),
                        ));
                    }
                }
                let terminal = if terminal_is_ok {
                    Ok(result())
                } else {
                    Err(ToolExecutionError::Failed("generated failure".into()))
                };
                sink.terminal(terminal).await;
                drop(sink);

                let (mut stream, sender) = turn_stream(32);
                let outcome = bridge_tool_stream(
                    rx,
                    sender,
                    "property-tool".into(),
                    "property-call".into(),
                    CancellationToken::new(),
                )
                .await;

                if terminal_is_ok {
                    prop_assert!(matches!(outcome.terminal, Ok(result) if result.content == "done"));
                } else {
                    prop_assert!(matches!(
                        outcome.terminal,
                        Err(ToolExecutionError::Failed(message)) if message == "generated failure"
                    ));
                }
                while let Some(event) = stream.try_recv() {
                    prop_assert!(
                        matches!(event, Ok(TurnEventV1::ToolProgress { .. })),
                        "bridge emitted a non-progress turn event"
                    );
                }
                Ok(())
            })?;
        }
    }
}

//! Debug hook for the communication bus.
//!
//! Observes EventBus.publish() and forwards events to debug sinks,
//! records to bag files, and tracks performance metrics.
//!
//! Design: `docs/plans/2026-06-19-aletheon-debug-system-design.md` (Layer 2).

use crate::kernel::debug::{DebugEvent, DebugLevel, DebugSink, Tracepoint};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, Mutex};

// ---------------------------------------------------------------------------
// EventFilter
// ---------------------------------------------------------------------------

/// Event filter for debug hooks.
///
/// When a field is `None`, that dimension is unconstrained.
#[derive(Debug, Clone)]
pub struct EventFilter {
    pub min_level: DebugLevel,
    pub modules: Option<HashSet<String>>,
    pub tracepoints: Option<HashSet<String>>,
}

impl Default for EventFilter {
    fn default() -> Self {
        Self {
            min_level: DebugLevel::Off,
            modules: None,
            tracepoints: None,
        }
    }
}

impl EventFilter {
    pub fn matches(&self, event: &DebugEvent) -> bool {
        if event.level < self.min_level {
            return false;
        }
        if let Some(ref mods) = self.modules {
            if !mods.contains(&event.module) {
                return false;
            }
        }
        if let Some(ref tps) = self.tracepoints {
            if !tps.contains(&event.tracepoint) {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// EventRecorder (rosbag equivalent)
// ---------------------------------------------------------------------------

/// Records events to a bag file for later replay.
pub struct EventRecorder {
    path: PathBuf,
    buffer: VecDeque<DebugEvent>,
    _max_buffer: usize,
    event_count: u64,
    started_at: std::time::Instant,
}

impl EventRecorder {
    pub fn new(path: PathBuf, max_buffer: usize) -> Self {
        Self {
            path,
            buffer: VecDeque::with_capacity(max_buffer),
            _max_buffer: max_buffer,
            event_count: 0,
            started_at: std::time::Instant::now(),
        }
    }

    pub fn record(&mut self, event: DebugEvent) {
        self.buffer.push_back(event);
        self.event_count += 1;
        // Actual file I/O is done in flush().
    }

    pub fn event_count(&self) -> u64 {
        self.event_count
    }

    pub fn duration(&self) -> std::time::Duration {
        self.started_at.elapsed()
    }

    /// Flush buffered events to disk. Call periodically or on stop.
    pub async fn flush(&mut self) -> anyhow::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        while let Some(event) = self.buffer.pop_front() {
            let line = serde_json::to_string(&event)?;
            file.write_all(line.as_bytes()).await?;
            file.write_all(b"\n").await?;
        }
        file.flush().await?;
        Ok(())
    }

    /// Stop recording and flush remaining events.
    pub async fn stop(mut self) -> anyhow::Result<RecordingMeta> {
        self.flush().await?;
        Ok(RecordingMeta {
            path: self.path,
            event_count: self.event_count,
            duration: self.started_at.elapsed(),
        })
    }
}

/// Metadata about a completed recording.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingMeta {
    pub path: PathBuf,
    pub event_count: u64,
    pub duration: std::time::Duration,
}

// ---------------------------------------------------------------------------
// PerfCounter
// ---------------------------------------------------------------------------

/// Performance counter for token usage, latency, throughput.
#[derive(Debug, Default)]
pub struct PerfCounter {
    pub tokens_in: AtomicU64,
    pub tokens_out: AtomicU64,
    pub turn_count: AtomicU64,
    pub error_count: AtomicU64,
    pub tool_calls: Mutex<HashMap<String, u64>>,
}

impl PerfCounter {
    pub fn record_turn(&self, tokens_in: u64, tokens_out: u64) {
        self.tokens_in.fetch_add(tokens_in, Ordering::Relaxed);
        self.tokens_out.fetch_add(tokens_out, Ordering::Relaxed);
        self.turn_count.fetch_add(1, Ordering::Relaxed);
    }

    pub async fn record_tool_call(&self, tool: &str) {
        let mut map = self.tool_calls.lock().await;
        *map.entry(tool.to_string()).or_insert(0) += 1;
    }

    pub fn record_error(&self) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> PerfSnapshot {
        PerfSnapshot {
            tokens_in: self.tokens_in.load(Ordering::Relaxed),
            tokens_out: self.tokens_out.load(Ordering::Relaxed),
            turn_count: self.turn_count.load(Ordering::Relaxed),
            error_count: self.error_count.load(Ordering::Relaxed),
        }
    }
}

/// Snapshot of performance metrics (serializable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfSnapshot {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub turn_count: u64,
    pub error_count: u64,
}

// ---------------------------------------------------------------------------
// RecorderSink — DebugSink that records events to a bag file
// ---------------------------------------------------------------------------

/// A DebugSink that buffers events into an EventRecorder for bag-file recording.
pub struct RecorderSink {
    recorder: Mutex<EventRecorder>,
}

impl RecorderSink {
    pub fn new(recorder: EventRecorder) -> Self {
        Self {
            recorder: Mutex::new(recorder),
        }
    }

    /// Consume the sink and stop the recorder, returning metadata.
    pub async fn into_recorder_stop(self) -> anyhow::Result<RecordingMeta> {
        let rec = self.recorder.into_inner();
        rec.stop().await
    }
}

#[async_trait]
impl DebugSink for RecorderSink {
    async fn emit(&self, event: DebugEvent) {
        self.recorder.lock().await.record(event);
    }

    fn should_trace(&self, _tp: &Tracepoint) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// SubscriberSink — forwards DebugEvents to an mpsc channel
// ---------------------------------------------------------------------------

/// A DebugSink that forwards events to an mpsc channel for a connected client.
pub struct SubscriberSink {
    id: String,
    tx: mpsc::Sender<DebugEvent>,
    filter: EventFilter,
}

impl SubscriberSink {
    pub fn new(id: String, tx: mpsc::Sender<DebugEvent>, filter: EventFilter) -> Self {
        Self { id, tx, filter }
    }
}

#[async_trait]
impl DebugSink for SubscriberSink {
    async fn emit(&self, event: DebugEvent) {
        // Best-effort: drop event if the channel is full or closed.
        let _ = self.tx.try_send(event);
    }

    fn should_trace(&self, _tp: &Tracepoint) -> bool {
        true
    }

    fn sink_id(&self) -> &str {
        &self.id
    }

    fn sink_filter(&self) -> Option<&EventFilter> {
        Some(&self.filter)
    }
}

// ---------------------------------------------------------------------------
// DebugBusHook
// ---------------------------------------------------------------------------

/// Debug hook attached to CommunicationBus.
///
/// Observes every published event, forwards matching ones to registered
/// sinks, optionally records to a bag file, and tracks performance metrics.
pub struct DebugBusHook {
    sinks: Vec<Arc<dyn DebugSink>>,
    filter: EventFilter,
    recorder: Option<EventRecorder>,
    perf: Arc<PerfCounter>,
}

impl DebugBusHook {
    pub fn new(filter: EventFilter) -> Self {
        Self {
            sinks: Vec::new(),
            filter,
            recorder: None,
            perf: Arc::new(PerfCounter::default()),
        }
    }

    /// Add a debug sink to receive matching events.
    pub fn with_sink(mut self, sink: Arc<dyn DebugSink>) -> Self {
        self.sinks.push(sink);
        self
    }

    /// Attach an event recorder for bag-file recording.
    pub fn with_recorder(mut self, recorder: EventRecorder) -> Self {
        self.recorder = Some(recorder);
        self
    }

    /// Get a handle to the performance counter.
    pub fn perf_counter(&self) -> Arc<PerfCounter> {
        self.perf.clone()
    }

    /// Add a debug sink at runtime (non-builder).
    /// Returns a sink ID that can be used to remove it later.
    pub fn add_sink(&mut self, sink: Arc<dyn DebugSink>) -> usize {
        let id = self.sinks.len();
        self.sinks.push(sink);
        id
    }

    /// Remove a debug sink by index (returned from add_sink).
    /// If the index is out of range, this is a no-op.
    pub fn remove_sink(&mut self, index: usize) {
        if index < self.sinks.len() {
            self.sinks.remove(index);
        }
    }

    /// Replace the event filter (used by debug.trace_start/stop).
    pub fn set_filter(&mut self, filter: EventFilter) {
        self.filter = filter;
    }

    /// Get a reference to the current global event filter.
    pub fn current_filter(&self) -> &EventFilter {
        &self.filter
    }

    /// Remove a sink by its sink_id (used for subscriber unsubscription).
    pub fn remove_sink_by_id(&mut self, id: &str) {
        self.sinks.retain(|s| s.sink_id() != id);
    }

    /// Remove all subscriber sinks (those that have a per-sink filter).
    /// Keeps non-subscriber sinks (e.g. recorder sinks).
    pub fn clear_subscriber_sinks(&mut self) {
        self.sinks.retain(|s| s.sink_filter().is_none());
    }

    /// Called on every EventBus.publish().
    ///
    /// Forwards matching events to sinks and records to bag.
    pub async fn on_event(&mut self, event: &DebugEvent) {
        // Forward to sinks with per-sink filtering
        for sink in &self.sinks {
            let passes = match sink.sink_filter() {
                Some(f) => f.matches(event),
                None => self.filter.matches(event), // fallback to global
            };
            if passes {
                sink.emit(event.clone()).await;
            }
        }

        // Record to bag (uses global filter)
        if self.filter.matches(event) {
            if let Some(ref mut rec) = self.recorder {
                rec.record(event.clone());
            }
        }
    }

    /// Stop recording and return metadata.
    pub async fn stop_recording(&mut self) -> anyhow::Result<Option<RecordingMeta>> {
        if let Some(recorder) = self.recorder.take() {
            Ok(Some(recorder.stop().await?))
        } else {
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;

    fn make_event(level: DebugLevel, module: &str, tracepoint: &str) -> DebugEvent {
        DebugEvent {
            ts: 1000,
            tracepoint: tracepoint.to_string(),
            module: module.to_string(),
            level,
            data: json!(null),
            session_id: None,
            agent_id: None,
        }
    }

    // -- EventFilter --------------------------------------------------------

    #[test]
    fn filter_default_accepts_all() {
        let filter = EventFilter::default();
        let event = make_event(DebugLevel::Trace, "mod", "tp");
        assert!(filter.matches(&event));
    }

    #[test]
    fn filter_min_level_rejects_lower() {
        let filter = EventFilter {
            min_level: DebugLevel::Info,
            ..Default::default()
        };
        // Debug(4) >= Info(3) → passes
        assert!(filter.matches(&make_event(DebugLevel::Debug, "m", "tp")));
        // Info(3) >= Info(3) → passes
        assert!(filter.matches(&make_event(DebugLevel::Info, "m", "tp")));
        // Error(1) < Info(3) → rejected
        assert!(!filter.matches(&make_event(DebugLevel::Error, "m", "tp")));
        // Warn(2) < Info(3) → rejected
        assert!(!filter.matches(&make_event(DebugLevel::Warn, "m", "tp")));
    }

    #[test]
    fn filter_modules_whitelist() {
        let filter = EventFilter {
            modules: Some(HashSet::from(["runtime".into()])),
            ..Default::default()
        };
        assert!(filter.matches(&make_event(DebugLevel::Info, "runtime", "tp")));
        assert!(!filter.matches(&make_event(DebugLevel::Info, "body", "tp")));
    }

    #[test]
    fn filter_tracepoints_whitelist() {
        let filter = EventFilter {
            tracepoints: Some(HashSet::from(["react_loop.iteration".into()])),
            ..Default::default()
        };
        assert!(filter.matches(&make_event(DebugLevel::Info, "m", "react_loop.iteration")));
        assert!(!filter.matches(&make_event(DebugLevel::Info, "m", "other")));
    }

    // -- EventRecorder ------------------------------------------------------

    #[tokio::test]
    async fn recorder_flush_and_stop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bag");

        let mut recorder = EventRecorder::new(path.clone(), 100);
        for i in 0..5 {
            recorder.record(make_event(DebugLevel::Info, "m", &format!("tp_{i}")));
        }
        assert_eq!(recorder.event_count(), 5);

        let meta = recorder.stop().await.unwrap();
        assert_eq!(meta.event_count, 5);
        assert!(meta.path.exists());

        // Verify file contents are valid NDJSON
        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = contents.trim().lines().collect();
        assert_eq!(lines.len(), 5);
        for line in &lines {
            let _: DebugEvent = serde_json::from_str(line).unwrap();
        }
    }

    // -- PerfCounter --------------------------------------------------------

    #[tokio::test]
    async fn perf_counter_snapshots() {
        let perf = PerfCounter::default();
        perf.record_turn(100, 50);
        perf.record_turn(200, 80);
        perf.record_error();

        let snap = perf.snapshot();
        assert_eq!(snap.tokens_in, 300);
        assert_eq!(snap.tokens_out, 130);
        assert_eq!(snap.turn_count, 2);
        assert_eq!(snap.error_count, 1);
    }

    #[tokio::test]
    async fn perf_counter_tool_calls() {
        let perf = PerfCounter::default();
        perf.record_tool_call("read_file").await;
        perf.record_tool_call("bash").await;
        perf.record_tool_call("read_file").await;

        let map = perf.tool_calls.lock().await;
        assert_eq!(map.get("read_file"), Some(&2));
        assert_eq!(map.get("bash"), Some(&1));
    }

    // -- DebugBusHook -------------------------------------------------------

    /// Minimal sink for testing.
    struct CollectSink {
        events: Arc<Mutex<Vec<DebugEvent>>>,
    }

    #[async_trait]
    impl DebugSink for CollectSink {
        async fn emit(&self, event: DebugEvent) {
            self.events.lock().await.push(event);
        }
        fn should_trace(&self, _tp: &crate::kernel::debug::Tracepoint) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn hook_forwards_matching_events() {
        let collected = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::new(CollectSink {
            events: collected.clone(),
        });

        let filter = EventFilter {
            min_level: DebugLevel::Info,
            ..Default::default()
        };
        let mut hook = DebugBusHook::new(filter).with_sink(sink);

        // Should pass (Info >= Info)
        hook.on_event(&make_event(DebugLevel::Info, "m", "tp")).await;
        // Should be filtered (Debug < Info)
        hook.on_event(&make_event(DebugLevel::Debug, "m", "tp")).await;
        // Should pass
        hook.on_event(&make_event(DebugLevel::Error, "m", "tp")).await;

        let events = collected.lock().await;
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn hook_records_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hook_test.bag");

        let recorder = EventRecorder::new(path.clone(), 100);
        let mut hook = DebugBusHook::new(EventFilter::default()).with_recorder(recorder);

        hook.on_event(&make_event(DebugLevel::Info, "m", "tp")).await;
        hook.on_event(&make_event(DebugLevel::Debug, "m", "tp")).await;

        let meta = hook.stop_recording().await.unwrap().unwrap();
        assert_eq!(meta.event_count, 2);
    }

    #[tokio::test]
    async fn hook_stop_recording_none_when_no_recorder() {
        let mut hook = DebugBusHook::new(EventFilter::default());
        assert!(hook.stop_recording().await.unwrap().is_none());
    }
}

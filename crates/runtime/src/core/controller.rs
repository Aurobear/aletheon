//! Transport-agnostic controller.
//!
//! The Controller sits behind every frontend (TUI, daemon, HTTP, desktop).
//! All frontends issue the same commands and observe the same event stream.
//!
//! **Scaffold module** — this is a lightweight multi-frontend abstraction that
//! duplicates some state from `ReActLoop` and handler logic. It will be wired
//! into TUI and HTTP frontends in a future phase. Until then, dead-code
//! warnings are suppressed with `#[allow(dead_code)]` on intentionally unused
//! fields.

use super::config::RuntimeConfig;
use super::event_sink::{Event, EventSink};
use super::react_loop::{ReActLoop, PLAN_MODE_MARKER};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// Options for constructing a Controller.
#[derive(Debug, Clone)]
pub struct ControllerOptions {
    pub working_dir: String,
    pub data_dir: String,
    pub system_prompt: String,
    pub max_iterations: usize,
    pub compaction_enabled: bool,
}

impl Default for ControllerOptions {
    fn default() -> Self {
        Self {
            working_dir: "/tmp".into(),
            data_dir: "/tmp/aletheon".into(),
            system_prompt: "You are a helpful assistant.".into(),
            max_iterations: 0,
            compaction_enabled: true,
        }
    }
}

/// Transport-agnostic agent controller.
///
/// Parked — future multi-frontend (TUI/HTTP).
/// See docs/plans/2026-07-04-config-cleanup-refactor-design.md §5.
///
/// This is a scaffold for multi-frontend support (TUI, HTTP, desktop).
/// It duplicates some state from `ReActLoop` and handler.rs so that each
/// frontend can hold a shared `Controller` without coupling to internals.
/// It will be wired into real frontends in a future phase.
#[allow(dead_code)]
pub struct Controller {
    /// The ReAct loop (holds conversation state).
    react_loop: Arc<Mutex<ReActLoop>>,
    /// Event sink for lifecycle events.
    event_sink: Arc<dyn EventSink>,
    /// Whether a turn is currently running.
    running: Arc<Mutex<bool>>,
    /// Cancellation token for the current turn.
    cancel_token: CancellationToken,
    /// Working directory.
    working_dir: String,
    /// System prompt (immutable after construction).
    system_prompt: String,
    /// Pending memory updates (drain into user message).
    memory_queue: Arc<Mutex<Vec<String>>>,
    /// Plan mode flag.
    plan_mode: Arc<Mutex<bool>>,
}

impl Controller {
    /// Create a new Controller.
    pub fn new(opts: ControllerOptions, event_sink: Arc<dyn EventSink>) -> Self {
        let config = RuntimeConfig {
            max_iterations: opts.max_iterations,
            compaction_enabled: opts.compaction_enabled,
            ..RuntimeConfig::default()
        };

        let mut react_loop = ReActLoop::new(config);
        react_loop.set_system_prompt(opts.system_prompt.clone());

        Self {
            react_loop: Arc::new(Mutex::new(react_loop)),
            event_sink,
            running: Arc::new(Mutex::new(false)),
            cancel_token: CancellationToken::new(),
            working_dir: opts.working_dir,
            system_prompt: opts.system_prompt,
            memory_queue: Arc::new(Mutex::new(Vec::new())),
            plan_mode: Arc::new(Mutex::new(false)),
        }
    }

    /// Get the system prompt (immutable).
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    /// Set plan mode. Injected into user message, NOT system prompt.
    pub async fn set_plan_mode(&self, enabled: bool) {
        *self.plan_mode.lock().await = enabled;
        self.event_sink.emit(Event::PlanModeChanged { enabled });
    }

    /// Queue a memory update for the next turn.
    pub async fn queue_memory(&self, fact: String) {
        self.memory_queue.lock().await.push(fact.clone());
        self.event_sink.emit(Event::MemoryUpdated { fact });
    }

    /// Compose user message with mid-session injections.
    /// Drains memory_queue into the message (same logic as ReActLoop).
    pub async fn compose_user_message(&self, input: &str) -> String {
        let mut parts = Vec::new();

        let plan = *self.plan_mode.lock().await;
        if plan {
            parts.push(PLAN_MODE_MARKER.to_string());
        }

        let mut queue = self.memory_queue.lock().await;
        if !queue.is_empty() {
            let updates = queue
                .iter()
                .map(|m| format!("- {}", m))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("<memory-update>\n{}\n</memory-update>", updates));
            queue.clear();
        }

        parts.push(input.to_string());
        parts.join("\n\n")
    }

    /// Check if a turn is currently running.
    pub async fn is_running(&self) -> bool {
        *self.running.lock().await
    }

    /// Cancel the current turn.
    pub async fn cancel(&self) {
        self.cancel_token.cancel();
        info!("Turn cancelled");
    }

    /// Get the event sink.
    pub fn event_sink(&self) -> &dyn EventSink {
        self.event_sink.as_ref()
    }

    /// Get the working directory.
    pub fn working_dir(&self) -> &str {
        &self.working_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::event_sink::NullEventSink;

    #[tokio::test]
    async fn compose_plain_input() {
        let controller = Controller::new(ControllerOptions::default(), Arc::new(NullEventSink));
        let msg = controller.compose_user_message("hello").await;
        assert_eq!(msg, "hello");
    }

    #[tokio::test]
    async fn compose_with_plan_mode() {
        let controller = Controller::new(ControllerOptions::default(), Arc::new(NullEventSink));
        controller.set_plan_mode(true).await;
        let msg = controller.compose_user_message("hello").await;
        assert!(msg.contains("PLAN MODE ACTIVE"));
    }

    #[tokio::test]
    async fn compose_drains_memory_queue() {
        let controller = Controller::new(ControllerOptions::default(), Arc::new(NullEventSink));
        controller.queue_memory("fact 1".into()).await;
        controller.queue_memory("fact 2".into()).await;

        let msg = controller.compose_user_message("hello").await;
        assert!(msg.contains("<memory-update>"));
        assert!(msg.contains("fact 1"));
        assert!(msg.contains("fact 2"));

        // Queue should be drained
        let msg2 = controller.compose_user_message("world").await;
        assert!(!msg2.contains("<memory-update>"));
    }

    #[tokio::test]
    async fn system_prompt_immutable() {
        let controller = Controller::new(ControllerOptions::default(), Arc::new(NullEventSink));
        let p1 = controller.system_prompt().to_string();
        controller.set_plan_mode(true).await;
        controller.queue_memory("fact".into()).await;
        let p2 = controller.system_prompt().to_string();
        assert_eq!(p1, p2);
    }

    #[tokio::test]
    async fn cancel_with_no_turn() {
        let controller = Controller::new(ControllerOptions::default(), Arc::new(NullEventSink));
        controller.cancel().await; // should not panic
    }

    #[test]
    fn controller_options_default() {
        let opts = ControllerOptions::default();
        assert_eq!(opts.max_iterations, 0);
        assert!(opts.compaction_enabled);
        assert_eq!(opts.working_dir, "/tmp");
    }

    #[test]
    fn controller_options_custom() {
        let opts = ControllerOptions {
            working_dir: "/workspace".into(),
            data_dir: "/data".into(),
            system_prompt: "Custom prompt".into(),
            max_iterations: 10,
            compaction_enabled: false,
        };
        assert_eq!(opts.max_iterations, 10);
        assert!(!opts.compaction_enabled);
    }

    #[tokio::test]
    async fn working_dir_accessible() {
        let controller = Controller::new(ControllerOptions::default(), Arc::new(NullEventSink));
        assert_eq!(controller.working_dir(), "/tmp");
    }

    #[tokio::test]
    async fn is_running_initially_false() {
        let controller = Controller::new(ControllerOptions::default(), Arc::new(NullEventSink));
        assert!(!controller.is_running().await);
    }

    #[tokio::test]
    async fn compose_plan_and_memory() {
        let controller = Controller::new(ControllerOptions::default(), Arc::new(NullEventSink));
        controller.set_plan_mode(true).await;
        controller.queue_memory("fact 1".into()).await;

        let msg = controller.compose_user_message("do something").await;
        assert!(msg.contains("PLAN MODE ACTIVE"));
        assert!(msg.contains("<memory-update>"));
        assert!(msg.contains("fact 1"));
        assert!(msg.contains("do something"));
    }
}

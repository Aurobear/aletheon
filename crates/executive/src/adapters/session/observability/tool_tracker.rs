use std::collections::HashMap;
use std::sync::Arc;

use fabric::Clock;
use fabric::MonoTime;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// State of a tracked tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallState {
    pub tool_call_id: String,
    pub tool_name: String,
    pub registered_at_epoch_ms: u64,
    /// Whether the tool call has been sent to the provider.
    pub called: bool,
    /// Whether the tool call has received a final result.
    pub settled: bool,
    /// Whether the tool call ended in error.
    pub is_error: bool,
    /// Elapsed time in milliseconds (set on settle).
    pub elapsed_ms: Option<u64>,
}

/// Tracks tool calls through their lifecycle: registered -> called -> settled.
pub struct ToolTracker {
    calls: HashMap<String, ToolCallState>,
    created_at: MonoTime,
    clock: Arc<dyn Clock>,
}

impl ToolTracker {
    /// Create a new tracker.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        let created_at = clock.mono_now();
        Self {
            calls: HashMap::new(),
            created_at,
            clock,
        }
    }

    /// Register a new tool call (before it is sent to the provider).
    pub fn register(&mut self, tool_call_id: impl Into<String>, tool_name: impl Into<String>) {
        let tool_call_id = tool_call_id.into();
        let tool_name = tool_name.into();
        debug!(tool_call_id = %tool_call_id, tool_name = %tool_name, "Tool call registered");
        self.calls.insert(
            tool_call_id.clone(),
            ToolCallState {
                tool_call_id,
                tool_name,
                registered_at_epoch_ms: self.elapsed_ms(),
                called: false,
                settled: false,
                is_error: false,
                elapsed_ms: None,
            },
        );
    }

    /// Mark a tool call as having been sent to the provider.
    pub fn mark_called(&mut self, tool_call_id: &str) -> bool {
        if let Some(state) = self.calls.get_mut(tool_call_id) {
            state.called = true;
            debug!(tool_call_id, "Tool call marked as called");
            true
        } else {
            false
        }
    }

    /// Mark a tool call as settled (result received).
    pub fn mark_settled(&mut self, tool_call_id: &str, is_error: bool) -> bool {
        let now = self.elapsed_ms();
        if let Some(state) = self.calls.get_mut(tool_call_id) {
            state.settled = true;
            state.is_error = is_error;
            state.elapsed_ms = Some(now - state.registered_at_epoch_ms);
            debug!(
                tool_call_id,
                is_error,
                elapsed_ms = state.elapsed_ms,
                "Tool call settled"
            );
            true
        } else {
            false
        }
    }

    /// Return unsettled tool calls (registered or called but not settled).
    pub fn unsettled_calls(&self) -> Vec<&ToolCallState> {
        self.calls.values().filter(|s| !s.settled).collect()
    }

    /// Fail all unsettled tool calls (e.g. on session abort).
    /// Returns the number of calls marked as failed.
    pub fn fail_unsettled(&mut self) -> usize {
        let now = self.elapsed_ms();
        let mut count = 0;
        for state in self.calls.values_mut() {
            if !state.settled {
                state.settled = true;
                state.is_error = true;
                state.elapsed_ms = Some(now - state.registered_at_epoch_ms);
                count += 1;
            }
        }
        if count > 0 {
            debug!(count, "Failed all unsettled tool calls");
        }
        count
    }

    /// Total number of tracked calls.
    pub fn total_calls(&self) -> usize {
        self.calls.len()
    }

    fn elapsed_ms(&self) -> u64 {
        self.clock.mono_now().0.saturating_sub(self.created_at.0)
    }
}

impl Default for ToolTracker {
    fn default() -> Self {
        Self::new(Arc::new(kernel::chronos::TestClock::default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::chronos::TestClock;

    fn test_tracker() -> ToolTracker {
        ToolTracker::new(Arc::new(TestClock::default()))
    }

    #[test]
    fn test_register_and_mark_called() {
        let mut tracker = test_tracker();
        tracker.register("call-1", "bash");
        assert_eq!(tracker.total_calls(), 1);

        assert!(tracker.mark_called("call-1"));
        assert!(!tracker.mark_called("nonexistent"));
    }

    #[test]
    fn test_settle_and_unsettled() {
        let mut tracker = test_tracker();
        tracker.register("c1", "bash");
        tracker.register("c2", "read_file");
        tracker.mark_called("c1");

        assert_eq!(tracker.unsettled_calls().len(), 2);

        tracker.mark_settled("c1", false);
        let unsettled = tracker.unsettled_calls();
        assert_eq!(unsettled.len(), 1);
        assert_eq!(unsettled[0].tool_call_id, "c2");
    }

    #[test]
    fn test_fail_unsettled() {
        let mut tracker = test_tracker();
        tracker.register("c1", "bash");
        tracker.register("c2", "read_file");
        tracker.mark_called("c1");
        tracker.mark_settled("c1", false);

        let failed = tracker.fail_unsettled();
        assert_eq!(failed, 1);

        let unsettled = tracker.unsettled_calls();
        assert_eq!(unsettled.len(), 0);
        assert_eq!(tracker.total_calls(), 2);
    }
}

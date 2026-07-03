//! Storm breaker — detects and breaks model loops.
//!
//! Tracks consecutive identical tool failures and successes.
//! When a threshold is reached, injects a directive to change approach.

use std::collections::HashMap;

const DEFAULT_THRESHOLD: usize = 3;

/// Tracks tool call patterns to detect loops.
pub struct StormBreaker {
    /// Key: (tool_name, error_signature), Value: consecutive count
    failure_counts: HashMap<(String, String), usize>,
    /// Key: tool_name, Value: consecutive success count
    success_counts: HashMap<String, usize>,
    /// Threshold to trigger
    threshold: usize,
}

impl StormBreaker {
    pub fn new(threshold: usize) -> Self {
        Self {
            failure_counts: HashMap::new(),
            success_counts: HashMap::new(),
            threshold,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_THRESHOLD)
    }

    /// Record a tool call result. Returns a directive if loop detected.
    pub fn record(&mut self, tool_name: &str, is_error: bool, content: &str) -> Option<String> {
        if is_error {
            let error_sig = Self::extract_error_signature(content);
            let key = (tool_name.to_string(), error_sig);
            let count = self.failure_counts.entry(key).or_insert(0);
            *count += 1;

            // Reset success counter for this tool
            self.success_counts.remove(tool_name);

            if *count >= self.threshold {
                return Some(format!(
                    "⚠️ Storm breaker: {} has failed {} times with the same error. \
                     Previous attempts did not work. Try a completely different approach.",
                    tool_name, count
                ));
            }
        } else {
            let count = self
                .success_counts
                .entry(tool_name.to_string())
                .or_insert(0);
            *count += 1;

            // Reset failure counters for this tool
            self.failure_counts.retain(|k, _| k.0 != tool_name);

            if *count >= self.threshold {
                return Some(format!(
                    "⚠️ Storm breaker: {} has succeeded {} times in a row. \
                     Verify the result is correct before continuing.",
                    tool_name, count
                ));
            }
        }
        None
    }

    /// Reset all counters (called at turn start).
    pub fn reset(&mut self) {
        self.failure_counts.clear();
        self.success_counts.clear();
    }

    /// Total number of unique failure patterns being tracked.
    pub fn failure_count(&self) -> usize {
        self.failure_counts.len()
    }

    /// Check if any pattern has reached the loop threshold.
    pub fn has_triggered(&self) -> bool {
        self.failure_counts.values().any(|&c| c >= self.threshold)
            || self.success_counts.values().any(|&c| c >= self.threshold)
    }

    /// Extract a normalized error signature for comparison.
    fn extract_error_signature(content: &str) -> String {
        // Take first 100 chars, lowercase, collapse whitespace
        let s: String = content
            .chars()
            .take(100)
            .flat_map(|c| {
                if c.is_whitespace() {
                    vec![' ']
                } else {
                    vec![c.to_ascii_lowercase()]
                }
            })
            .collect();
        s.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_trigger_below_threshold() {
        let mut sb = StormBreaker::new(3);
        assert!(sb.record("bash", true, "error: file not found").is_none());
        assert!(sb.record("bash", true, "error: file not found").is_none());
    }

    #[test]
    fn trigger_on_consecutive_failures() {
        let mut sb = StormBreaker::new(3);
        sb.record("bash", true, "error: file not found");
        sb.record("bash", true, "error: file not found");
        let directive = sb.record("bash", true, "error: file not found");
        assert!(directive.is_some());
        assert!(directive.unwrap().contains("Storm breaker"));
    }

    #[test]
    fn reset_on_success() {
        let mut sb = StormBreaker::new(3);
        sb.record("bash", true, "error: file not found");
        sb.record("bash", false, "ok"); // success resets
        sb.record("bash", true, "error: file not found");
        sb.record("bash", true, "error: file not found");
        // Only 2 consecutive failures after reset, not 3
        assert!(sb.record("bash", true, "error: file not found").is_some());
    }

    #[test]
    fn different_errors_dont_trigger() {
        let mut sb = StormBreaker::new(3);
        sb.record("bash", true, "error: file not found");
        sb.record("bash", true, "error: permission denied");
        assert!(sb.record("bash", true, "error: timeout").is_none());
    }

    #[test]
    fn trigger_on_consecutive_successes() {
        let mut sb = StormBreaker::new(3);
        sb.record("write_file", false, "ok");
        sb.record("write_file", false, "ok");
        let warning = sb.record("write_file", false, "ok");
        assert!(warning.is_some());
        assert!(warning.unwrap().contains("succeeded"));
    }

    #[test]
    fn reset_clears_all() {
        let mut sb = StormBreaker::new(2);
        sb.record("bash", true, "error");
        sb.record("write_file", false, "ok");
        sb.reset();
        assert!(sb.record("bash", true, "error").is_none());
        assert!(sb.record("write_file", false, "ok").is_none());
    }

    #[test]
    fn different_tools_independent() {
        let mut sb = StormBreaker::new(3);
        sb.record("bash", true, "error");
        sb.record("grep", true, "error");
        sb.record("bash", true, "error");
        // bash has 2, grep has 1 — neither triggers
        assert!(sb.record("grep", true, "error").is_none());
    }
}

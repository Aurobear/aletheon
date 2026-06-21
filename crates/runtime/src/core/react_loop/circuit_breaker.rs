// crates/runtime/src/core/react_loop/circuit_breaker.rs
use std::collections::VecDeque;
use tracing::warn;

/// Signature of a tool call for loop detection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolCallSignature {
    pub tool_name: String,
    pub args_hash: u64,
}

impl ToolCallSignature {
    pub fn new(tool_name: &str, args: &serde_json::Value) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        args.to_string().hash(&mut hasher);
        Self {
            tool_name: tool_name.to_string(),
            args_hash: hasher.finish(),
        }
    }
}

/// Status returned by circuit breaker check.
#[derive(Debug, Clone)]
pub enum CircuitBreakerStatus {
    /// No issues detected.
    Ok,
    /// Warning: pattern detected but not yet tripped.
    Warning(String),
    /// Circuit tripped: loop detected, must stop.
    Tripped(String),
}

/// Detects infinite loops and repeated tool calls.
#[derive(Debug)]
pub struct CircuitBreaker {
    recent_calls: VecDeque<ToolCallSignature>,
    max_repeats: usize,
    window_size: usize,
}

impl CircuitBreaker {
    /// Create a new circuit breaker.
    /// - max_repeats: number of identical calls *allowed* before the *next* one
    ///   trips the breaker. E.g. max_repeats=3 means 3 calls get Ok/Warning,
    ///   and the 4th call triggers Tripped.
    /// - window_size: how many recent calls to track
    pub fn new(max_repeats: usize, window_size: usize) -> Self {
        Self {
            recent_calls: VecDeque::with_capacity(window_size),
            max_repeats,
            window_size,
        }
    }

    /// Check if a new tool call would trip the circuit breaker.
    /// Returns status indicating if it's safe to proceed.
    pub fn check(&mut self, call: &ToolCallSignature) -> CircuitBreakerStatus {
        // Count how many times this exact call appears in the window
        let count = self.recent_calls.iter().filter(|c| *c == call).count();

        // Always add to window so the count increases on repeated calls
        if self.recent_calls.len() >= self.window_size {
            self.recent_calls.pop_front();
        }
        self.recent_calls.push_back(call.clone());

        if count >= self.max_repeats {
            let reason = format!(
                "Loop detected: tool '{}' with same args called {} times in last {} calls",
                call.tool_name, count + 1, self.window_size
            );
            warn!("{}", reason);
            CircuitBreakerStatus::Tripped(reason)
        } else if count >= self.max_repeats - 1 {
            let reason = format!(
                "Warning: tool '{}' with same args called {} times, will trip at {}",
                call.tool_name, count + 1, self.max_repeats
            );
            CircuitBreakerStatus::Warning(reason)
        } else {
            CircuitBreakerStatus::Ok
        }
    }

    /// Reset the circuit breaker for a new turn.
    pub fn reset(&mut self) {
        self.recent_calls.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_loop() {
        let mut cb = CircuitBreaker::new(3, 10);
        let call1 = ToolCallSignature::new("read_file", &serde_json::json!({"path": "/a"}));
        let call2 = ToolCallSignature::new("read_file", &serde_json::json!({"path": "/b"}));

        assert!(matches!(cb.check(&call1), CircuitBreakerStatus::Ok));
        assert!(matches!(cb.check(&call2), CircuitBreakerStatus::Ok));
    }

    #[test]
    fn test_loop_detection() {
        let mut cb = CircuitBreaker::new(3, 10);
        let call = ToolCallSignature::new("bash_exec", &serde_json::json!({"command": "ls"}));

        assert!(matches!(cb.check(&call), CircuitBreakerStatus::Ok));
        assert!(matches!(cb.check(&call), CircuitBreakerStatus::Ok));
        assert!(matches!(cb.check(&call), CircuitBreakerStatus::Warning(_)));
        assert!(matches!(cb.check(&call), CircuitBreakerStatus::Tripped(_)));
    }

    #[test]
    fn test_different_args_no_loop() {
        let mut cb = CircuitBreaker::new(10, 20);
        let call1 = ToolCallSignature::new("bash_exec", &serde_json::json!({"command": "ls"}));
        let call2 = ToolCallSignature::new("bash_exec", &serde_json::json!({"command": "pwd"}));

        for _ in 0..5 {
            assert!(matches!(cb.check(&call1), CircuitBreakerStatus::Ok));
            assert!(matches!(cb.check(&call2), CircuitBreakerStatus::Ok));
        }
    }

    #[test]
    fn test_reset() {
        let mut cb = CircuitBreaker::new(2, 10);
        let call = ToolCallSignature::new("test", &serde_json::json!({}));

        cb.check(&call);
        cb.check(&call);
        assert!(matches!(cb.check(&call), CircuitBreakerStatus::Tripped(_)));

        cb.reset();
        assert!(matches!(cb.check(&call), CircuitBreakerStatus::Ok));
    }
}

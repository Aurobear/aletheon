//! ResourceGovernor: enforce resource limits and trigger throttling.
//!
//! Tracks token usage, tool calls, memory, disk writes, and CPU.
//! Returns throttle actions when limits are approached.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use fabric::MonoTime;

/// Helper: compute elapsed Duration between two MonoTime values.
fn mono_elapsed(now: MonoTime, earlier: MonoTime) -> Duration {
    Duration::from_millis(now.0.saturating_sub(earlier.0))
}

// ── Resource Limits ─────────────────────────────────────────────────────────

/// Configurable resource limits.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResourceLimits {
    /// Max tokens per single inference turn.
    pub max_tokens_per_turn: u32,
    /// Max tokens per hour.
    pub max_tokens_per_hour: u32,
    /// Max tool calls per turn.
    pub max_tool_calls_per_turn: u32,
    /// Max concurrent tool executions.
    pub max_concurrent_tools: u32,
    /// Max memory usage in MB.
    pub max_memory_mb: u64,
    /// Max disk writes per hour in MB.
    pub max_disk_write_mb_per_hour: u64,
    /// Max CPU percentage (0-100).
    pub max_cpu_percent: f32,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_tokens_per_turn: 100_000,
            max_tokens_per_hour: 500_000,
            max_tool_calls_per_turn: 50,
            max_concurrent_tools: 8,
            max_memory_mb: 500,
            max_disk_write_mb_per_hour: 1024,
            max_cpu_percent: 80.0,
        }
    }
}

// ── Resource Usage ──────────────────────────────────────────────────────────

/// Current resource usage counters.
#[derive(Debug, Clone)]
pub struct ResourceUsage {
    /// Tokens used in the current turn.
    pub tokens_this_turn: u32,
    /// Tokens used in the current hour.
    pub tokens_this_hour: u32,
    /// Tool calls in the current turn.
    pub tool_calls_this_turn: u32,
    /// Currently active concurrent tool executions.
    pub active_tools: u32,
    /// Current memory usage in MB.
    pub memory_mb: u64,
    /// Disk writes in the current hour in MB.
    pub disk_write_mb_hour: u64,
    /// Current CPU percentage.
    pub cpu_percent: f32,
    /// When the hourly window started.
    pub hour_window_start: MonoTime,
}

impl ResourceUsage {
    /// Reset the hourly counters if the window has elapsed.
    pub fn maybe_reset_hourly(&mut self, clock: &dyn fabric::Clock) {
        let now = clock.mono_now();
        if mono_elapsed(now, self.hour_window_start) >= Duration::from_secs(3600) {
            self.tokens_this_hour = 0;
            self.disk_write_mb_hour = 0;
            self.hour_window_start = now;
        }
    }

    /// Reset per-turn counters.
    pub fn reset_turn(&mut self) {
        self.tokens_this_turn = 0;
        self.tool_calls_this_turn = 0;
    }
}

// ── Resource Request ────────────────────────────────────────────────────────

/// A request to consume resources.
#[derive(Debug, Clone)]
pub enum ResourceRequest {
    /// Request to use N tokens.
    Tokens(u32),
    /// Request to execute a tool call.
    ToolCall,
    /// Request to run a concurrent tool.
    ConcurrentTool,
    /// Request to write N MB to disk.
    DiskWrite(u64),
}

// ── Resource Violation ──────────────────────────────────────────────────────

/// Which resource limit was violated.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ResourceViolation {
    TokenTurnLimit { current: u32, limit: u32 },
    TokenHourLimit { current: u32, limit: u32 },
    ToolCallLimit { current: u32, limit: u32 },
    ConcurrencyLimit { current: u32, limit: u32 },
    MemoryLimit { current: u64, limit: u64 },
    DiskWriteLimit { current: u64, limit: u64 },
    CpuLimit { current: f32, limit: f32 },
}

impl std::fmt::Display for ResourceViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TokenTurnLimit { current, limit } => {
                write!(f, "Token turn limit: {}/{}", current, limit)
            }
            Self::TokenHourLimit { current, limit } => {
                write!(f, "Token hour limit: {}/{}", current, limit)
            }
            Self::ToolCallLimit { current, limit } => {
                write!(f, "Tool call limit: {}/{}", current, limit)
            }
            Self::ConcurrencyLimit { current, limit } => {
                write!(f, "Concurrency limit: {}/{}", current, limit)
            }
            Self::MemoryLimit { current, limit } => {
                write!(f, "Memory limit: {}MB/{}MB", current, limit)
            }
            Self::DiskWriteLimit { current, limit } => {
                write!(f, "Disk write limit: {}MB/{}MB", current, limit)
            }
            Self::CpuLimit { current, limit } => {
                write!(f, "CPU limit: {:.1}%/{:.1}%", current, limit)
            }
        }
    }
}

impl std::error::Error for ResourceViolation {}

// ── Throttle Action ─────────────────────────────────────────────────────────

/// Action to take when resources are constrained.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ThrottleAction {
    /// No throttling needed.
    None,
    /// Reduce context window size.
    ReduceContext,
    /// Force local model inference only.
    ForceLocalOnly,
    /// Reject new tasks.
    RejectNewTasks,
    /// Enter safe mode.
    EnterSafeMode,
}

// ── ResourceGovernor ────────────────────────────────────────────────────────

/// Enforces resource limits and returns throttle actions.
pub struct ResourceGovernor {
    pub limits: ResourceLimits,
    pub usage: Arc<Mutex<ResourceUsage>>,
    clock: Arc<dyn fabric::Clock>,
}

impl ResourceGovernor {
    /// Create with default limits.
    pub fn new(clock: Arc<dyn fabric::Clock>) -> Self {
        Self {
            limits: ResourceLimits::default(),
            usage: Arc::new(Mutex::new(ResourceUsage {
                tokens_this_turn: 0,
                tokens_this_hour: 0,
                tool_calls_this_turn: 0,
                active_tools: 0,
                memory_mb: 0,
                disk_write_mb_hour: 0,
                cpu_percent: 0.0,
                hour_window_start: clock.mono_now(),
            })),
            clock,
        }
    }

    /// Create with custom limits.
    pub fn with_limits(limits: ResourceLimits, clock: Arc<dyn fabric::Clock>) -> Self {
        Self {
            limits,
            usage: Arc::new(Mutex::new(ResourceUsage {
                tokens_this_turn: 0,
                tokens_this_hour: 0,
                tool_calls_this_turn: 0,
                active_tools: 0,
                memory_mb: 0,
                disk_write_mb_hour: 0,
                cpu_percent: 0.0,
                hour_window_start: clock.mono_now(),
            })),
            clock,
        }
    }

    /// Check if a resource request can be fulfilled.
    pub fn check_allow(&self, request: &ResourceRequest) -> Result<(), ResourceViolation> {
        let mut usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        usage.maybe_reset_hourly(&*self.clock);

        match request {
            ResourceRequest::Tokens(n) => {
                // Per-turn check
                if usage.tokens_this_turn + n > self.limits.max_tokens_per_turn {
                    return Err(ResourceViolation::TokenTurnLimit {
                        current: usage.tokens_this_turn,
                        limit: self.limits.max_tokens_per_turn,
                    });
                }
                // Per-hour check
                if usage.tokens_this_hour + n > self.limits.max_tokens_per_hour {
                    return Err(ResourceViolation::TokenHourLimit {
                        current: usage.tokens_this_hour,
                        limit: self.limits.max_tokens_per_hour,
                    });
                }
                usage.tokens_this_turn += n;
                usage.tokens_this_hour += n;
            }
            ResourceRequest::ToolCall => {
                if usage.tool_calls_this_turn + 1 > self.limits.max_tool_calls_per_turn {
                    return Err(ResourceViolation::ToolCallLimit {
                        current: usage.tool_calls_this_turn,
                        limit: self.limits.max_tool_calls_per_turn,
                    });
                }
                usage.tool_calls_this_turn += 1;
            }
            ResourceRequest::ConcurrentTool => {
                if usage.active_tools + 1 > self.limits.max_concurrent_tools {
                    return Err(ResourceViolation::ConcurrencyLimit {
                        current: usage.active_tools,
                        limit: self.limits.max_concurrent_tools,
                    });
                }
                usage.active_tools += 1;
            }
            ResourceRequest::DiskWrite(mb) => {
                if usage.disk_write_mb_hour + mb > self.limits.max_disk_write_mb_per_hour {
                    return Err(ResourceViolation::DiskWriteLimit {
                        current: usage.disk_write_mb_hour,
                        limit: self.limits.max_disk_write_mb_per_hour,
                    });
                }
                usage.disk_write_mb_hour += mb;
            }
        }

        Ok(())
    }

    /// Release a concurrent tool slot.
    pub fn release_concurrent_tool(&self) {
        let mut usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        if usage.active_tools > 0 {
            usage.active_tools -= 1;
        }
    }

    /// Reset per-turn counters (call at turn boundary).
    pub fn reset_turn(&self) {
        let mut usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        usage.reset_turn();
    }

    /// Determine throttle action based on current usage ratios.
    pub fn emergency_throttle(&self) -> ThrottleAction {
        let usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());

        let token_ratio = usage.tokens_this_hour as f32 / self.limits.max_tokens_per_hour as f32;
        let memory_ratio = usage.memory_mb as f32 / self.limits.max_memory_mb as f32;

        if token_ratio > 0.95 || memory_ratio > 0.95 {
            ThrottleAction::EnterSafeMode
        } else if token_ratio > 0.80 || memory_ratio > 0.80 {
            ThrottleAction::RejectNewTasks
        } else if token_ratio > 0.60 || memory_ratio > 0.60 {
            ThrottleAction::ForceLocalOnly
        } else if token_ratio > 0.40 {
            ThrottleAction::ReduceContext
        } else {
            ThrottleAction::None
        }
    }

    /// Get current usage snapshot.
    pub fn current_usage(&self) -> ResourceUsage {
        self.usage.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::chronos::TestClock;

    fn test_clock() -> Arc<dyn fabric::Clock> {
        Arc::new(TestClock::default())
    }

    fn test_governor(limits: ResourceLimits) -> ResourceGovernor {
        ResourceGovernor::with_limits(limits, test_clock())
    }

    #[test]
    fn test_token_turn_limit() {
        let governor = test_governor(ResourceLimits {
            max_tokens_per_turn: 100,
            max_tokens_per_hour: 10000,
            ..Default::default()
        });

        assert!(governor.check_allow(&ResourceRequest::Tokens(50)).is_ok());
        assert!(governor.check_allow(&ResourceRequest::Tokens(60)).is_err());
    }

    #[test]
    fn test_token_hour_limit() {
        let governor = test_governor(ResourceLimits {
            max_tokens_per_turn: 10000,
            max_tokens_per_hour: 100,
            ..Default::default()
        });

        assert!(governor.check_allow(&ResourceRequest::Tokens(50)).is_ok());
        assert!(governor.check_allow(&ResourceRequest::Tokens(60)).is_err());
    }

    #[test]
    fn test_tool_call_limit() {
        let governor = test_governor(ResourceLimits {
            max_tool_calls_per_turn: 3,
            ..Default::default()
        });

        assert!(governor.check_allow(&ResourceRequest::ToolCall).is_ok());
        assert!(governor.check_allow(&ResourceRequest::ToolCall).is_ok());
        assert!(governor.check_allow(&ResourceRequest::ToolCall).is_ok());
        assert!(governor.check_allow(&ResourceRequest::ToolCall).is_err());
    }

    #[test]
    fn test_concurrency_limit() {
        let governor = test_governor(ResourceLimits {
            max_concurrent_tools: 2,
            ..Default::default()
        });

        assert!(governor
            .check_allow(&ResourceRequest::ConcurrentTool)
            .is_ok());
        assert!(governor
            .check_allow(&ResourceRequest::ConcurrentTool)
            .is_ok());
        assert!(governor
            .check_allow(&ResourceRequest::ConcurrentTool)
            .is_err());

        governor.release_concurrent_tool();
        assert!(governor
            .check_allow(&ResourceRequest::ConcurrentTool)
            .is_ok());
    }

    #[test]
    fn test_disk_write_limit() {
        let governor = test_governor(ResourceLimits {
            max_disk_write_mb_per_hour: 100,
            ..Default::default()
        });

        assert!(governor
            .check_allow(&ResourceRequest::DiskWrite(50))
            .is_ok());
        assert!(governor
            .check_allow(&ResourceRequest::DiskWrite(60))
            .is_err());
    }

    #[test]
    fn test_reset_turn() {
        let governor = test_governor(ResourceLimits {
            max_tokens_per_turn: 100,
            max_tokens_per_hour: 10000,
            ..Default::default()
        });

        governor.check_allow(&ResourceRequest::Tokens(90)).unwrap();
        assert!(governor.check_allow(&ResourceRequest::Tokens(20)).is_err());

        governor.reset_turn();
        assert!(governor.check_allow(&ResourceRequest::Tokens(20)).is_ok());
    }

    #[test]
    fn test_emergency_throttle_none() {
        let governor = test_governor(ResourceLimits::default());
        assert_eq!(governor.emergency_throttle(), ThrottleAction::None);
    }

    #[test]
    fn test_emergency_throttle_high_usage() {
        let governor = test_governor(ResourceLimits {
            max_tokens_per_hour: 100,
            ..Default::default()
        });

        // Use 70% of tokens
        governor.check_allow(&ResourceRequest::Tokens(70)).unwrap();
        assert_eq!(
            governor.emergency_throttle(),
            ThrottleAction::ForceLocalOnly
        );
    }

    #[test]
    fn test_emergency_throttle_critical() {
        let governor = test_governor(ResourceLimits {
            max_tokens_per_hour: 100,
            ..Default::default()
        });

        // Use 96% of tokens
        governor.check_allow(&ResourceRequest::Tokens(96)).unwrap();
        assert_eq!(governor.emergency_throttle(), ThrottleAction::EnterSafeMode);
    }

    #[test]
    fn test_resource_violation_display() {
        let v = ResourceViolation::TokenTurnLimit {
            current: 50,
            limit: 100,
        };
        assert!(format!("{}", v).contains("50/100"));
    }
}

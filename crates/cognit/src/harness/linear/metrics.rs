/// Metrics collected during a single ReAct turn.
#[derive(Debug, Clone)]
pub struct TurnMetrics {
    pub tool_calls_made: usize,
    pub tool_errors: usize,
    /// Provider requests repeated after a transient inference failure.
    pub provider_retries: u64,
    pub elapsed_ms: u64,
    pub iterations: usize,
    pub completed_normally: bool,
    pub stop: ::contracts::TurnStop,
}

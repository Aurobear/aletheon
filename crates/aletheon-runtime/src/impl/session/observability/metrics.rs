use serde::{Deserialize, Serialize};
use tracing::debug;

/// Breakdown of token usage across a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsageBreakdown {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl TokenUsageBreakdown {
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

/// Aggregated metrics for a session.
#[derive(Debug, Clone, Default)]
struct MetricsState {
    llm_call_count: u64,
    total_inference_latency_ms: u64,
    tool_call_count: u64,
    total_tool_latency_ms: u64,
    hook_execution_count: u64,
    total_hook_latency_ms: u64,
    token_usage: TokenUsageBreakdown,
}

/// Exporter that accumulates session metrics.
pub struct MetricsExporter {
    state: MetricsState,
}

impl MetricsExporter {
    /// Create a new metrics exporter.
    pub fn new() -> Self {
        Self {
            state: MetricsState::default(),
        }
    }

    /// Record an LLM inference call.
    pub fn record_inference(&mut self, input_tokens: u64, output_tokens: u64, latency_ms: u64) {
        self.state.llm_call_count += 1;
        self.state.total_inference_latency_ms += latency_ms;
        self.state.token_usage.input_tokens += input_tokens;
        self.state.token_usage.output_tokens += output_tokens;
        debug!(
            input_tokens,
            output_tokens, latency_ms, "Recorded inference metrics"
        );
    }

    /// Record a tool call.
    pub fn record_tool_call(&mut self, latency_ms: u64) {
        self.state.tool_call_count += 1;
        self.state.total_tool_latency_ms += latency_ms;
        debug!(latency_ms, "Recorded tool call metrics");
    }

    /// Record a hook execution.
    pub fn record_hook_execution(&mut self, latency_ms: u64) {
        self.state.hook_execution_count += 1;
        self.state.total_hook_latency_ms += latency_ms;
        debug!(latency_ms, "Recorded hook execution metrics");
    }

    /// Add cache token usage (e.g. from prompt caching).
    pub fn record_cache_usage(&mut self, read_tokens: u64, write_tokens: u64) {
        self.state.token_usage.cache_read_tokens += read_tokens;
        self.state.token_usage.cache_write_tokens += write_tokens;
    }

    /// Get the current token usage breakdown.
    pub fn token_usage(&self) -> &TokenUsageBreakdown {
        &self.state.token_usage
    }

    /// Get the number of LLM calls.
    pub fn llm_call_count(&self) -> u64 {
        self.state.llm_call_count
    }

    /// Get the number of tool calls.
    pub fn tool_call_count(&self) -> u64 {
        self.state.tool_call_count
    }

    /// Get the number of hook executions.
    pub fn hook_execution_count(&self) -> u64 {
        self.state.hook_execution_count
    }

    /// Get the total inference latency in ms.
    pub fn total_inference_latency_ms(&self) -> u64 {
        self.state.total_inference_latency_ms
    }

    /// Get the total tool call latency in ms.
    pub fn total_tool_latency_ms(&self) -> u64 {
        self.state.total_tool_latency_ms
    }

    /// Get the total hook execution latency in ms.
    pub fn total_hook_latency_ms(&self) -> u64 {
        self.state.total_hook_latency_ms
    }
}

impl Default for MetricsExporter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_inference_accumulates() {
        let mut exporter = MetricsExporter::new();
        exporter.record_inference(100, 50, 300);
        exporter.record_inference(200, 80, 500);

        assert_eq!(exporter.llm_call_count(), 2);
        assert_eq!(exporter.total_inference_latency_ms(), 800);
        assert_eq!(exporter.token_usage().input_tokens, 300);
        assert_eq!(exporter.token_usage().output_tokens, 130);
        assert_eq!(exporter.token_usage().total(), 430);
    }

    #[test]
    fn test_record_tool_call() {
        let mut exporter = MetricsExporter::new();
        exporter.record_tool_call(100);
        exporter.record_tool_call(200);
        exporter.record_tool_call(150);

        assert_eq!(exporter.tool_call_count(), 3);
        assert_eq!(exporter.total_tool_latency_ms(), 450);
    }

    #[test]
    fn test_record_hook_execution() {
        let mut exporter = MetricsExporter::new();
        exporter.record_hook_execution(50);

        assert_eq!(exporter.hook_execution_count(), 1);
        assert_eq!(exporter.total_hook_latency_ms(), 50);
    }

    #[test]
    fn test_cache_usage() {
        let mut exporter = MetricsExporter::new();
        exporter.record_cache_usage(1000, 500);
        assert_eq!(exporter.token_usage().cache_read_tokens, 1000);
        assert_eq!(exporter.token_usage().cache_write_tokens, 500);
        assert_eq!(exporter.token_usage().total(), 1500);
    }
}

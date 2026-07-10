use std::collections::HashMap;

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

/// USD price per 1K tokens for a provider (mirrors `ProviderPricing` from config).
#[derive(Debug, Clone, Copy)]
pub struct PricingRate {
    pub input_per_1k: f64,
    pub output_per_1k: f64,
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
    /// Per-provider token attribution.
    per_provider: HashMap<String, TokenUsageBreakdown>,
    /// Static pricing table (populated from config at init).
    pricing: HashMap<String, PricingRate>,
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

    /// Record inference attributed to a specific provider (also updates globals).
    pub fn record_inference_for(
        &mut self,
        provider: &str,
        input_tokens: u64,
        output_tokens: u64,
        latency_ms: u64,
    ) {
        self.record_inference(input_tokens, output_tokens, latency_ms);
        let e = self
            .state
            .per_provider
            .entry(provider.to_string())
            .or_default();
        e.input_tokens += input_tokens;
        e.output_tokens += output_tokens;
    }

    /// Record cache usage attributed to a specific provider (also updates globals).
    pub fn record_cache_usage_for(&mut self, provider: &str, read_tokens: u64, write_tokens: u64) {
        self.record_cache_usage(read_tokens, write_tokens);
        let e = self
            .state
            .per_provider
            .entry(provider.to_string())
            .or_default();
        e.cache_read_tokens += read_tokens;
        e.cache_write_tokens += write_tokens;
    }

    /// Token usage for one provider, if any was recorded.
    pub fn provider_usage(&self, provider: &str) -> Option<&TokenUsageBreakdown> {
        self.state.per_provider.get(provider)
    }

    /// Install a static pricing rate for a provider.
    pub fn set_pricing(&mut self, provider: &str, rate: PricingRate) {
        self.state.pricing.insert(provider.to_string(), rate);
    }

    /// Cost in USD for one provider (tracked + priced). Returns `None` if unpriced.
    pub fn cost_for(&self, provider: &str) -> Option<f64> {
        let usage = self.state.per_provider.get(provider)?;
        let rate = self.state.pricing.get(provider)?;
        Some(
            (usage.input_tokens as f64 / 1000.0) * rate.input_per_1k
                + (usage.output_tokens as f64 / 1000.0) * rate.output_per_1k,
        )
    }

    /// Total cost across all priced+tracked providers.
    pub fn total_cost(&self) -> f64 {
        self.state
            .per_provider
            .keys()
            .filter_map(|p| self.cost_for(p))
            .sum()
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

    #[test]
    fn per_provider_attribution_and_cost() {
        let mut ex = MetricsExporter::new();
        ex.record_inference_for("anthropic", 1_000, 500, 300);
        ex.record_inference_for("openai", 2_000, 1_000, 400);
        ex.record_cache_usage_for("anthropic", 800, 0);

        // Per-provider attribution
        let a = ex.provider_usage("anthropic").expect("anthropic tracked");
        assert_eq!(a.input_tokens, 1_000);
        assert_eq!(a.output_tokens, 500);
        assert_eq!(a.cache_read_tokens, 800);
        let o = ex.provider_usage("openai").expect("openai tracked");
        assert_eq!(o.input_tokens, 2_000);

        // Global aggregate still correct
        assert_eq!(ex.token_usage().input_tokens, 3_000);
        assert_eq!(ex.token_usage().output_tokens, 1_500);
        assert_eq!(ex.llm_call_count(), 2);

        // Cost: only anthropic priced -> 1.0k*$3 + 0.5k*$15 = $10.50
        ex.set_pricing(
            "anthropic",
            PricingRate {
                input_per_1k: 3.0,
                output_per_1k: 15.0,
            },
        );
        assert!((ex.cost_for("anthropic").unwrap() - 10.5).abs() < 1e-9);
        assert!(
            ex.cost_for("openai").is_none(),
            "unpriced provider has no cost"
        );
        assert!((ex.total_cost() - 10.5).abs() < 1e-9);
    }

    #[test]
    fn existing_global_tests_still_pass() {
        // Verify existing global methods still work without per-provider data.
        let mut ex = MetricsExporter::new();
        ex.record_inference(100, 50, 300);
        assert_eq!(ex.llm_call_count(), 1);
        assert_eq!(ex.token_usage().input_tokens, 100);
        assert!(ex.provider_usage("any").is_none());
    }
}

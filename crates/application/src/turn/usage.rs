//! Turn usage accounting with completeness tracked independently per metric.

#[derive(Debug, Default)]
pub struct TurnUsageAccumulator {
    input_tokens: u64,
    output_tokens: u64,
    usage_seen: bool,
    input_complete: bool,
    output_complete: bool,
    cache_read_tokens: u64,
    cache_read_seen: bool,
    cache_read_complete: bool,
    cache_write_tokens: u64,
    cache_write_seen: bool,
    cache_write_complete: bool,
    active_context_tokens: Option<u64>,
}

impl TurnUsageAccumulator {
    pub fn new() -> Self {
        Self {
            input_complete: true,
            output_complete: true,
            cache_read_complete: true,
            cache_write_complete: true,
            ..Default::default()
        }
    }

    pub fn observe(&mut self, usage: &contracts::InferenceUsage) {
        self.usage_seen = true;
        match usage.total_input_tokens {
            Some(value) => self.input_tokens = self.input_tokens.saturating_add(value),
            None => self.input_complete = false,
        }
        match usage.output_tokens {
            Some(value) => self.output_tokens = self.output_tokens.saturating_add(value),
            None => self.output_complete = false,
        }
        match usage.cache_read_tokens {
            Some(value) => {
                self.cache_read_seen = true;
                self.cache_read_tokens = self.cache_read_tokens.saturating_add(value);
            }
            None => self.cache_read_complete = false,
        }
        match usage.cache_write_tokens {
            Some(value) => {
                self.cache_write_seen = true;
                self.cache_write_tokens = self.cache_write_tokens.saturating_add(value);
            }
            None => self.cache_write_complete = false,
        }
    }

    pub fn observe_active_context(&mut self, used_tokens: u64) {
        self.active_context_tokens = Some(used_tokens);
    }

    pub fn token_totals(&self) -> (u64, u64) {
        (self.input_tokens, self.output_tokens)
    }

    pub fn projection_metrics(
        &self,
        metrics: &contracts::TurnMetrics,
    ) -> crate::evaluation_projection::EvaluationProjectionMetrics {
        crate::evaluation_projection::EvaluationProjectionMetrics {
            elapsed_ms: Some(metrics.elapsed_ms),
            inference_rounds: Some(metrics.iterations.try_into().unwrap_or(u64::MAX)),
            provider_retries: Some(metrics.provider_retries),
            tool_calls: Some(metrics.tool_calls_made as u64),
            tool_errors: Some(metrics.tool_errors as u64),
            cumulative_input_tokens: (self.usage_seen && self.input_complete)
                .then_some(self.input_tokens),
            cumulative_output_tokens: (self.usage_seen && self.output_complete)
                .then_some(self.output_tokens),
            active_context_tokens: self.active_context_tokens,
            cache_read_tokens: (self.cache_read_seen && self.cache_read_complete)
                .then_some(self.cache_read_tokens),
            cache_write_tokens: (self.cache_write_seen && self.cache_write_complete)
                .then_some(self.cache_write_tokens),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_dimensions_do_not_erase_independent_metrics() {
        let mut usage = TurnUsageAccumulator::new();
        usage.observe(&contracts::InferenceUsage {
            total_input_tokens: Some(10),
            output_tokens: None,
            cache_read_tokens: Some(3),
            cache_write_tokens: None,
            ..Default::default()
        });
        usage.observe_active_context(7);
        let projection = usage.projection_metrics(&contracts::TurnMetrics::default());
        assert_eq!(projection.cumulative_input_tokens, Some(10));
        assert_eq!(projection.cumulative_output_tokens, None);
        assert_eq!(projection.cache_read_tokens, Some(3));
        assert_eq!(projection.cache_write_tokens, None);
        assert_eq!(projection.active_context_tokens, Some(7));
    }

    #[test]
    fn inference_rounds_retries_and_tool_calls_remain_separate() {
        let projection = TurnUsageAccumulator::new().projection_metrics(&contracts::TurnMetrics {
            iterations: 4,
            provider_retries: 2,
            tool_calls_made: 1,
            ..Default::default()
        });
        assert_eq!(projection.inference_rounds, Some(4));
        assert_eq!(projection.provider_retries, Some(2));
        assert_eq!(projection.tool_calls, Some(1));
    }
}

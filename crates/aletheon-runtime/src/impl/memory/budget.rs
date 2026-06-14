/// Context budget tracking -- prevents token overflow.
pub struct ContextBudget {
    max_tokens: usize,
    used_tokens: usize,
    compact_threshold: f64,  // e.g. 0.7 = compact at 70%
}

impl ContextBudget {
    pub fn new(max_tokens: usize, compact_threshold: f64) -> Self {
        Self {
            max_tokens,
            used_tokens: 0,
            compact_threshold,
        }
    }

    pub fn record_usage(&mut self, tokens: usize) {
        self.used_tokens = self.used_tokens.saturating_add(tokens);
    }

    pub fn reset(&mut self) {
        self.used_tokens = 0;
    }

    pub fn should_compact(&self) -> bool {
        self.usage_ratio() >= self.compact_threshold
    }

    pub fn usage_ratio(&self) -> f64 {
        if self.max_tokens == 0 {
            return 0.0;
        }
        self.used_tokens as f64 / self.max_tokens as f64
    }

    pub fn remaining(&self) -> usize {
        self.max_tokens.saturating_sub(self.used_tokens)
    }

    pub fn max_tokens(&self) -> usize {
        self.max_tokens
    }
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self::new(200_000, 0.7)
    }
}

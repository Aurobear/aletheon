use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GovernedReviewSettings {
    pub enabled: bool,
    pub model: String,
    pub max_input_bytes: u64,
    pub max_output_bytes: u64,
    pub max_tokens: u64,
    pub max_tool_calls: u32,
    pub max_wall_time_seconds: u64,
    pub max_pending_jobs: usize,
    pub max_concurrent_jobs: usize,
}

impl Default for GovernedReviewSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            model: "default".into(),
            max_input_bytes: 262_144,
            max_output_bytes: 65_536,
            max_tokens: 16_384,
            max_tool_calls: 0,
            max_wall_time_seconds: 120,
            max_pending_jobs: 64,
            max_concurrent_jobs: 2,
        }
    }
}

impl GovernedReviewSettings {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.model.trim().is_empty() || self.model.len() > 256 {
            anyhow::bail!("governed_review.model must be 1..=256 bytes");
        }
        if self.max_tool_calls != 0 {
            anyhow::bail!("governed_review.max_tool_calls must remain zero");
        }
        let limits =
            crate::application::governed_review::GovernedReviewLimits {
                max_input_bytes: self.max_input_bytes,
                max_output_bytes: self.max_output_bytes,
                max_tokens: self.max_tokens,
                max_wall_time_ms: self.max_wall_time_seconds.checked_mul(1000).ok_or_else(
                    || anyhow::anyhow!("governed review wall time overflows milliseconds"),
                )?,
                max_pending_jobs: self.max_pending_jobs,
                max_concurrent_jobs: self.max_concurrent_jobs,
            };
        limits.validate()
    }

    pub fn limits(
        &self,
    ) -> anyhow::Result<crate::application::governed_review::GovernedReviewLimits> {
        self.validate()?;
        Ok(crate::application::governed_review::GovernedReviewLimits {
            max_input_bytes: self.max_input_bytes,
            max_output_bytes: self.max_output_bytes,
            max_tokens: self.max_tokens,
            max_wall_time_ms: self.max_wall_time_seconds * 1000,
            max_pending_jobs: self.max_pending_jobs,
            max_concurrent_jobs: self.max_concurrent_jobs,
        })
    }
}

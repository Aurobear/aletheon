//! Deployment configuration for channel adapters.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TelegramChannelConfig {
    #[serde(default)] pub enabled: bool,
    pub bot_token_env: Option<String>,
    pub owner_user_id: Option<i64>,
    #[serde(default = "default_poll_timeout_secs")] pub poll_timeout_secs: u64,
}

fn default_poll_timeout_secs() -> u64 { 10 }

impl Default for TelegramChannelConfig {
    fn default() -> Self { Self { enabled: false, bot_token_env: None, owner_user_id: None, poll_timeout_secs: 10 } }
}

impl TelegramChannelConfig {
    pub fn validate(&mut self) -> Vec<String> {
        self.poll_timeout_secs = self.poll_timeout_secs.clamp(1, 50);
        if !self.enabled { return vec![]; }
        let mut errors = vec![];
        if self.bot_token_env.as_ref().is_none_or(|name| name.trim().is_empty()) {
            errors.push("telegram.enabled=true but bot_token_env is not set".into());
        }
        if self.owner_user_id.is_none_or(|id| id <= 0) {
            errors.push("telegram.enabled=true but owner_user_id is not positive".into());
        }
        errors
    }
}

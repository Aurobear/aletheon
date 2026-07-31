//! Overload/backpressure configuration for the turn coordinator (D2-M5-T2).
//!
//! Controls how the daemon responds when too many concurrent turns are active.
//! Defaults are bounded for an always-on daemon.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Backpressure limits for turn admission.
///
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct BackpressureConfig {
    /// Maximum number of concurrent turns allowed across all connections.
    /// `None` explicitly disables the limit.
    pub max_concurrent_turns: Option<usize>,
    /// Maximum authenticated Unix socket connections per daemon.
    /// `None` explicitly disables the limit.
    pub max_connections: Option<usize>,
    /// Maximum logical SQLite bytes for the canonical event spine.
    /// `None` explicitly disables the size limit.
    pub max_event_spine_bytes: Option<u64>,
}

impl Default for BackpressureConfig {
    fn default() -> Self {
        Self {
            max_concurrent_turns: Some(8),
            max_connections: Some(64),
            max_event_spine_bytes: Some(1024 * 1024 * 1024),
        }
    }
}

impl BackpressureConfig {
    /// True if backpressure is active (a limit is set and the current count
    /// equals or exceeds it). When no limit is set, this always returns false.
    pub fn is_exceeded(&self, active_count: usize) -> bool {
        self.max_concurrent_turns
            .is_some_and(|limit| active_count >= limit)
    }

    /// Produce a human-readable error message for a rejected turn.
    pub fn overload_message(&self) -> String {
        match self.max_concurrent_turns {
            Some(limit) => format!(
                "server overloaded: {limit} concurrent turns already in-flight (limit: {limit})"
            ),
            None => "server overloaded".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_bounded() {
        let cfg = BackpressureConfig::default();
        assert!(!cfg.is_exceeded(0));
        assert!(cfg.is_exceeded(8));
        assert_eq!(cfg.max_connections, Some(64));
        assert_eq!(cfg.max_event_spine_bytes, Some(1024 * 1024 * 1024));
    }

    #[test]
    fn limited_rejects_when_full() {
        let cfg = BackpressureConfig {
            max_concurrent_turns: Some(2),
            max_connections: Some(4),
            max_event_spine_bytes: Some(1024),
        };
        assert!(!cfg.is_exceeded(0));
        assert!(!cfg.is_exceeded(1));
        assert!(cfg.is_exceeded(2));
        assert!(cfg.is_exceeded(3));
    }

    #[test]
    fn parses_from_toml() {
        let cfg: BackpressureConfig = toml::from_str("max_concurrent_turns = 5\n").unwrap();
        assert_eq!(cfg.max_concurrent_turns, Some(5));
    }

    #[test]
    fn empty_section_uses_safe_defaults() {
        let cfg: BackpressureConfig = toml::from_str("").unwrap();
        assert_eq!(cfg, BackpressureConfig::default());
    }
}

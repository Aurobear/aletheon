//! Overload/backpressure configuration for the turn coordinator (D2-M5-T2).
//!
//! Controls how the daemon responds when too many concurrent turns are active.
//! All defaults are permissive (no-op) for backward compatibility.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Backpressure limits for turn admission.
///
/// Default (`max_concurrent_turns = None`): unlimited, backward compatible.
/// Set `max_concurrent_turns = Some(N)` to reject new turns with a 503-like
/// error when N turns are already in-flight.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct BackpressureConfig {
    /// Maximum number of concurrent turns allowed across all connections.
    /// `None` means unlimited (default).
    #[serde(default)]
    pub max_concurrent_turns: Option<usize>,
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
                "server overloaded: {} concurrent turns already in-flight (limit: {limit})",
                limit
            ),
            None => "server overloaded".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_unlimited() {
        let cfg = BackpressureConfig::default();
        assert!(!cfg.is_exceeded(0));
        assert!(!cfg.is_exceeded(100));
    }

    #[test]
    fn limited_rejects_when_full() {
        let cfg = BackpressureConfig {
            max_concurrent_turns: Some(2),
        };
        assert!(!cfg.is_exceeded(0));
        assert!(!cfg.is_exceeded(1));
        assert!(cfg.is_exceeded(2));
        assert!(cfg.is_exceeded(3));
    }

    #[test]
    fn parses_from_toml() {
        let cfg: BackpressureConfig =
            toml::from_str("max_concurrent_turns = 5\n").unwrap();
        assert_eq!(cfg.max_concurrent_turns, Some(5));
    }

    #[test]
    fn empty_section_is_unlimited() {
        let cfg: BackpressureConfig = toml::from_str("").unwrap();
        assert!(cfg.max_concurrent_turns.is_none());
    }
}

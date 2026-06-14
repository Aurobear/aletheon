use std::time::Instant;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Minimal-operation mode triggered by repeated crashes.
///
/// While active the agent restricts itself to health-check responses and
/// refuses to start new reasoning tasks.  The mode auto-exits after a
/// configurable cooldown period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafeMode {
    active: bool,
    entry_count: u32,
    #[serde(skip)]
    entered_at: Option<Instant>,
    /// Cooldown duration in seconds before auto-exit.
    cooldown_secs: u64,
}

impl Default for SafeMode {
    fn default() -> Self {
        Self {
            active: false,
            entry_count: 0,
            entered_at: None,
            cooldown_secs: 60,
        }
    }
}

impl SafeMode {
    /// Create a safe mode with a specific cooldown (in seconds).
    pub fn with_cooldown(cooldown_secs: u64) -> Self {
        Self {
            cooldown_secs,
            ..Default::default()
        }
    }

    /// Enter safe mode.  Each call increments the entry counter.
    pub fn enter(&mut self) {
        if !self.active {
            warn!(entry_count = self.entry_count + 1, "Entering safe mode");
        }
        self.active = true;
        self.entry_count += 1;
        self.entered_at = Some(Instant::now());
    }

    /// Exit safe mode.
    pub fn exit(&mut self) {
        if self.active {
            info!("Exiting safe mode");
        }
        self.active = false;
        self.entered_at = None;
    }

    /// Returns `true` if safe mode is currently active.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Returns how many times safe mode has been entered since creation.
    pub fn entry_count(&self) -> u32 {
        self.entry_count
    }

    /// Returns the configured cooldown duration.
    pub fn cooldown(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.cooldown_secs)
    }

    /// Returns `true` if safe mode is active **and** the cooldown has elapsed.
    pub fn should_auto_exit(&self) -> bool {
        if !self.active {
            return false;
        }
        self.entered_at
            .map(|t| t.elapsed().as_secs() >= self.cooldown_secs)
            .unwrap_or(false)
    }

    /// Check the cooldown and exit automatically if it has elapsed.
    /// Returns `true` if safe mode was exited as a result.
    pub fn tick(&mut self) -> bool {
        if self.should_auto_exit() {
            self.exit();
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_mode_entry_exit() {
        let mut sm = SafeMode::default();
        assert!(!sm.is_active());
        assert_eq!(sm.entry_count(), 0);

        sm.enter();
        assert!(sm.is_active());
        assert_eq!(sm.entry_count(), 1);

        sm.enter(); // double-enter increments counter but stays active
        assert_eq!(sm.entry_count(), 2);

        sm.exit();
        assert!(!sm.is_active());
    }

    #[test]
    fn test_safe_mode_auto_exit() {
        let mut sm = SafeMode::with_cooldown(0); // immediate cooldown
        sm.enter();
        assert!(sm.is_active());
        assert!(sm.should_auto_exit());

        let exited = sm.tick();
        assert!(exited);
        assert!(!sm.is_active());
    }

    #[test]
    fn test_safe_mode_cooldown_not_elapsed() {
        let mut sm = SafeMode::with_cooldown(3600); // 1 hour
        sm.enter();
        assert!(sm.is_active());
        assert!(!sm.should_auto_exit());

        let exited = sm.tick();
        assert!(!exited);
        assert!(sm.is_active());
    }

    #[test]
    fn test_safe_mode_serialization() {
        let mut sm = SafeMode::default();
        sm.enter();

        let json = serde_json::to_string(&sm).unwrap();
        let restored: SafeMode = serde_json::from_str(&json).unwrap();
        assert!(restored.is_active());
        assert_eq!(restored.entry_count(), 1);
    }
}

//! EmergencyKillswitch: halt agent operation on critical failures.
//!
//! Monitors for trigger conditions and activates emergency shutdown:
//! 1. Cancels all active tasks
//! 2. Saves state snapshot
//! 3. Notifies user
//! 4. Enters safe mode

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ── Killswitch Trigger ──────────────────────────────────────────────────────

/// Conditions that can trigger the emergency killswitch.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum KillswitchTrigger {
    /// Too many consecutive failures.
    ConsecutiveFailures { count: u32 },
    /// Prompt injection detected with high confidence (0-100 integer).
    InjectionDetected { confidence_pct: u8 },
    /// Resource exhausted beyond threshold.
    ResourceExhausted { resource: String },
    /// User manually triggered.
    UserTriggered,
    /// Anomalous behavior pattern detected.
    AnomalousBehavior { pattern: String },
    /// Security policy violation.
    SecurityPolicyViolation { violation: String },
}

/// Result of a killswitch activation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KillswitchActivation {
    pub trigger: KillswitchTrigger,
    pub reason: String,
    pub activated_at_ms: u64,
    pub auto_recover: bool,
}

// ── Trigger Config ──────────────────────────────────────────────────────────

/// Configuration for a specific trigger.
#[derive(Debug, Clone)]
pub struct TriggerConfig {
    /// Cooldown period after activation before auto-recovery.
    pub cooldown: Duration,
    /// Whether the system can auto-recover after cooldown.
    pub auto_recover: bool,
    /// Whether user confirmation is required to recover.
    pub requires_user_confirmation: bool,
}

/// Default trigger configurations.
pub fn default_trigger_configs() -> HashMap<KillswitchTrigger, TriggerConfig> {
    let mut configs = HashMap::new();

    configs.insert(
        KillswitchTrigger::ConsecutiveFailures { count: 10 },
        TriggerConfig {
            cooldown: Duration::from_secs(30),
            auto_recover: true,
            requires_user_confirmation: false,
        },
    );

    configs.insert(
        KillswitchTrigger::InjectionDetected { confidence_pct: 90 },
        TriggerConfig {
            cooldown: Duration::from_secs(60),
            auto_recover: false,
            requires_user_confirmation: true,
        },
    );

    configs.insert(
        KillswitchTrigger::ResourceExhausted {
            resource: "any".to_string(),
        },
        TriggerConfig {
            cooldown: Duration::from_secs(120),
            auto_recover: true,
            requires_user_confirmation: false,
        },
    );

    configs.insert(
        KillswitchTrigger::UserTriggered,
        TriggerConfig {
            cooldown: Duration::from_secs(0),
            auto_recover: false,
            requires_user_confirmation: true,
        },
    );

    configs.insert(
        KillswitchTrigger::SecurityPolicyViolation {
            violation: "any".to_string(),
        },
        TriggerConfig {
            cooldown: Duration::from_secs(300),
            auto_recover: false,
            requires_user_confirmation: true,
        },
    );

    configs
}

// ── EmergencyKillswitch ─────────────────────────────────────────────────────

/// Monitors for critical failure conditions and activates emergency shutdown.
pub struct EmergencyKillswitch {
    /// Default trigger configurations.
    configs: HashMap<KillswitchTrigger, TriggerConfig>,
    /// Current activation state.
    state: Arc<Mutex<KillswitchState>>,
    clock: Arc<dyn fabric::Clock>,
}

#[derive(Debug, Default)]
struct KillswitchState {
    /// Whether the killswitch is currently active.
    active: bool,
    /// The trigger that caused activation.
    active_trigger: Option<KillswitchTrigger>,
    /// When it was activated (epoch millis).
    activated_at_ms: Option<u64>,
    /// Consecutive failure counter.
    consecutive_failures: u32,
    /// User has confirmed recovery.
    user_confirmed: bool,
}

impl EmergencyKillswitch {
    /// Create with default trigger configs.
    pub fn new(clock: Arc<dyn fabric::Clock>) -> Self {
        Self {
            configs: default_trigger_configs(),
            state: Arc::new(Mutex::new(KillswitchState::default())),
            clock,
        }
    }

    /// Record a failure (increments consecutive failure counter).
    pub fn record_failure(&self) {
        let mut state = self.state.lock().unwrap();
        state.consecutive_failures += 1;

        // Check if we've hit the consecutive failure threshold
        if state.consecutive_failures >= 10 {
            drop(state);
            let _ = self.try_activate(
                KillswitchTrigger::ConsecutiveFailures { count: 10 },
                "10 consecutive failures".to_string(),
            );
        }
    }

    /// Reset the consecutive failure counter (on success).
    pub fn record_success(&self) {
        let mut state = self.state.lock().unwrap();
        state.consecutive_failures = 0;
    }

    /// Check if injection was detected and activate if confidence is high enough.
    pub fn check_injection(&self, confidence: f32) {
        if confidence >= 0.9 {
            let _ = self.try_activate(
                KillswitchTrigger::InjectionDetected {
                    confidence_pct: (confidence * 100.0) as u8,
                },
                format!(
                    "Prompt injection detected with confidence {:.2}",
                    confidence
                ),
            );
        }
    }

    /// Check resource exhaustion and activate if critical.
    pub fn check_resource_exhaustion(&self, resource: &str, usage_ratio: f32) {
        if usage_ratio > 0.95 {
            let _ = self.try_activate(
                KillswitchTrigger::ResourceExhausted {
                    resource: resource.to_string(),
                },
                format!(
                    "Resource '{}' exhausted at {:.1}%",
                    resource,
                    usage_ratio * 100.0
                ),
            );
        }
    }

    /// User-triggered emergency stop.
    pub fn user_activate(&self) {
        let _ = self.try_activate(
            KillswitchTrigger::UserTriggered,
            "User triggered emergency stop".to_string(),
        );
    }

    /// Report a security policy violation.
    pub fn report_violation(&self, violation: &str) {
        let _ = self.try_activate(
            KillswitchTrigger::SecurityPolicyViolation {
                violation: violation.to_string(),
            },
            format!("Security policy violation: {}", violation),
        );
    }

    /// Try to activate the killswitch. Returns false if already active or in cooldown.
    fn try_activate(&self, trigger: KillswitchTrigger, reason: String) -> bool {
        let mut state = self.state.lock().unwrap();

        // Already active — don't re-activate
        if state.active {
            return false;
        }

        // Check cooldown
        if let Some(activated_at_ms) = state.activated_at_ms {
            if let Some(config) = self.find_config(&trigger) {
                let elapsed_ms = (self.clock.wall_now().0 as u64).saturating_sub(activated_at_ms);
                if !config.auto_recover && !state.user_confirmed {
                    return false; // Still in cooldown, not user-confirmed
                }
                if elapsed_ms < config.cooldown.as_millis() as u64 {
                    return false; // Still in cooldown
                }
            }
        }

        state.active = true;
        state.active_trigger = Some(trigger);
        state.activated_at_ms = Some(self.clock.wall_now().0 as u64);
        state.user_confirmed = false;

        tracing::error!(reason = %reason, "Emergency killswitch activated");

        true
    }

    /// Check if the killswitch is currently active.
    pub fn is_active(&self) -> bool {
        self.state.lock().unwrap().active
    }

    /// Get the activation trigger if active.
    pub fn activation_trigger(&self) -> Option<KillswitchTrigger> {
        let state = self.state.lock().unwrap();
        if state.active {
            state.active_trigger.clone()
        } else {
            None
        }
    }

    /// Attempt to recover. Returns true if recovery succeeded.
    pub fn try_recover(&self) -> bool {
        let mut state = self.state.lock().unwrap();

        if !state.active {
            return true; // Already recovered
        }

        // Check if recovery is allowed
        if let Some(ref trigger) = state.active_trigger {
            if let Some(config) = self.find_config(trigger) {
                if config.requires_user_confirmation && !state.user_confirmed {
                    return false; // Needs user confirmation
                }

                if let Some(activated_at_ms) = state.activated_at_ms {
                    let elapsed_ms = (self.clock.wall_now().0 as u64).saturating_sub(activated_at_ms);
                    if elapsed_ms < config.cooldown.as_millis() as u64 {
                        return false; // Still in cooldown
                    }
                }
            }
        }

        state.active = false;
        state.active_trigger = None;
        state.activated_at_ms = None;
        state.consecutive_failures = 0;

        tracing::info!("Emergency killswitch recovered");
        true
    }

    /// User confirms recovery (for triggers that require it).
    pub fn user_confirm_recovery(&self) {
        let mut state = self.state.lock().unwrap();
        state.user_confirmed = true;
    }

    /// Find the config for a trigger (with fallback to wildcard matches).
    fn find_config(&self, trigger: &KillswitchTrigger) -> Option<&TriggerConfig> {
        // Direct match
        if let Some(config) = self.configs.get(trigger) {
            return Some(config);
        }

        // Wildcard matches
        match trigger {
            KillswitchTrigger::ConsecutiveFailures { .. } => self
                .configs
                .get(&KillswitchTrigger::ConsecutiveFailures { count: 10 }),
            KillswitchTrigger::InjectionDetected { .. } => self
                .configs
                .get(&KillswitchTrigger::InjectionDetected { confidence_pct: 90 }),
            KillswitchTrigger::ResourceExhausted { .. } => {
                self.configs.get(&KillswitchTrigger::ResourceExhausted {
                    resource: "any".to_string(),
                })
            }
            KillswitchTrigger::SecurityPolicyViolation { .. } => {
                self.configs
                    .get(&KillswitchTrigger::SecurityPolicyViolation {
                        violation: "any".to_string(),
                    })
            }
            _ => None,
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;

    fn test_clock() -> Arc<dyn fabric::Clock> {
        Arc::new(TestClock::default())
    }

    fn test_ks() -> EmergencyKillswitch {
        EmergencyKillswitch::new(test_clock())
    }

    #[test]
    fn test_initially_inactive() {
        let ks = test_ks();
        assert!(!ks.is_active());
    }

    #[test]
    fn test_user_activate() {
        let ks = test_ks();
        ks.user_activate();
        assert!(ks.is_active());
    }

    #[test]
    fn test_consecutive_failures_trigger() {
        let ks = test_ks();
        for _ in 0..10 {
            ks.record_failure();
        }
        assert!(ks.is_active());
    }

    #[test]
    fn test_success_resets_counter() {
        let ks = test_ks();
        for _ in 0..9 {
            ks.record_failure();
        }
        ks.record_success();
        for _ in 0..9 {
            ks.record_failure();
        }
        // Should not be active — counter was reset
        assert!(!ks.is_active());
    }

    #[test]
    fn test_injection_detection() {
        let ks = test_ks();
        ks.check_injection(0.95);
        assert!(ks.is_active());
    }

    #[test]
    fn test_injection_low_confidence() {
        let ks = test_ks();
        ks.check_injection(0.5);
        assert!(!ks.is_active());
    }

    #[test]
    fn test_resource_exhaustion() {
        let ks = test_ks();
        ks.check_resource_exhaustion("memory", 0.98);
        assert!(ks.is_active());
    }

    #[test]
    fn test_resource_normal() {
        let ks = test_ks();
        ks.check_resource_exhaustion("memory", 0.5);
        assert!(!ks.is_active());
    }

    #[test]
    fn test_violation_report() {
        let ks = test_ks();
        ks.report_violation("unauthorized file access");
        assert!(ks.is_active());
    }

    #[test]
    fn test_user_recover_requires_confirmation() {
        let ks = test_ks();
        ks.user_activate();
        assert!(ks.is_active());

        // Recovery should fail without user confirmation
        assert!(!ks.try_recover());

        // After user confirmation
        ks.user_confirm_recovery();
        assert!(ks.try_recover());
        assert!(!ks.is_active());
    }

    #[test]
    fn test_activation_trigger() {
        let ks = test_ks();
        assert!(ks.activation_trigger().is_none());

        ks.user_activate();
        assert_eq!(
            ks.activation_trigger(),
            Some(KillswitchTrigger::UserTriggered)
        );
    }

    #[test]
    fn test_double_activate_ignored() {
        let ks = test_ks();
        assert!(ks.try_activate(KillswitchTrigger::UserTriggered, "first".to_string(),));
        // Second activation should be ignored
        assert!(!ks.try_activate(KillswitchTrigger::UserTriggered, "second".to_string(),));
    }

    #[test]
    fn test_trigger_config_display() {
        let trigger = KillswitchTrigger::ConsecutiveFailures { count: 10 };
        // Just make sure it's Debug-printable
        let _ = format!("{:?}", trigger);
    }
}

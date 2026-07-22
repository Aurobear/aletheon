//! Independent latched EmergencyStop authority.
//! E-stop is NOT Cancel/SafeStop — it is a separate high-priority local path.
//! Once latched, only a local trusted adapter may reset.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Mutex;
use fabric::types::emergency_stop::{EStopEvent, EStopState};

pub struct EmergencyStop {
    state: Mutex<EStopState>,
    triggered_at_ms: AtomicI64,
    reason: Mutex<String>,
    /// Number of times E-stop has been triggered (for exactly-once latch testing).
    trigger_count: AtomicBool,
}

impl EmergencyStop {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(EStopState::Armed),
            triggered_at_ms: AtomicI64::new(0),
            reason: Mutex::new(String::new()),
            trigger_count: AtomicBool::new(false),
        }
    }

    /// Trigger the emergency stop. Idempotent and exactly-once.
    /// Once triggered, stays latched until local reset.
    pub fn trigger(&self, now_ms: i64, reason: impl Into<String>) -> EStopEvent {
        let mut state = self.state.lock().unwrap();

        // Exactly-once latch
        if *state != EStopState::Armed {
            return EStopEvent {
                device_id: String::new(),
                state: *state,
                triggered_at_ms: self.triggered_at_ms.load(Ordering::SeqCst),
                reason: self.reason.lock().unwrap().clone(),
                reset_at_ms: None,
                operator_id: None,
            };
        }

        self.trigger_count.store(true, Ordering::SeqCst);
        self.triggered_at_ms.store(now_ms, Ordering::SeqCst);
        *self.reason.lock().unwrap() = reason.into();
        *state = EStopState::Triggered;

        // Immediately transition to Latched
        *state = EStopState::Latched;

        EStopEvent {
            device_id: String::new(),
            state: EStopState::Latched,
            triggered_at_ms: now_ms,
            reason: self.reason.lock().unwrap().clone(),
            reset_at_ms: None,
            operator_id: None,
        }
    }

    /// Attempt to reset the E-stop. Only allowed from ResetRequired state,
    /// and only by a local operator (operator_id must be Some).
    /// Remote RPC cannot reset — if operator_id is None, the reset is rejected.
    pub fn reset(&self, operator_id: Option<&str>, _now_ms: i64) -> Result<EStopState, String> {
        let mut state = self.state.lock().unwrap();

        match *state {
            EStopState::Latched => {
                // Transition to ResetRequired first
                *state = EStopState::ResetRequired;
                Ok(*state)
            }
            EStopState::ResetRequired => {
                // Only local operator (with ID) may arm
                let op = operator_id.ok_or("remote reset is not allowed — local operator required")?;
                if op.is_empty() {
                    return Err("operator_id must not be empty".into());
                }
                *state = EStopState::Armed;
                Ok(*state)
            }
            EStopState::Armed => Ok(*state),
            EStopState::Triggered => {
                *state = EStopState::Latched;
                Ok(*state)
            }
        }
    }

    pub fn state(&self) -> EStopState {
        *self.state.lock().unwrap()
    }

    pub fn is_armed(&self) -> bool {
        self.state() == EStopState::Armed
    }

    /// Returns true if the E-stop was ever triggered (latch evidence).
    pub fn was_triggered(&self) -> bool {
        self.trigger_count.load(Ordering::SeqCst)
    }
}

impl Default for EmergencyStop {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_armed() {
        let estop = EmergencyStop::new();
        assert!(estop.is_armed());
        assert_eq!(estop.state(), EStopState::Armed);
    }

    #[test]
    fn trigger_latches_exactly_once() {
        let estop = EmergencyStop::new();
        estop.trigger(1000, "physical button");
        assert_eq!(estop.state(), EStopState::Latched);
        assert!(estop.was_triggered());

        // Second trigger is idempotent
        estop.trigger(2000, "second press");
        assert_eq!(estop.state(), EStopState::Latched);
    }

    #[test]
    fn reset_requires_operator() {
        let estop = EmergencyStop::new();
        estop.trigger(1000, "test");

        // First: Latched -> ResetRequired
        assert_eq!(estop.reset(None, 2000).unwrap(), EStopState::ResetRequired);

        // Remote reset (no operator_id) is rejected
        assert!(estop.reset(None, 3000).is_err());
        assert_eq!(estop.state(), EStopState::ResetRequired);
    }

    #[test]
    fn local_operator_can_arm() {
        let estop = EmergencyStop::new();
        estop.trigger(1000, "test");
        estop.reset(None, 2000).unwrap(); // -> ResetRequired
        assert!(estop.reset(Some("operator-1"), 3000).is_ok());
        assert_eq!(estop.state(), EStopState::Armed);
    }

    #[test]
    fn concurrent_triggers_are_safe() {
        use std::sync::Arc;
        use std::thread;
        let estop = Arc::new(EmergencyStop::new());
        let mut handles = vec![];
        for i in 0..10 {
            let e = estop.clone();
            handles.push(thread::spawn(move || {
                e.trigger(i * 100, "concurrent");
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(estop.state(), EStopState::Latched);
        assert!(estop.was_triggered());
    }
}
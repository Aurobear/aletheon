//! Integration tests for emergency stop public API.

use hardware::EmergencyStop;
use fabric::types::emergency_stop::EStopState;

#[test]
fn default_is_armed() {
    let estop = EmergencyStop::default();
    assert_eq!(estop.state(), EStopState::Armed);
    assert!(estop.is_armed());
}

#[test]
fn trigger_transitions_to_latched() {
    let estop = EmergencyStop::new();
    let event = estop.trigger(1000, "integration test");
    assert_eq!(event.state, EStopState::Latched);
    assert_eq!(estop.state(), EStopState::Latched);
    assert!(estop.was_triggered());
}

#[test]
fn full_reset_cycle() {
    let estop = EmergencyStop::new();

    // Trigger -> Latched
    estop.trigger(1000, "test");
    assert_eq!(estop.state(), EStopState::Latched);

    // Latched -> ResetRequired (step 1, no operator needed)
    assert_eq!(estop.reset(None, 2000).unwrap(), EStopState::ResetRequired);

    // ResetRequired rejects None operator
    assert!(estop.reset(None, 3000).is_err());

    // ResetRequired -> Armed (step 2, requires operator)
    assert_eq!(estop.reset(Some("op-1"), 3000).unwrap(), EStopState::Armed);
    assert!(estop.is_armed());
}

#[test]
fn was_triggered_is_sticky() {
    let estop = EmergencyStop::new();
    assert!(!estop.was_triggered());

    estop.trigger(1000, "test");
    assert!(estop.was_triggered());

    // Reset through full cycle
    estop.reset(None, 2000).unwrap();
    estop.reset(Some("op-1"), 3000).unwrap();

    // Still reports as having been triggered
    assert!(estop.was_triggered());
}

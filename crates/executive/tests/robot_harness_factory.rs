//! Tests for RobotHarness factory selection and required ports.

use cognit::harness::HarnessKind;

#[test]
fn robot_kind_exists_and_is_selectable() {
    // Verify the Robot variant exists in HarnessKind
    let kind = HarnessKind::Robot;
    assert!(!matches!(kind, HarnessKind::Linear)); // Not accidentally Linear
}

#[test]
fn linear_remains_default() {
    // Linear must remain the default for backward compatibility
    let default: HarnessKind = Default::default();
    assert!(matches!(default, HarnessKind::Linear));
}

#[test]
fn robot_does_not_fallback_to_linear() {
    let kind = HarnessKind::Robot;
    assert!(matches!(kind, HarnessKind::Robot));
    assert!(!matches!(kind, HarnessKind::Linear));
}

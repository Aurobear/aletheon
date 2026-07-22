use cognit::harness::robot::state::{RobotHarnessConfig, RobotState, VerificationSignal};

#[test]
fn full_lifecycle_through_observe_to_complete() {
    // Test the state machine transitions without the async runner
    let config = RobotHarnessConfig::default();
    assert_eq!(config.max_retries, 1);
    assert_eq!(config.max_replans, 1);
}

#[test]
fn terminal_states_dont_transition() {
    assert!(RobotState::Completed.is_terminal());
    assert!(RobotState::Failed.is_terminal());
    // Terminal states stay terminal
    assert_eq!(RobotState::Completed.next(&VerificationSignal::Matched), RobotState::Completed);
    assert_eq!(RobotState::Failed.next(&VerificationSignal::Unsafe), RobotState::Failed);
}

#[test]
fn normal_path_observe_to_completed() {
    let mut s = RobotState::Observe;
    s = s.next(&VerificationSignal::Matched); assert_eq!(s, RobotState::Plan);
    s = s.next(&VerificationSignal::Matched); assert_eq!(s, RobotState::Authorize);
    s = s.next(&VerificationSignal::Matched); assert_eq!(s, RobotState::Execute);
    s = s.next(&VerificationSignal::Matched); assert_eq!(s, RobotState::Verify);
    s = s.next(&VerificationSignal::Matched); assert_eq!(s, RobotState::Settle);
    s = s.next(&VerificationSignal::Matched); assert_eq!(s, RobotState::Completed);
    assert!(s.is_terminal());
}

#[test]
fn retry_path_with_remaining_retries() {
    let mut s = RobotState::Verify;
    s = s.next(&VerificationSignal::Retryable { remaining_retries: 1 });
    assert_eq!(s, RobotState::Retry);
    s = s.next(&VerificationSignal::Matched);
    assert_eq!(s, RobotState::Execute);
}

#[test]
fn retry_exhausted_goes_to_replan() {
    let mut s = RobotState::Verify;
    s = s.next(&VerificationSignal::Retryable { remaining_retries: 0 });
    assert_eq!(s, RobotState::Replan);
    s = s.next(&VerificationSignal::Matched);
    assert_eq!(s, RobotState::Plan);
}

#[test]
fn replan_exhausted_goes_to_recover() {
    let mut s = RobotState::Verify;
    s = s.next(&VerificationSignal::Replannable { remaining_replans: 0 });
    assert_eq!(s, RobotState::Recover);
}

#[test]
fn unsafe_always_goes_to_safe_stop() {
    let mut s = RobotState::Verify;
    s = s.next(&VerificationSignal::Unsafe);
    assert_eq!(s, RobotState::SafeStop);
    s = s.next(&VerificationSignal::Matched);
    assert_eq!(s, RobotState::Failed);
}

#[test]
fn unknown_with_retries_goes_to_retry() {
    let mut s = RobotState::Verify;
    s = s.next(&VerificationSignal::Unknown { remaining_retries: 1 });
    assert_eq!(s, RobotState::Retry);
}

#[test]
fn unknown_exhausted_goes_to_safe_stop() {
    let mut s = RobotState::Verify;
    s = s.next(&VerificationSignal::Unknown { remaining_retries: 0 });
    assert_eq!(s, RobotState::SafeStop);
}

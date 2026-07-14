use aletheon_kernel::chronos::{SystemTimer, TestClock};
use fabric::{Clock, MonoDeadline, MonoTime, Timer};

#[test]
fn chronos_virtual_clock_deadline_is_deterministic() {
    let clock = TestClock::new(1_000, 10);
    let timer = SystemTimer;
    let deadline = MonoDeadline::after(MonoTime(10), 25);
    assert!(!timer.is_expired(clock.mono_now(), deadline));
    clock.advance(24);
    assert!(!timer.is_expired(clock.mono_now(), deadline));
    clock.advance(1);
    assert!(timer.is_expired(clock.mono_now(), deadline));
}

use aletheon_kernel::chronos::{TestClock, Timer};
use fabric::{MonoDeadline, MonoTime};

#[test]
fn chronos_virtual_clock_deadline_is_deterministic() {
    let clock = TestClock::new(1_000, 10);
    let deadline = MonoDeadline::after(MonoTime(10), 25);
    assert!(!Timer::is_expired(&clock, deadline));
    clock.advance(24);
    assert!(!Timer::is_expired(&clock, deadline));
    clock.advance(1);
    assert!(Timer::is_expired(&clock, deadline));
}

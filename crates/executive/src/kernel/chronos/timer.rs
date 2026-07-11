use fabric::{Clock, MonoDeadline};

/// Kernel timer helpers. Tests use `TestClock`, so deadline assertions do not sleep.
#[derive(Debug, Default, Clone, Copy)]
pub struct Timer;

impl Timer {
    pub fn is_expired(clock: &dyn Clock, deadline: MonoDeadline) -> bool {
        deadline.is_expired_at(clock.mono_now())
    }
}

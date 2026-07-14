//! Monotonic/wall-clock kernel time providers.

pub mod system_clock;
pub mod timer;

pub use system_clock::{SystemClock, TestClock};
pub use timer::{SystemTimer, TestTimer};

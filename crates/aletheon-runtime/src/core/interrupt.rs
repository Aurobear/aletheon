//! Interrupt handling for canceling streaming and in-flight operations.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use aletheon_abi::ui_event::InterruptReason;

/// Shared cancel flag for interrupting the ReAct loop.
#[derive(Debug, Clone)]
pub struct InterruptFlag {
    flag: Arc<AtomicBool>,
    reason: Arc<std::sync::Mutex<Option<InterruptReason>>>,
}

impl InterruptFlag {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            reason: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Request an interrupt.
    pub fn request(&self, reason: InterruptReason) {
        *self.reason.lock().unwrap() = Some(reason);
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Check if an interrupt has been requested.
    pub fn is_requested(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Take the interrupt reason (resets the flag).
    pub fn take_reason(&self) -> Option<InterruptReason> {
        if self.flag.swap(false, Ordering::SeqCst) {
            self.reason.lock().unwrap().take()
        } else {
            None
        }
    }

    /// Reset the flag (e.g., at the start of a new turn).
    pub fn reset(&self) {
        self.flag.store(false, Ordering::SeqCst);
        *self.reason.lock().unwrap() = None;
    }
}

impl Default for InterruptFlag {
    fn default() -> Self {
        Self::new()
    }
}

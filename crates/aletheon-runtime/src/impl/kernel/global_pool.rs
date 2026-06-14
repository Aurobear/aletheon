use aletheon_abi::agent::Pid;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Global token pool for system-wide token management.
///
/// Manages a fixed budget of tokens that can be claimed and released
/// by agents during a pulse cycle. Uses atomic operations for lock-free
/// concurrency.
pub struct GlobalTokenPool {
    total_budget: AtomicU32,
    allocated: AtomicU32,
    active: AtomicBool,
}

impl GlobalTokenPool {
    /// Creates a new GlobalTokenPool with the given total budget.
    pub fn new(total_budget: u32) -> Self {
        Self {
            total_budget: AtomicU32::new(total_budget),
            allocated: AtomicU32::new(0),
            active: AtomicBool::new(false),
        }
    }

    /// Begins a new pulse cycle.
    ///
    /// Resets the allocated counter and marks the pool as active.
    /// The `total` parameter sets the new total budget for this pulse.
    pub fn begin_pulse(&self, total: u32) {
        self.total_budget.store(total, Ordering::SeqCst);
        self.allocated.store(0, Ordering::SeqCst);
        self.active.store(true, Ordering::SeqCst);
    }

    /// Ends the current pulse cycle.
    ///
    /// Marks the pool as inactive. No new claims can be made until
    /// the next `begin_pulse` call.
    pub fn end_pulse(&self) {
        self.active.store(false, Ordering::SeqCst);
    }

    /// Attempts to claim tokens from the pool.
    ///
    /// Returns the number of tokens actually granted, which may be less
    /// than requested if insufficient tokens are available. Returns 0
    /// if the pool is inactive or no tokens are available.
    ///
    /// Uses atomic CAS loop to ensure thread-safe allocation.
    pub fn claim(&self, _pid: Pid, requested: u32, _priority: u8) -> u32 {
        if !self.active.load(Ordering::SeqCst) {
            return 0;
        }

        let total = self.total_budget.load(Ordering::SeqCst);

        loop {
            let current_allocated = self.allocated.load(Ordering::SeqCst);
            let available = total.saturating_sub(current_allocated);

            if available == 0 {
                return 0;
            }

            let granted = requested.min(available);
            let new_allocated = current_allocated + granted;

            match self.allocated.compare_exchange_weak(
                current_allocated,
                new_allocated,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return granted,
                Err(_) => continue,
            }
        }
    }

    /// Releases unused tokens back to the pool.
    ///
    /// Subtracts the given number of tokens from the allocated counter.
    /// The caller should only release tokens that were previously claimed
    /// but not used.
    pub fn release(&self, unused: u32) {
        self.allocated.fetch_sub(unused, Ordering::SeqCst);
    }

    /// Returns the total token budget.
    pub fn total(&self) -> u32 {
        self.total_budget.load(Ordering::SeqCst)
    }

    /// Returns the number of currently allocated tokens.
    pub fn allocated(&self) -> u32 {
        self.allocated.load(Ordering::SeqCst)
    }

    /// Returns the number of available (unallocated) tokens.
    pub fn available(&self) -> u32 {
        let total = self.total_budget.load(Ordering::SeqCst);
        let allocated = self.allocated.load(Ordering::SeqCst);
        total.saturating_sub(allocated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claim_basic() {
        let pool = GlobalTokenPool::new(1000);
        let pid = Pid::new();

        pool.begin_pulse(1000);
        let granted = pool.claim(pid, 500, 0);

        assert_eq!(granted, 500);
        assert_eq!(pool.allocated(), 500);
        assert_eq!(pool.available(), 500);
    }

    #[test]
    fn test_claim_exceeds_available() {
        let pool = GlobalTokenPool::new(1000);
        let pid1 = Pid::new();
        let pid2 = Pid::new();

        pool.begin_pulse(1000);

        let granted1 = pool.claim(pid1, 800, 0);
        assert_eq!(granted1, 800);

        let granted2 = pool.claim(pid2, 500, 0);
        assert_eq!(granted2, 200);
        assert_eq!(pool.allocated(), 1000);
        assert_eq!(pool.available(), 0);
    }

    #[test]
    fn test_claim_when_inactive() {
        let pool = GlobalTokenPool::new(1000);
        let pid = Pid::new();

        // Pool is inactive by default
        let granted = pool.claim(pid, 500, 0);
        assert_eq!(granted, 0);

        // Begin pulse and claim
        pool.begin_pulse(1000);
        let granted = pool.claim(pid, 500, 0);
        assert_eq!(granted, 500);

        // End pulse and try to claim again
        pool.end_pulse();
        let granted = pool.claim(pid, 500, 0);
        assert_eq!(granted, 0);
    }

    #[test]
    fn test_release() {
        let pool = GlobalTokenPool::new(1000);
        let pid = Pid::new();

        pool.begin_pulse(1000);
        let granted = pool.claim(pid, 800, 0);
        assert_eq!(granted, 800);
        assert_eq!(pool.available(), 200);

        pool.release(300);
        assert_eq!(pool.allocated(), 500);
        assert_eq!(pool.available(), 500);
    }

    #[test]
    fn test_pulse_cycle() {
        let pool = GlobalTokenPool::new(1000);
        let pid = Pid::new();

        // First pulse
        pool.begin_pulse(1000);
        pool.claim(pid, 600, 0);
        assert_eq!(pool.allocated(), 600);
        assert_eq!(pool.total(), 1000);

        // End pulse
        pool.end_pulse();
        assert!(!pool.active.load(Ordering::SeqCst));

        // Second pulse with different budget
        pool.begin_pulse(2000);
        assert_eq!(pool.allocated(), 0);
        assert_eq!(pool.total(), 2000);
        assert_eq!(pool.available(), 2000);
    }
}

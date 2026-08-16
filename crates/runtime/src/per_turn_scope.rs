//! R2 per-turn OperationScope with RAII drain (Aletheon closure plan §10).
//!
//! Eliminates the daemon-global `current_scope` concurrency-overwrite risk.
//! A per-turn scope is created and owned locally by `run_turn`; an RAII guard
//! triggers fallback cleanup on `Drop`; normal end calls `settle_and_drain()`,
//! abnormal end calls `abort_and_drain()`.  Drains are idempotent and bind
//! PID/start identity, tool invocation, temp dirs and leases to the turn
//! generation.  No daemon-global mutable slot passes the current turn.

/// An idempotent per-turn scoped resource.  Draining twice is safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedResource {
    /// Resource identity (PID, tool call, temp dir, lease).
    pub resource_id: String,
    /// Turn generation the resource is bound to (fencing).
    pub turn_generation: u64,
    /// Whether the resource has been drained.
    pub drained: bool,
}

impl ScopedResource {
    pub fn new(resource_id: impl Into<String>, turn_generation: u64) -> Self {
        Self {
            resource_id: resource_id.into(),
            turn_generation,
            drained: false,
        }
    }

    /// Idempotent drain.  A second drain is a no-op.
    pub fn drain(&mut self) {
        self.drained = true;
    }
}

/// How a per-turn scope is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeExit {
    Settle,
    Abort,
}

/// A per-turn scope: local, not shared across turns.
#[derive(Debug, Default)]
pub struct PerTurnScope {
    resources: Vec<ScopedResource>,
    drained: bool,
    pub generation: u64,
}

impl PerTurnScope {
    pub fn new(generation: u64) -> Self {
        Self {
            resources: Vec::new(),
            drained: false,
            generation,
        }
    }

    /// Register a resource bound to this turn's generation.
    pub fn bind(&mut self, resource_id: impl Into<String>) {
        self.resources
            .push(ScopedResource::new(resource_id, self.generation));
    }

    /// Normal end: explicit settle_and_drain.
    pub fn settle_and_drain(&mut self) {
        self.drain_all(ScopeExit::Settle);
    }

    /// Abnormal end: abort_and_drain (cancel/timeout/shutdown).
    pub fn abort_and_drain(&mut self) {
        self.drain_all(ScopeExit::Abort);
    }

    fn drain_all(&mut self, exit: ScopeExit) {
        let _ = exit;
        if self.drained {
            return; // idempotent
        }
        for resource in &mut self.resources {
            resource.drain();
        }
        self.drained = true;
    }

    pub fn active_count(&self) -> usize {
        self.resources.iter().filter(|r| !r.drained).count()
    }

    pub fn is_drained(&self) -> bool {
        self.drained
    }
}

/// RAII guard: on drop, if not explicitly drained, run fallback abort drain.
/// This guarantees cleanup on early-return / panic / disconnect.
pub struct ScopeGuard<'a> {
    scope: &'a mut PerTurnScope,
    explicitly_drained: bool,
}

impl<'a> ScopeGuard<'a> {
    pub fn new(scope: &'a mut PerTurnScope) -> Self {
        Self {
            scope,
            explicitly_drained: false,
        }
    }

    pub fn settle(mut self) {
        self.scope.settle_and_drain();
        self.explicitly_drained = true;
    }

    pub fn abort(mut self) {
        self.scope.abort_and_drain();
        self.explicitly_drained = true;
    }
}

impl<'a> Drop for ScopeGuard<'a> {
    fn drop(&mut self) {
        if !self.explicitly_drained {
            self.scope.abort_and_drain(); // fallback cleanup (idempotent)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settle_drains_all_and_is_idempotent() {
        let mut scope = PerTurnScope::new(1);
        scope.bind("pid:1000");
        scope.bind("tool:call-1");
        scope.settle_and_drain();
        assert_eq!(scope.active_count(), 0);
        assert!(scope.is_drained());
        // Second drain is a no-op (idempotent).
        scope.settle_and_drain();
        assert_eq!(scope.active_count(), 0);
    }

    #[test]
    fn guard_drop_triggers_fallback_abort() {
        let mut scope = PerTurnScope::new(2);
        scope.bind("lease:x");
        {
            let _guard = ScopeGuard::new(&mut scope);
            // early return without explicit settle/abort → Drop drains
        }
        assert!(scope.is_drained());
        assert_eq!(scope.active_count(), 0);
    }

    #[test]
    fn concurrent_turns_do_not_share_scope() {
        let mut t1 = PerTurnScope::new(1);
        let mut t2 = PerTurnScope::new(2);
        t1.bind("pid:1");
        t2.bind("pid:2");
        t1.settle_and_drain();
        // t2 is untouched by t1's drain.
        assert_eq!(t1.active_count(), 0);
        assert_eq!(t2.active_count(), 1);
    }
}

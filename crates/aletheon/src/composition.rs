//! CGP-01 aletheon composition skeleton (Agent Kernel V2).
//!
//! Explicit lifecycle phases for the future aletheon composition root:
//! `preflight` -> `open` -> `compose` -> `bootstrap` -> `serve` -> `drain`.
//!
//! Constraints honoured (CGP-01):
//! - no `ComponentGraph`/`ServiceBag` — every component is an explicit handle;
//! - `compose` does **no** I/O and **no** spawn (it only wires handles);
//! - config is normalized per owner, not a single Aletheon `AppConfig`;
//! - the legacy launcher is still the live path; this skeleton is a one-way
//!   additive seam (no official socket cutover).
//!
//! `compose` is deliberately sync and pure: it returns a typed
//! `ComposedRuntime` that `serve` may later run.  Building this skeleton does
//! not start a daemon, open a socket, or touch any writer.

/// Phase outcomes.  `serve`/`drain` are not implemented in this seam — the
/// legacy launcher remains authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LifecyclePhase {
    #[default]
    Preflight,
    Open,
    Compose,
    Bootstrap,
    Serve,
    Drain,
}

/// Per-owner normalized config handles.  The composition root never spreads a
/// single Aletheon `AppConfig`; each owner receives only its own slice.
#[derive(Debug, Clone, Default)]
pub struct OwnerConfig {
    pub owner: &'static str,
    pub data_dir: Option<std::path::PathBuf>,
}

/// Explicit component handle (CGP-01: no service bag).  Carries the owner
/// port the component implements, so `compose` can wire it without I/O.
#[derive(Debug, Clone)]
pub struct ComponentHandle {
    pub name: &'static str,
    pub owner: &'static str,
}

impl ComponentHandle {
    pub fn new(name: &'static str, owner: &'static str) -> Self {
        Self { name, owner }
    }
}

/// The result of a pure `compose`.  `serve` would run it; this seam only
/// holds the handles.
#[derive(Debug, Default)]
pub struct ComposedRuntime {
    pub components: Vec<ComponentHandle>,
}

/// The aletheon composition skeleton.  Built explicitly per phase; no global
/// component registry, no service bag.
#[derive(Debug, Default)]
pub struct CompositionSkeleton {
    pub phase: LifecyclePhase,
    pub components: Vec<ComponentHandle>,
}

impl CompositionSkeleton {
    pub fn new() -> Self {
        Self {
            phase: LifecyclePhase::Preflight,
            components: Vec::new(),
        }
    }

    /// Preflight: validate inputs without I/O.  Pure, fail-fast.
    pub fn preflight(&mut self, owners: &[OwnerConfig]) -> Result<(), String> {
        if owners.iter().any(|o| o.owner.trim().is_empty()) {
            return Err("owner name must not be empty".into());
        }
        self.phase = LifecyclePhase::Preflight;
        Ok(())
    }

    /// Open: resolve explicit component handles (no I/O; the caller passes
    /// already-open resources).
    pub fn open(&mut self, components: Vec<ComponentHandle>) {
        self.components = components;
        self.phase = LifecyclePhase::Open;
    }

    /// Compose: **pure wiring only** — no I/O, no spawn, no socket.  Returns a
    /// typed `ComposedRuntime` the caller may later serve.
    pub fn compose(&self) -> ComposedRuntime {
        // Explicitly no side effects here (CGP-01 acceptance: "compose 不
        // I/O/spawn").
        ComposedRuntime {
            components: self.components.clone(),
        }
    }

    /// Bootstrap/serve/drain are placeholders in this seam; the legacy
    /// launcher remains the live path.
    pub fn mark_ready(&mut self) {
        self.phase = LifecyclePhase::Bootstrap;
    }

    pub fn phase(&self) -> LifecyclePhase {
        self.phase
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preflight_rejects_empty_owner() {
        let mut skeleton = CompositionSkeleton::new();
        assert!(skeleton
            .preflight(&[OwnerConfig {
                owner: "",
                data_dir: None,
            }])
            .is_err());
    }

    #[test]
    fn compose_is_pure_and_returns_handles() {
        let mut skeleton = CompositionSkeleton::new();
        skeleton.open(vec![
            ComponentHandle::new("runtime", "runtime"),
            ComponentHandle::new("gateway", "gateway"),
        ]);
        let composed = skeleton.compose();
        assert_eq!(composed.components.len(), 2);
        // No global ComponentGraph/ServiceBag: only explicit handles.
        assert_eq!(skeleton.phase, LifecyclePhase::Open);
    }
}

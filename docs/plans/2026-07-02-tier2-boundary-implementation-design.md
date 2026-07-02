# Tier 2 Boundary Corrections -- Implementation Design

**Date:** 2026-07-02
**Status:** Design (design-only gate in effect)
**Source plan:** `docs/plans/2026-07-01-tier2-boundary-corrections-plan.md`
**Roadmap:** `docs/plans/2026-07-01-modules-roadmap-design.md`
**Branch:** `auro/feat/20260701-aletheon-governed-memory-design` (current; branch per sub-phase per repo policy)

---

## 1. Verified Ground Truth Table

Every claim from the source plan's ground truth table was opened, read, and
compared against the actual codebase at commit f87ea6f. Results:

| # | Claim | Anchor | Status | Detail |
|---|---|---|---|---|
| 1 | Package names: base, dasein, cognit, corpus, runtime, interact, memory, metacog | `crates/*/Cargo.toml` `[package] name` | **MATCH** | All 8 names confirmed |
| 2 | Confirmation decision inline in Self's `review()` | `crates/dasein/src/core/mod.rs:389-391` | **MATCH** | `if care_score > 0.8 { if ctx.permissions.max_level() < base::PermissionLevel::SystemChange { RequireConfirmation }}` at lines 389-398 |
| 3 | `review()` signature | `crates/dasein/src/core/mod.rs:346` | **MATCH** | `async fn review(&self, intent: &Intent, ctx: &Context) -> Result<Verdict>` |
| 4 | Self type is `SelfField`, built via `SelfField::new(config: SelfFieldConfig)` | `crates/dasein/src/core/mod.rs:106` | **MATCH** | `pub fn new(config: SelfFieldConfig) -> Self` at line 106; `SelfFieldConfig` defined at line 47 |
| 5 | `Verdict` lives in `base` | `crates/base/src/include/self_field.rs:16-32` | **MATCH** | `pub enum Verdict { Allow, AllowWithModification, Deny, RequireConfirmation, SandboxFirst, Delay }`; re-exported at `base/src/lib.rs:101` as `base::Verdict` |
| 6 | `Context` is a `base` type | `crates/base/src/types/context.rs:19-37` | **MATCH** | `pub struct Context { request_id, session_id, trace, permissions, working_dir, metadata }`; re-exported at `base/src/lib.rs:108` as `base::Context` |
| 7 | `dasein` does NOT depend on `runtime` | `crates/dasein/Cargo.toml` `[dependencies]` | **MATCH** | Deps: base, corpus, cognit, memory, tokio, parking_lot, serde, serde_json, anyhow, async-trait, uuid, chrono, tracing, glob, toml, dirs, regex, libc, rusqlite. No `runtime`. |
| 8 | Existing `review()` test harness | `crates/dasein/src/core/mod.rs:468-621` | **MATCH** | `default_config()`, `make_intent()`, `minimal_ctx()`, `make_ctx_with_perms()` at lines 468-560; 11+ review tests |
| 9 | Daemon entry `pub async fn run(...)` | `crates/runtime/src/impl/daemon/mod.rs:77-81` | **MATCH** | `pub async fn run(config_path: Option<PathBuf>, env_path: Option<PathBuf>, socket: PathBuf) -> Result<()>` |
| 10 | Only caller is the `aletheond` bin | `crates/runtime/src/bin/aletheond.rs:35` | **MATCH** | `runtime::r#impl::daemon::run(args.config, args.env, args.socket).await` |
| 11 | ENTIRE `cognit -> corpus/interact` coupling is 2 imports in 1 file | `crates/cognit/src/impl/grounding/vision.rs:7-8` | **MATCH** | `use interact::acix::grounding::{GroundingProvider, GroundingResult}; use corpus::drivers::driver::types::Image;` |
| 12 | `GroundingProvider`/`GroundingResult`/`MockGroundingProvider` defined in interact | `crates/interact/src/acix/grounding.rs` | **MATCH** | Full file: GroundingResult (lines 7-20), GroundingProvider trait (lines 45-55), MockGroundingProvider (lines 60-74) |
| 13 | `Image`/`Bounds` defined in corpus; `Image::to_base64_png` uses `png`+`base64` | `crates/corpus/src/drivers/driver/types.rs:92-97, 99-120, 135-141` | **MATCH** | `Image { width: u32, height: u32, data: Vec<u8> }` (92-97), `to_base64_png` (99-120), `Bounds { x: i32, y: i32, width: i32, height: i32 }` (135-141) |
| 14 | `png`/`base64` are corpus deps; `base` has neither | `crates/corpus/Cargo.toml:21-22`; `crates/base/Cargo.toml` | **MATCH** | `base64 = "0.22"`, `png = "0.17"` in corpus; absent from base |

**Drifts found (3):**

| # | Drift | Plan claim | Actual | Impact |
|---|---|---|---|---|
| D1 | `ToolDefinition` location | Plan says `base/src/types/tool.rs` | `ToolDefinition` is at `crates/base/src/types/llm_types.rs` re-exported at `base/src/lib.rs:112`. `tool.rs` contains the `Tool` trait. | Plan code referencing `ToolDefinition` is fine if using `base::ToolDefinition` re-export path. |
| D2 | SelfField construction site for Task 4 wiring | Plan says `verdict_handler.rs` | SelfField is constructed at `crates/runtime/src/impl/daemon/handler/mod.rs:364-368`, wrapped in `Arc<Mutex<SelfField>>`. `verdict_handler.rs` only imports Verdict/VerdictAction types. | Task 4 must wire at `daemon/handler/mod.rs:368` (after `SelfField::new`), not in `verdict_handler.rs`. Requires `.lock().await.set_permission_authority(...)` due to `Arc<Mutex<>>` wrapper. |
| D3 | `dasein` "lightweight" implication | Plan implies dasein keeps only necessary deps | dasein has rusqlite, libc, dirs, regex, glob (14 deps total) -- it does its own SQLite I/O. However, it correctly has NO `runtime` dep. | Material to the design only: tests must include deps for tokio, uuid, etc. No change needed. |

**No missing claims.** All 14 claims verified, 3 drifts characterized.

---

## 2. Architecture Overview

### 2.1 Current State (pre-Tier-2)

```
                    runtime
                   /  |\  \  \  \
                  /   | \  \  \  \
              dasein  |  |  |  |  \
              /  \    |  |  |  |   \
          corpus  cognit  |  |  metacog memory
            (png,base64)  |  |
                         /    \
                    corpus    interact
                  (Image,   (GroundingProvider,
                   Bounds)   GroundingResult)

KEY VIOLATIONS:
  A) dasein::review() makes permission decisions inline (should be in runtime)
  B) No RuntimeHost trait -- daemon::run() is the only entry point
  C) cognit depends on corpus (Image) and interact (GroundingProvider)
     -- Brain crate should only depend on base
```

### 2.2 Target State (after all 3 phases)

```
                              runtime
                             /  |  \  \  \
                            /   |   \  \  \
                        dasein  |    |  |  \
                        /       |    |  |   \
                    corpus   cognit  |  |    metacog
                                    |  |
                                    |  |
                                memory  (etc.)

                    RuntimeHost trait
                    +-- DaemonHost (wraps daemon::run)
                    +-- [future] SystemdHost, CliHost

                    PermissionAuthority trait (in base)
                    +-- PermissionManager (in runtime)


     base  (ABI crate -- no impl)
    ┌─────────────────────────────────────────────┐
    │  include/  types/  events/  ipc/  kernel/  │
    │  policy/                                    │
    │    permission_authority.rs  [NEW]           │
    │  types/                                     │
    │    vision.rs               [NEW]  Image,    │
    │                              Bounds, codec  │
    │    grounding.rs            [NEW]  Grounding │
    │                              Provider trait │
    └─────────────────────────────────────────────┘
           ^              ^              ^
           |              |              |
        cognit       corpus/           dasein
       (only        interact
        base!)     (re-export
                    from base)
```

---

## 3. Phase Ordering and Dependencies

```
Phase 2c (cognit inversion) ── independent ── no deps on 2a/2b
Phase 2a (PermissionManager) ── independent ── no deps on 2b/2c
Phase 2b (RuntimeHost) ── independent ── no deps on 2a/2c

Recommended order: 2c first → 2a → 2b
```

**Rationale for 2c-first:**
- Lowest risk (pure type relocation, zero behavior change)
- Highest structural payoff (proves the multi-repo extraction seam)
- Unblocks Tier 4 thinking immediately
- Re-exports keep backward compat; `cargo build --workspace` is the gate

**All 3 phases are fully parallelizable.** No shared files, no ordering
constraints, no data dependencies. A 3-person team could implement them
simultaneously on separate branches.

**Downstream dependency chain:**
- 2a → prerequisite for M-D (self-evolution gating), M-I (goal autonomous loop), Tier 3 (provider permissions)
- 2b → prerequisite for M-F (additional hosts)
- 2c → prerequisite for Tier 4 (crate extraction), M-B (plugin lifecycle trait in base)

---

## 4. Phase 2a -- PermissionManager (delegate confirmation from dasein to runtime)

### 4.1 Task Summary

| Task | Scope | Files |
|---|---|---|
| Task 1 | `PermissionAuthority` trait in `base` | create `base/src/policy/permission_authority.rs`, mod `base/src/policy/mod.rs`, mod `base/src/lib.rs` |
| Task 2 | `PermissionManager` in `runtime` | create `runtime/src/core/permission_manager.rs`, mod `runtime/src/core/mod.rs` |
| Task 3 | `SelfField` delegation + fallback | mod `dasein/src/core/mod.rs` |
| Task 4 | Wire manager onto daemon's SelfField | mod `runtime/src/impl/daemon/handler/mod.rs` |

### 4.2 Exact Code

#### Task 1: `PermissionAuthority` trait in `base/src/policy/`

**NEW FILE: `crates/base/src/policy/permission_authority.rs`**

```rust
//! Runtime-owned permission policy contract (Tier 2a).
//!
//! The Self layer (`dasein`) delegates the "does this action need user
//! confirmation / is it permitted" decision to whatever implements this
//! trait, so the *policy* lives in the Runtime while identity/care/boundary
//! judgment stays in Self.
//!
//! Returning `None` means "no opinion" -- the caller falls back to its
//! default behavior. This keeps the trait additive: an un-wired system
//! behaves exactly as before.

use crate::context::Context;
use crate::Verdict;

/// Decides permission verdicts on behalf of the Runtime.
///
/// # Design
///
/// - `None` return = "defer to caller's default rule"
/// - Object-safe (takes `&self`, `Send + Sync`)
/// - Lives in `base` so `dasein` can hold a reference without depending on `runtime`
pub trait PermissionAuthority: Send + Sync {
    /// Given the current context, care relevance of an action, and the action
    /// name, decide whether it should be confirmed or gated.
    ///
    /// Returns `None` to defer to the caller's inline fallback rule.
    fn confirmation_verdict(
        &self,
        ctx: &Context,
        care_score: f64,
        action: &str,
    ) -> Option<Verdict>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RiskLevel;
    use std::path::PathBuf;

    struct AlwaysConfirm;
    impl PermissionAuthority for AlwaysConfirm {
        fn confirmation_verdict(
            &self,
            _ctx: &Context,
            _care: f64,
            action: &str,
        ) -> Option<Verdict> {
            Some(Verdict::RequireConfirmation {
                reason: format!("policy requires confirmation for '{action}'"),
                risk_level: RiskLevel::Medium,
            })
        }
    }

    struct NeverOpinion;
    impl PermissionAuthority for NeverOpinion {
        fn confirmation_verdict(
            &self,
            _ctx: &Context,
            _care: f64,
            _action: &str,
        ) -> Option<Verdict> {
            None
        }
    }

    #[test]
    fn authority_can_return_a_verdict() {
        let ctx = Context::new("t", PathBuf::from("/tmp"));
        let v = AlwaysConfirm.confirmation_verdict(&ctx, 0.9, "system.reboot");
        assert!(matches!(
            v,
            Some(Verdict::RequireConfirmation { .. })
        ));
    }

    #[test]
    fn authority_can_defer_by_returning_none() {
        let ctx = Context::new("t", PathBuf::from("/tmp"));
        let v = NeverOpinion.confirmation_verdict(&ctx, 0.9, "ls");
        assert!(v.is_none());
    }
}
```

**MODIFY: `crates/base/src/policy/mod.rs`** (currently 3 lines)

Replace:
```rust
//! Execution policy engine.

pub mod execpolicy;
```
With:
```rust
//! Execution policy engine.

pub mod execpolicy;
pub mod permission_authority;
```

**MODIFY: `crates/base/src/lib.rs`** -- add after line 79 (`pub use policy::execpolicy;`):

At line 80, insert:
```rust
pub use policy::permission_authority;
```

Run validation: `cargo test -p base policy::permission_authority`


#### Task 2: `PermissionManager` in `runtime/src/core/`

**NEW FILE: `crates/runtime/src/core/permission_manager.rs`**

```rust
//! Runtime permission policy (Tier 2a).
//!
//! Owns the confirmation/whitelist/sandbox policy decisions that previously
//! lived inline in `dasein::review()`. Phase 1 reproduces the exact prior
//! rule; whitelist and sandbox-policy selection are future additions.
//!
//! Port of `dasein/src/core/mod.rs:389-398` (behavior-identical).

use base::context::Context;
use base::policy::permission_authority::PermissionAuthority;
use base::{PermissionLevel, RiskLevel, Verdict};

/// The Runtime's permission authority.
///
/// Currently ports the single inline rule from `dasein::review()`:
/// high care + insufficient permissions = RequireConfirmation.
/// Future: whitelist, sandbox-policy selection, per-action rules.
#[derive(Default, Clone)]
pub struct PermissionManager;

impl PermissionManager {
    pub fn new() -> Self {
        Self
    }
}

impl PermissionAuthority for PermissionManager {
    fn confirmation_verdict(
        &self,
        ctx: &Context,
        care_score: f64,
        action: &str,
    ) -> Option<Verdict> {
        // Exact port of crates/dasein/src/core/mod.rs:389-398.
        // Behavior-preserving: same threshold (0.8), same comparison
        // (max_level < SystemChange), same message format.
        if care_score > 0.8
            && ctx.permissions.max_level() < PermissionLevel::SystemChange
        {
            return Some(Verdict::RequireConfirmation {
                reason: format!(
                    "High care relevance ({care_score:.2}) with insufficient permissions for action '{action}'"
                ),
                risk_level: RiskLevel::Medium,
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::policy::permission_authority::PermissionAuthority;
    use std::path::PathBuf;

    #[test]
    fn high_care_insufficient_perms_requires_confirmation() {
        let mgr = PermissionManager::new();
        // Default Context has CapabilitySet::new() => max_level = ReadOnly
        let ctx = Context::new("t", PathBuf::from("/tmp"));
        let v = mgr.confirmation_verdict(&ctx, 0.9, "settings.update");
        assert!(matches!(
            v,
            Some(Verdict::RequireConfirmation { .. })
        ));
    }

    #[test]
    fn low_care_no_opinion() {
        let mgr = PermissionManager::new();
        let ctx = Context::new("t", PathBuf::from("/tmp"));
        assert!(mgr
            .confirmation_verdict(&ctx, 0.1, "ls")
            .is_none());
    }

    #[test]
    fn high_care_but_sufficient_perms_no_opinion() {
        let mgr = PermissionManager::new();
        // ctx with SystemChange-level capability
        use base::capability::{Capability, CapabilitySet};
        let mut perms = CapabilitySet::new();
        perms.add(Capability::new(
            "system.admin",
            base::PermissionLevel::SystemChange,
            "admin access",
        ));
        let mut ctx = Context::new("t", PathBuf::from("/tmp"));
        ctx.permissions = perms;
        assert!(mgr
            .confirmation_verdict(&ctx, 0.9, "settings.update")
            .is_none());
    }
}
```

**MODIFY: `crates/runtime/src/core/mod.rs`** -- add after line 9 (`pub mod orchestrator;`):

At line 10, insert:
```rust
pub mod permission_manager;
```

(Actual line number will shift; insert between `orchestrator` and `react_loop` declarations.)

Run validation: `cargo test -p runtime permission_manager`


#### Task 3: `SelfField` delegation + fallback

**MODIFY: `crates/dasein/src/core/mod.rs`**

**Step A: Add import** -- after line 24 (`use anyhow::Result;`):

```rust
use std::sync::Arc;
```

**Step B: Add field to struct** -- in the `pub struct SelfField {` block, after line 102 (the `dasein_event_tx` field):

Add after line 102, before the closing `}` at line 103:
```rust
    /// Optional Runtime permission authority. When set, `review()` delegates
    /// the confirmation verdict to it instead of using the inline rule.
    permission_authority: Option<Arc<dyn base::policy::permission_authority::PermissionAuthority>>,
```

**Step C: Initialize in `new()`** -- in `SelfField::new()`, after line 148 (`dasein,`) and before line 149 (`dasein_event_tx,`):

Add after line 148:
```rust
            permission_authority: None,
```

**Step D: Add setter** -- after the `boundary` accessor methods (after line 160, which ends the `boundary()` method):

Insert:
```rust
    /// Install the Runtime's permission authority.
    ///
    /// Without it, the inline fallback rule at `review()` lines 389-398
    /// is used (behavior-preserving). This is called by the Runtime
    /// daemon handler after constructing SelfField.
    pub fn set_permission_authority(
        &mut self,
        authority: Arc<dyn base::policy::permission_authority::PermissionAuthority>,
    ) {
        self.permission_authority = Some(authority);
    }
```

**Step E: Delegate at the permission stage** -- replace lines 387-408 (the current inline rule):

Current code (lines 387-408):
```rust
        // 5. Permission check -- if the action requires a capability the context doesn't have,
        //    require confirmation for high care scores.
        if care_score > 0.8 {
            // High care relevance -- check if context has sufficient permissions
            if ctx.permissions.max_level() < base::PermissionLevel::SystemChange {
                let verdict = Verdict::RequireConfirmation {
                    reason: format!(
                        "High care relevance ({:.2}) with insufficient permissions for action '{}'",
                        care_score, intent.action
                    ),
                    risk_level: RiskLevel::Medium,
                };
                self.narrative.record(
                    "permission_check",
                    "Insufficient permissions for high-care action",
                    Some(&intent.action),
                    &verdict,
                );
                return Ok(verdict);
            }
        }
```

Replace with:
```rust
        // 5. Permission check -- delegate to the Runtime authority if installed,
        //    otherwise fall back to the historical inline rule (behavior-preserving).
        if let Some(authority) = &self.permission_authority {
            if let Some(verdict) = authority.confirmation_verdict(ctx, care_score, &intent.action) {
                self.narrative.record(
                    "permission_check",
                    "Runtime permission authority required confirmation",
                    Some(&intent.action),
                    &verdict,
                );
                return Ok(verdict);
            }
        } else if care_score > 0.8 {
            // Fallback: historical inline rule (exact port, line-for-line).
            if ctx.permissions.max_level() < base::PermissionLevel::SystemChange {
                let verdict = Verdict::RequireConfirmation {
                    reason: format!(
                        "High care relevance ({:.2}) with insufficient permissions for action '{}'",
                        care_score, intent.action
                    ),
                    risk_level: RiskLevel::Medium,
                };
                self.narrative.record(
                    "permission_check",
                    "Insufficient permissions for high-care action",
                    Some(&intent.action),
                    &verdict,
                );
                return Ok(verdict);
            }
        }
```

**Step F: Add delegation test** -- in the tests module (after line 621), add:

```rust
    use base::policy::permission_authority::PermissionAuthority;
    use std::sync::Arc;

    struct StubAuthority;
    impl PermissionAuthority for StubAuthority {
        fn confirmation_verdict(
            &self,
            _ctx: &base::Context,
            _care: f64,
            action: &str,
        ) -> Option<Verdict> {
            Some(Verdict::RequireConfirmation {
                reason: format!("stub gate for {action}"),
                risk_level: RiskLevel::Medium,
            })
        }
    }

    #[tokio::test]
    async fn review_delegates_permission_verdict_to_authority() {
        let mut sf = SelfField::new(default_config());
        sf.set_permission_authority(Arc::new(StubAuthority));
        let intent = make_intent("settings.update", "update a setting");
        let ctx = minimal_ctx();
        let verdict = sf.review(&intent, &ctx).await.unwrap();
        assert!(
            matches!(verdict, Verdict::RequireConfirmation { .. }),
            "authority verdict must be honored, got {verdict:?}"
        );
    }

    #[tokio::test]
    async fn review_falls_back_to_inline_when_no_authority_installed() {
        let sf = SelfField::new(default_config());
        // Low-care action -- shouldn't trigger confirmation
        let intent = make_intent("ls", "list files");
        let ctx = minimal_ctx();
        let verdict = sf.review(&intent, &ctx).await.unwrap();
        assert!(
            matches!(verdict, Verdict::Allow),
            "no authority + low care = allow, got {verdict:?}"
        );
    }
```

Run validation:
```bash
cargo test -p dasein review_delegates_permission_verdict_to_authority
cargo test -p dasein review_falls_back_to_inline_when_no_authority_installed
cargo test -p dasein  # all existing review_* tests must still pass
```


#### Task 4: Wire manager onto daemon's SelfField

**CRITICAL DRIFT from plan:** SelfField is constructed at
`crates/runtime/src/impl/daemon/handler/mod.rs:364-368`, NOT in
`verdict_handler.rs`. Plan incorrectly identified the wiring site.
The SelfField is inside `Arc<Mutex<SelfField>>`, requiring a lock
before calling `set_permission_authority`.

**MODIFY: `crates/runtime/src/impl/daemon/handler/mod.rs`**

After line 368 (`let self_field = Arc::new(Mutex::new(SelfField::new(self_field_config)));`):

Insert:
```rust
        // Tier 2a: install the Runtime PermissionManager as the permission authority.
        // This delegates the confirmation verdict from dasein's inline rule to the
        // Runtime's policy manager (behavior-identical port).
        {
            use crate::core::permission_manager::PermissionManager;
            let mut sf = self_field.lock().await;
            sf.set_permission_authority(std::sync::Arc::new(PermissionManager::new()));
        }
```

Run validation:
```bash
cargo build --workspace
# Manual smoke: aletheond starts, high-care SystemChange actions trigger
# confirmation, read-only actions pass without prompting.
```

---

## 5. Phase 2b -- RuntimeHost Trait (daemon becomes one host)

### 5.1 Task Summary

| Task | Scope | Files |
|---|---|---|
| Task 5 | `RuntimeHost` trait + `DaemonHost` | create `runtime/src/host/mod.rs`, mod `runtime/src/lib.rs`, mod `runtime/src/bin/aletheond.rs` |

### 5.2 Exact Code

**NEW FILE: `crates/runtime/src/host/mod.rs`**

```rust
//! Host abstraction (Tier 2b).
//!
//! A `RuntimeHost` is a deployment form of the runtime. `DaemonHost` is the
//! Unix-socket daemon. Additional hosts (systemd, container, CLI-one-shot) are
//! M-F, built on this trait.
//!
//! # Design
//!
//! - `init`: prepare resources (socket dirs, PID files, etc.)
//! - `serve`: run to completion (blocking on the host's event loop)
//! - `shutdown`: release resources
//! - Object-safe: `serve` takes `self: Box<Self>` for ownership transfer
//! - Phase 1 wraps `daemon::run` unchanged -- no protocol or behavior change

use anyhow::Result;
use std::path::PathBuf;

/// A deployment host for the runtime.
#[async_trait::async_trait]
pub trait RuntimeHost: Send {
    /// Prepare resources before serving. Called once at startup.
    async fn init(&mut self) -> Result<()>;

    /// Run the host's event loop to completion. Takes ownership.
    async fn serve(self: Box<Self>) -> Result<()>;

    /// Release resources. Called during graceful shutdown.
    async fn shutdown(&mut self) -> Result<()>;
}

/// The Unix-socket daemon host.
///
/// Wraps today's `daemon::run` unchanged. Socket-dir creation, PID-file
/// writing, and signal handling remain inside `daemon::run` for Phase 1
/// (behavior-identical). They migrate into `init()`/`shutdown()` when a
/// second host needs them (M-F).
pub struct DaemonHost {
    config_path: Option<PathBuf>,
    env_path: Option<PathBuf>,
    socket: PathBuf,
}

impl DaemonHost {
    pub fn new(
        config_path: Option<PathBuf>,
        env_path: Option<PathBuf>,
        socket: PathBuf,
    ) -> Self {
        Self {
            config_path,
            env_path,
            socket,
        }
    }
}

#[async_trait::async_trait]
impl RuntimeHost for DaemonHost {
    async fn init(&mut self) -> Result<()> {
        // Socket dir creation etc. currently happens inside daemon::run.
        // Keep it there for Phase 1 so behavior is identical.
        // This hook exists for future hosts (M-F).
        Ok(())
    }

    async fn serve(self: Box<Self>) -> Result<()> {
        // Delegate to the existing, unchanged daemon entry point.
        // Path matches the existing import in bin/aletheond.rs:
        //   runtime::r#impl::daemon::run(...)
        crate::r#impl::daemon::run(
            self.config_path,
            self.env_path,
            self.socket,
        )
        .await
    }

    async fn shutdown(&mut self) -> Result<()> {
        // PID-file cleanup currently tracked inside daemon::run.
        // Placeholder for future hosts (M-F).
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct CountingHost {
        inited: Arc<AtomicUsize>,
        shut: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl RuntimeHost for CountingHost {
        async fn init(&mut self) -> Result<()> {
            self.inited.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn serve(self: Box<Self>) -> Result<()> {
            Ok(())
        }
        async fn shutdown(&mut self) -> Result<()> {
            self.shut.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn host_lifecycle_is_drivable() {
        let inited = Arc::new(AtomicUsize::new(0));
        let shut = Arc::new(AtomicUsize::new(0));
        let mut host = CountingHost {
            inited: inited.clone(),
            shut: shut.clone(),
        };
        host.init().await.unwrap();
        host.shutdown().await.unwrap();
        assert_eq!(inited.load(Ordering::SeqCst), 1);
        assert_eq!(shut.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn daemon_host_has_zero_init_shutdown_cost() {
        // init/shutdown are no-ops for DaemonHost in Phase 1.
        let mut host = DaemonHost::new(
            None,
            None,
            PathBuf::from("/tmp/test.sock"),
        );
        host.init().await.unwrap();
        host.shutdown().await.unwrap();
    }
}
```

**MODIFY: `crates/runtime/src/lib.rs`** -- add after line 4 (`pub mod tools;`):

Insert:
```rust
pub mod host;
```

**MODIFY: `crates/runtime/src/bin/aletheond.rs`** -- replace the direct daemon call:

Current (line 35):
```rust
    runtime::r#impl::daemon::run(args.config, args.env, args.socket).await
```

Replace with:
```rust
    let mut host = runtime::host::DaemonHost::new(args.config, args.env, args.socket);
    host.init().await?;
    Box::new(host).serve().await
```

Run validation:
```bash
cargo test -p runtime host
cargo build --workspace
# Manual smoke: aletheond starts/stops/serves over socket identically.
```

---

## 6. Phase 2c -- Break cognit -> corpus/interact Inversion

### 6.1 Task Summary

| Task | Scope | Files |
|---|---|---|
| Task 6 | Move `Image`/`Bounds` (+ codec) into `base` | create `base/src/types/vision.rs`, mod `base/Cargo.toml`, mod `base/src/types/mod.rs`, mod `base/src/lib.rs`, mod `corpus/src/drivers/driver/types.rs` |
| Task 7 | Move grounding trait/types into `base`; sever cognit's corpus/interact deps | create `base/src/types/grounding.rs`, mod `base/src/types/mod.rs`, mod `base/src/lib.rs`, mod `interact/src/acix/grounding.rs`, mod `cognit/src/impl/grounding/vision.rs`, mod `cognit/Cargo.toml` |

### 6.2 Exact Code

#### Task 6: Move `Image`/`Bounds` into `base`

**NEW FILE: `crates/base/src/types/vision.rs`**

Copy verbatim from `crates/corpus/src/drivers/driver/types.rs:92-120` and `:135-141`:

```rust
//! Vision types -- image representation and codec.
//!
//! Moved from `corpus::drivers::driver::types` (Tier 2c) so that `cognit`
//! (Brain) can depend only on `base`, not on `corpus` (Body).

use serde::{Deserialize, Serialize};

/// RGB image in row-major format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGB bytes, row-major
}

/// Screen bounding box.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Image {
    /// Convert raw RGB image data to base64-encoded PNG.
    /// Returns (media_type, base64_data) suitable for LLM vision APIs.
    pub fn to_base64_png(&self) -> anyhow::Result<(String, String)> {
        use std::io::Cursor;

        let mut png_buf = Vec::new();
        {
            let mut cursor = Cursor::new(&mut png_buf);
            let mut encoder = png::Encoder::new(&mut cursor, self.width, self.height);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&self.data)?;
            writer.finish()?;
        }

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png_buf);
        Ok(("image/png".to_string(), b64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_encodes_to_base64_png() {
        let img = Image {
            width: 2,
            height: 2,
            data: vec![0u8; 2 * 2 * 3],
        };
        let (media_type, b64) = img.to_base64_png().unwrap();
        assert_eq!(media_type, "image/png");
        assert!(!b64.is_empty());
    }

    #[test]
    fn bounds_construction() {
        let b = Bounds {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        assert_eq!(b.width, 10);
        assert_eq!(b.height, 10);
    }
}
```

**MODIFY: `crates/base/Cargo.toml`** -- add after line 25 (`dashmap = "6"`):

```toml
png = "0.17"
base64 = "0.22"
```

**MODIFY: `crates/base/src/types/mod.rs`** -- add after line 12 (`pub mod resource;`):

```rust
pub mod vision;
```

**MODIFY: `crates/base/src/lib.rs`** -- add after line 57 (`pub use types::tool;`):

```rust
pub use types::vision;
```

**MODIFY: `crates/corpus/src/drivers/driver/types.rs`** -- replace the moved definitions.

Replace lines 91-120 (Image struct + impl) and lines 135-141 (Bounds struct) with:

```rust
// Re-exported from base (Tier 2c) for backward compatibility.
pub use base::types::vision::{Bounds, Image};
```

Run validation:
```bash
cargo test -p base types::vision
cargo build -p corpus  # corpus re-exports must resolve
```


#### Task 7: Move grounding trait/types into `base`; sever cognit deps

**NEW FILE: `crates/base/src/types/grounding.rs`**

Copy verbatim from `crates/interact/src/acix/grounding.rs`, with import path changed:

```rust
//! Visual grounding types and trait.
//!
//! Moved from `interact::acix::grounding` (Tier 2c) so that `cognit` (Brain)
//! can depend only on `base`, not on `interact` (Interface).

use crate::types::vision::{Bounds, Image};
use anyhow::Result;
use async_trait::async_trait;

/// Result of a visual grounding operation.
#[derive(Debug, Clone)]
pub struct GroundingResult {
    /// X coordinate of the element center
    pub x: i32,
    /// Y coordinate of the element center
    pub y: i32,
    /// Width of the bounding box
    pub width: i32,
    /// Height of the bounding box
    pub height: i32,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,
    /// Human-readable label of what was found
    pub label: String,
}

impl GroundingResult {
    /// Get the bounding box as a Bounds struct
    pub fn bounds(&self) -> Bounds {
        Bounds {
            x: self.x - self.width / 2,
            y: self.y - self.height / 2,
            width: self.width,
            height: self.height,
        }
    }

    /// Get the center point
    pub fn center(&self) -> (i32, i32) {
        (self.x, self.y)
    }
}

/// Provider for visual grounding -- locating UI elements by natural language description.
///
/// Implemented by the runtime layer to forward to a vision-capable LLM provider.
#[async_trait]
pub trait GroundingProvider: Send + Sync {
    /// Locate an element in the given screenshot by natural language description.
    async fn locate(&self, image: &Image, description: &str) -> Result<GroundingResult>;

    /// Locate multiple elements matching the description.
    /// Default implementation returns a single result.
    async fn locate_all(
        &self,
        image: &Image,
        description: &str,
    ) -> Result<Vec<GroundingResult>> {
        let result = self.locate(image, description).await?;
        Ok(vec![result])
    }
}

/// Mock grounding provider for testing.
///
/// Returns the center of the image with confidence 0.0 and label "mock".
pub struct MockGroundingProvider;

#[async_trait]
impl GroundingProvider for MockGroundingProvider {
    async fn locate(
        &self,
        image: &Image,
        _description: &str,
    ) -> Result<GroundingResult> {
        Ok(GroundingResult {
            x: (image.width / 2) as i32,
            y: (image.height / 2) as i32,
            width: 100,
            height: 50,
            confidence: 0.0,
            label: "mock".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_grounding_center() {
        let provider = MockGroundingProvider;
        let image = Image {
            width: 1920,
            height: 1080,
            data: vec![0u8; 1920 * 1080 * 3],
        };
        let result = provider.locate(&image, "anything").await.unwrap();
        assert_eq!(result.x, 960);
        assert_eq!(result.y, 540);
        assert_eq!(result.confidence, 0.0);
        assert_eq!(result.label, "mock");
    }

    #[tokio::test]
    async fn test_grounding_result_bounds() {
        let result = GroundingResult {
            x: 500,
            y: 300,
            width: 120,
            height: 40,
            confidence: 0.9,
            label: "button".to_string(),
        };
        let bounds = result.bounds();
        assert_eq!(bounds.x, 440);
        assert_eq!(bounds.y, 280);
        assert_eq!(bounds.width, 120);
        assert_eq!(bounds.height, 40);
        assert_eq!(result.center(), (500, 300));
    }
}
```

**MODIFY: `crates/base/src/types/mod.rs`** -- add after `pub mod vision;`:

```rust
pub mod grounding;
```

**MODIFY: `crates/base/src/lib.rs`** -- add after `pub use types::vision;`:

```rust
pub use types::grounding;
```

**MODIFY: `crates/interact/src/acix/grounding.rs`** -- replace ENTIRE file content with re-exports:

```rust
// Re-exported from base (Tier 2c) for backward compatibility.
pub use base::types::grounding::{GroundingProvider, GroundingResult, MockGroundingProvider};
```

**MODIFY: `crates/cognit/src/impl/grounding/vision.rs`** -- replace lines 7-8:

Replace:
```rust
use interact::acix::grounding::{GroundingProvider, GroundingResult};
use corpus::drivers::driver::types::Image;
```

With:
```rust
use base::types::grounding::{GroundingProvider, GroundingResult};
use base::types::vision::Image;
```

**MODIFY: `crates/cognit/Cargo.toml`** -- remove lines 10-11:

Remove:
```toml
corpus = { path = "../corpus", features = ["input", "display", "a11y", "ocr"] }
interact = { path = "../interact" }
```

(The `base` dep at line 9 stays. No other lines change.)

Run validation:
```bash
cargo test -p base types::grounding
cargo test -p cognit # all cognit tests must pass with only base dep
cargo build --workspace

# Prove the inversion is gone:
cargo tree -p cognit -e normal --prefix none | grep -E '^(corpus|interact) ' && echo "STILL COUPLED (bad)" || echo "OK: cognit depends only on base"
```

---

## 7. How Downstream Modules Consume These Changes

### 7.1 M-D: Self-Evolution Gating (needs 2a)

```rust
// In runtime/src/core/orchestrator.rs (post-task evolution hook):
use crate::core::permission_manager::PermissionManager;
use base::policy::permission_authority::PermissionAuthority;

// Before allowing a self-mutation to proceed, check through the authority:
let manager = PermissionManager::new();
let verdict = manager.confirmation_verdict(&ctx, care_score, "self.evolve");
match verdict {
    Some(Verdict::RequireConfirmation { .. }) => {
        // Gate the evolution -- require user confirmation
    }
    Some(Verdict::Deny { reason }) => {
        // Block the evolution
    }
    None => {
        // Proceed -- policy has no opinion
    }
}
```

### 7.2 M-I: Goal Autonomous Loop (needs 2a)

```rust
// In runtime/src/core/goal/loop.rs (proactive goal step):
use base::policy::permission_authority::PermissionAuthority;

// Before executing a goal-driven action autonomously:
if let Some(authority) = &self.permission_authority {
    if let Some(verdict) = authority.confirmation_verdict(
        &ctx, care_score, &goal_action
    ) {
        // Gate the autonomous action
        return handle_verdict(verdict);
    }
}
```

### 7.3 Tier 3: Provider Permissions (needs 2a)

```rust
// In cognit/src/impl/llm/scheduler.rs:
// PermissionManager can be queried for provider-level policy:
// e.g., "which providers are allowed for this care level?"
// This builds on the same PermissionAuthority trait.
```

### 7.4 Tier 4: Multi-Repo Extraction (needs 2c)

```rust
// After 2c, cognit's dependency graph is:
//   cognit -> base (only!)
// This means cognit can be extracted to a separate repo:
//   [dependencies]
//   aletheon-base = { git = "..." }
//
// The extraction is now a packaging exercise, not a refactor.
```

### 7.5 M-B: Plugin Lifecycle Trait (needs 2c pattern)

```rust
// In base/src/include/plugin.rs (new, following 2c pattern):
#[async_trait]
pub trait Plugin: Send + Sync {
    async fn init(&mut self) -> Result<()>;
    async fn run(&mut self) -> Result<()>;
    async fn shutdown(&mut self) -> Result<()>;
}

// In runtime/src/impl/plugin/manager.rs:
// PluginManager holds Vec<Box<dyn Plugin>> -- the trait lives in base,
// impls live in runtime/corpus/interact. Same inversion pattern as 2c.
```

### 7.6 M-F: Additional Hosts (needs 2b)

```rust
// In runtime/src/host/systemd.rs (new, M-F):
pub struct SystemdHost { /* ... */ }

#[async_trait::async_trait]
impl RuntimeHost for SystemdHost {
    async fn init(&mut self) -> Result<()> { /* sd_notify READY=1 */ }
    async fn serve(self: Box<Self>) -> Result<()> { /* daemon::run */ }
    async fn shutdown(&mut self) -> Result<()> { /* sd_notify STOPPING=1 */ }
}
```

---

## 8. Rollback Plan Per Phase

### 8.1 Rollback 2a (PermissionManager)

```
git revert <commit-hash-task4>  # wire
git revert <commit-hash-task3>  # SelfField delegation
git revert <commit-hash-task2>  # PermissionManager
git revert <commit-hash-task1>  # PermissionAuthority trait
```

Rollback safety: SelfField keeps the inline fallback. Reverting Task 3 restores
the original code exactly. No data migration. No protocol change.

### 8.2 Rollback 2b (RuntimeHost)

```
git revert <commit-hash-task5>  # DaemonHost + bin rewire
```

Rollback safety: The bin change is a thin wrapper. Reverting restores the direct
`daemon::run` call. No protocol change.

### 8.3 Rollback 2c (cognit inversion)

```
git revert <commit-hash-task7>  # grounding move + cognit dep drop
git revert <commit-hash-task6>  # Image/Bounds move
```

Rollback safety: Re-exports in `corpus` and `interact` keep all existing import
paths valid. Reverting restores the original definitions. `base` gains `png` and
`base64` deps -- these remain if 2c is partially reverted, but since `base/src/types/vision.rs`
is the only consumer, no compilation issue arises.

---

## 9. Risk Assessment

### 9.1 PermissionManager Reject Behavior

**Risk:** The delegation path could silently skip the permission check if the
authority returns `None` when it should have returned a verdict.

**Mitigation in design:**
- `PermissionManager` ports the exact inline rule line-for-line
- The `else if` fallback (lines 389-398 original) is preserved verbatim
- Two test paths guard this: `review_delegates_permission_verdict_to_authority`
  (authority present, verdict returned) and existing `review_*` tests (no
  authority, fallback exercised)
- The `None` return in `PermissionAuthority` is semantically "no opinion" --
  this matches the trait contract and the fallback path

### 9.2 DaemonHost Refactor Safety

**Risk:** The `Box::new(host).serve()` call changes the ownership model from a
direct function call.

**Mitigation in design:**
- `DaemonHost::serve` is a direct delegation to `daemon::run` -- zero logic change
- `init()` and `shutdown()` are no-ops -- no behavior change
- The `aletheond` bin is the ONLY caller of `daemon::run` (verified at
  `crates/runtime/src/bin/aletheond.rs:35`) -- no other callers to break
- `Box<Self>` is a standard async-trait ownership pattern

### 9.3 Moving Types Without Breaking Semver

**Risk:** Moving `Image`/`Bounds`/`GroundingProvider`/`GroundingResult` could
break downstream consumers that import from the old paths.

**Mitigation in design:**
- ALL old paths are preserved via `pub use` re-exports:
  - `corpus::drivers::driver::types::{Image, Bounds}` -> `pub use base::types::vision::{Bounds, Image}`
  - `interact::acix::grounding::{GroundingProvider, GroundingResult, MockGroundingProvider}` -> `pub use base::types::grounding::{...}`
- `cargo build --workspace` gate proves ALL consumers still compile
- `corpus` already depends on `base` -- the re-export adds zero new deps
- `interact` already depends on `base` -- same

### 9.4 SelfField Arc<Mutex<>> Wrapping (DRIFT D2)

**Risk:** The plan assumed `SelfField` is directly owned, but it is behind
`Arc<Mutex<SelfField>>` in the daemon handler. Calling `set_permission_authority`
requires `.lock().await`, which is async.

**Mitigation in design:**
- Task 4 code explicitly locks before calling the setter: `let mut sf = self_field.lock().await;`
- The lock scope is minimal (just the setter call, in a block `{ }`)
- No other daemon code runs concurrently at this point (startup sequence)

### 9.5 Overall Phase Risk Matrix

| Phase | Risk Level | Rationale |
|---|---|---|
| 2a | Medium | Touches the approval path. Behavior-preserving port + fallback. |
| 2b | Low-Medium | Thin wrapper. No protocol change. Single call site. |
| 2c | Low | Pure type relocation. Re-exports. Zero behavior change. |

---

## 10. Test Commands Reference

```bash
# Phase 2a
cargo test -p base   policy::permission_authority
cargo test -p runtime permission_manager
cargo test -p dasein  review_delegates_permission_verdict_to_authority
cargo test -p dasein  review_falls_back_to_inline_when_no_authority_installed
cargo test -p dasein  # ALL review tests

# Phase 2b
cargo test -p runtime host

# Phase 2c
cargo test -p base  types::vision
cargo test -p base  types::grounding
cargo test -p cognit  # all tests with only base dep

# Full workspace smoke (after each phase)
cargo build --workspace
cargo test --workspace

# Proof that cognit inversion is resolved (after 2c)
cargo tree -p cognit -e normal --prefix none | grep -E '^(corpus|interact) ' \
  && echo "STILL COUPLED (bad)" \
  || echo "OK: cognit depends only on base"
```

---

## 11. Implementation Checklist

### Phase 2c (recommended first -- lowest risk, highest payoff)

- [ ] Task 6: `base/src/types/vision.rs` + `base/Cargo.toml` deps + `base/src/types/mod.rs` + `base/src/lib.rs` + `corpus/src/drivers/driver/types.rs` re-export
- [ ] Task 7: `base/src/types/grounding.rs` + `base/src/types/mod.rs` + `base/src/lib.rs` + `interact/src/acix/grounding.rs` re-export + `cognit/src/impl/grounding/vision.rs` import update + `cognit/Cargo.toml` dep removal
- [ ] `cargo build --workspace` passes
- [ ] `cargo tree` confirms cognit has no corpus/interact deps

### Phase 2a

- [ ] Task 1: `base/src/policy/permission_authority.rs` trait
- [ ] Task 2: `runtime/src/core/permission_manager.rs` impl
- [ ] Task 3: `dasein/src/core/mod.rs` delegation + fallback + tests
- [ ] Task 4: `runtime/src/impl/daemon/handler/mod.rs` wiring
- [ ] All dasein tests pass (fallback + delegation)

### Phase 2b

- [ ] Task 5: `runtime/src/host/mod.rs` trait + DaemonHost + `runtime/src/lib.rs` + `runtime/src/bin/aletheond.rs`
- [ ] `cargo test -p runtime host` passes
- [ ] Manual smoke: daemon starts/stops/serves identically

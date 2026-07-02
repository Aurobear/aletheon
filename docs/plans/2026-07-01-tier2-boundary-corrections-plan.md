# Tier 2 — Boundary Corrections — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Design-only handoff — do not execute product changes until the design-only gate is lifted.**

**Goal:** Correct three layer-boundary violations the docs call out — (2a) move the permission/confirmation *policy* out of Self into a Runtime `PermissionManager`, (2b) introduce a `RuntimeHost` trait so the daemon is one host among many, and (2c) break the `cognit → corpus/interact` dependency inversion — so the layering is honest and multi-repo extraction (Tier 4) becomes possible.

**Architecture:** Three independent phases, each shippable on its own. 2a adds a `PermissionAuthority` trait in `base` (which `dasein` already depends on) and delegates the verdict to a `runtime` implementation, preserving today's behavior when no authority is installed. 2b wraps the existing daemon entry in a `DaemonHost: RuntimeHost` without changing the wire protocol. 2c moves 3 shared types (`Image`, `Bounds`, `GroundingProvider`/`GroundingResult`) into `base` and re-exports them, dropping `corpus`+`interact` from `cognit`'s dep list.

**Tech Stack:** Rust, `async-trait`, `tokio`, `base` types (`Context`, `Verdict`, `PermissionLevel`).

**Spec:** `docs/plans/2026-07-01-modules-roadmap-design.md § "Tier 2 — Boundary Corrections"`

**Branch:** `auro/feat/20260701-aletheon-tier2-boundaries` (own branch per repo policy).

---

## Ground truth (verified 2026-07-01)

| Fact | Anchor |
|---|---|
| Package names: `base`, `dasein`, `cognit`, `corpus`, `runtime`, `interact` | each `crates/*/Cargo.toml` `[package] name` |
| **2a** confirmation decision is inline in Self's `review()` | `crates/dasein/src/core/mod.rs:389-391` (`if care_score > 0.8 { if ctx.permissions.max_level() < base::PermissionLevel::SystemChange { RequireConfirmation }}`) |
| **2a** `review()` signature | `crates/dasein/src/core/mod.rs:346` `async fn review(&self, intent: &Intent, ctx: &Context) -> Result<Verdict>` |
| **2a** the Self type is `SelfField`, built via `SelfField::new(config: SelfFieldConfig)` | `crates/dasein/src/core/mod.rs:106` |
| **2a** `Verdict` lives in `base` (so a `base` trait can return it) | `crates/base/src/include/self_field.rs` (`pub enum Verdict`) |
| **2a** `Context` is a `base` type (trait can take `&Context`) | `crates/base/src/types/context.rs`, re-exported `base::context` (`lib.rs:47`) |
| **2a** `dasein` does NOT depend on `runtime` (deps: base, corpus, cognit, memory) | `crates/dasein/Cargo.toml` `[dependencies]` |
| **2a** existing `review()` test harness (`make_intent`, `minimal_ctx`, `make_ctx_with_perms`, `default_config`) | `crates/dasein/src/core/mod.rs:472-560` |
| **2b** daemon entry `pub async fn run(config_path, env_path, socket) -> Result<()>` | `crates/runtime/src/impl/daemon/mod.rs:77-80` |
| **2b** the only caller is the `aletheond` bin | `crates/runtime/src/bin/aletheond.rs` (`runtime::r#impl::…::run(args.config, args.env, args.socket).await`) |
| **2c** the ENTIRE `cognit → corpus/interact` coupling is 2 imports in 1 file | `crates/cognit/src/impl/grounding/vision.rs:7-8` (`use interact::acix::grounding::{GroundingProvider, GroundingResult}; use corpus::drivers::driver::types::Image;`) |
| **2c** `GroundingProvider`/`GroundingResult`/`MockGroundingProvider` defined here (depend only on `Image`/`Bounds` + anyhow + async_trait) | `crates/interact/src/acix/grounding.rs` |
| **2c** `Image`/`Bounds` defined here; `Image::to_base64_png` uses `png`+`base64` | `crates/corpus/src/drivers/driver/types.rs:93,136` (impl `:99-117`) |
| **2c** `png`/`base64` are corpus deps; `base` has neither yet | `crates/corpus/Cargo.toml:21-22`; `crates/base/Cargo.toml` (no png/base64) |

---

## File map

| Phase | File | Change |
|---|---|---|
| 2a | `crates/base/src/policy/permission_authority.rs` | **new** — `PermissionAuthority` trait |
| 2a | `crates/base/src/policy/mod.rs`, `crates/base/src/lib.rs` | export the trait |
| 2a | `crates/runtime/src/core/permission_manager.rs` | **new** — `PermissionManager: PermissionAuthority` |
| 2a | `crates/dasein/src/core/mod.rs` | hold optional authority; delegate at `:389-391`; keep inline fallback |
| 2a | `crates/runtime/src/core/verdict_handler.rs` | install the manager on the `SelfField` |
| 2b | `crates/runtime/src/host/mod.rs` | **new** — `RuntimeHost` trait + `DaemonHost` |
| 2b | `crates/runtime/src/impl/mod.rs` / lib | expose `host` module |
| 2b | `crates/runtime/src/bin/aletheond.rs` | call `DaemonHost` instead of `daemon::run` directly |
| 2c | `crates/base/src/types/vision.rs` | **new** — move `Image`, `Bounds` (+ codec) |
| 2c | `crates/base/src/types/grounding.rs` | **new** — move `GroundingProvider`/`GroundingResult`/`MockGroundingProvider` |
| 2c | `crates/base/Cargo.toml` | add `png`, `base64` |
| 2c | `crates/corpus/src/drivers/driver/types.rs`, `crates/interact/src/acix/grounding.rs` | re-export from `base` (back-compat) |
| 2c | `crates/cognit/src/impl/grounding/vision.rs`, `crates/cognit/Cargo.toml` | import from `base`; drop `corpus`+`interact` deps |

Default checks per phase: `cargo build --workspace`, plus the phase's `cargo test -p <pkg>`.

---

## Phase 2a — Permission policy → Runtime `PermissionManager`

**Design.** Today Self makes the confirmation decision inline. We introduce a
`PermissionAuthority` trait in `base` (returns an `Option<Verdict>`; `None` = "no
opinion, fall back"). `SelfField` optionally holds `Arc<dyn PermissionAuthority>`;
`review()` delegates to it, and **keeps the current inline rule as the fallback**
when no authority is installed (so behavior is unchanged until the manager is
wired). `runtime`'s `PermissionManager` implements the trait, reproducing exactly
today's `care_score > 0.8 && max_level() < SystemChange → RequireConfirmation`
rule (and is the future home for whitelist/sandbox-policy decisions).

### Task 1: `PermissionAuthority` trait in `base`

**Files:** create `crates/base/src/policy/permission_authority.rs`; modify `crates/base/src/policy/mod.rs`, `crates/base/src/lib.rs`.

- [ ] **Step 1: Write the failing test**

```rust
// crates/base/src/policy/permission_authority.rs (tests at bottom)
#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use std::path::PathBuf;

    struct AlwaysConfirm;
    impl PermissionAuthority for AlwaysConfirm {
        fn confirmation_verdict(&self, _ctx: &Context, _care: f64, action: &str) -> Option<crate::Verdict> {
            Some(crate::Verdict::RequireConfirmation {
                reason: format!("policy requires confirmation for '{action}'"),
                risk_level: crate::RiskLevel::Medium,
            })
        }
    }

    #[test]
    fn authority_can_return_a_verdict() {
        let ctx = Context::new("t", PathBuf::from("/tmp"));
        let v = AlwaysConfirm.confirmation_verdict(&ctx, 0.9, "system.reboot");
        assert!(matches!(v, Some(crate::Verdict::RequireConfirmation { .. })));
    }
}
```

- [ ] **Step 2: Run — expected FAIL** (`PermissionAuthority` undefined).

Run: `cargo test -p base policy::permission_authority`

- [ ] **Step 3: Implement the trait**

```rust
// crates/base/src/policy/permission_authority.rs
//! Runtime-owned permission policy contract (Tier 2a). The Self layer (`dasein`)
//! delegates the "does this action need user confirmation / is it permitted"
//! decision to whatever implements this trait, so the *policy* lives in the
//! Runtime while identity/care/boundary judgment stays in Self.

use crate::context::Context;
use crate::Verdict;

/// Decides permission verdicts on behalf of the Runtime.
///
/// Returning `None` means "no opinion" — the caller falls back to its default
/// behavior. This keeps the trait additive: an un-wired system behaves exactly
/// as before.
pub trait PermissionAuthority: Send + Sync {
    /// Given the current context and the care relevance of an action, decide
    /// whether it should be confirmed / gated. `None` = defer to caller default.
    fn confirmation_verdict(&self, ctx: &Context, care_score: f64, action: &str) -> Option<Verdict>;
}
```

```rust
// crates/base/src/policy/mod.rs
pub mod execpolicy;
pub mod permission_authority;
```

```rust
// crates/base/src/lib.rs — next to `pub use policy::execpolicy;`
pub use policy::permission_authority;
```

> Verify `crate::Verdict` and `crate::RiskLevel` are exported at the crate root
> (they are used by `dasein` as `Verdict`/`RiskLevel`). If they are only under
> `base::self_field`, use that path in the trait and test instead.

- [ ] **Step 4: Run — expected PASS.** `cargo test -p base policy::permission_authority`

- [ ] **Step 5: Commit**

```bash
git add crates/base/src/policy/permission_authority.rs crates/base/src/policy/mod.rs crates/base/src/lib.rs
git commit -m "feat(base): add PermissionAuthority trait (Runtime permission policy contract)"
```

### Task 2: `PermissionManager` in `runtime`

**Files:** create `crates/runtime/src/core/permission_manager.rs`; register the module in `crates/runtime/src/core/mod.rs`.

- [ ] **Step 1: Write the failing test**

```rust
// crates/runtime/src/core/permission_manager.rs (tests at bottom)
#[cfg(test)]
mod tests {
    use super::*;
    use base::context::Context;
    use base::policy::permission_authority::PermissionAuthority;
    use std::path::PathBuf;

    #[test]
    fn high_care_insufficient_perms_requires_confirmation() {
        let mgr = PermissionManager::new();
        let ctx = Context::new("t", PathBuf::from("/tmp")); // default perms < SystemChange
        let v = mgr.confirmation_verdict(&ctx, 0.9, "settings.update");
        assert!(matches!(v, Some(base::Verdict::RequireConfirmation { .. })));
    }

    #[test]
    fn low_care_no_opinion() {
        let mgr = PermissionManager::new();
        let ctx = Context::new("t", PathBuf::from("/tmp"));
        assert!(mgr.confirmation_verdict(&ctx, 0.1, "ls").is_none());
    }
}
```

- [ ] **Step 2: Run — expected FAIL** (`PermissionManager` undefined).

Run: `cargo test -p runtime permission_manager`

- [ ] **Step 3: Implement — reproduce today's rule exactly**

```rust
// crates/runtime/src/core/permission_manager.rs
//! Runtime permission policy (Tier 2a). Owns the confirmation/whitelist/sandbox
//! policy decisions that previously lived inline in `dasein::review()`. Phase 1
//! reproduces the exact prior rule; whitelist/sandbox policy are future additions.

use base::context::Context;
use base::policy::permission_authority::PermissionAuthority;
use base::{PermissionLevel, RiskLevel, Verdict};

/// The Runtime's permission authority.
#[derive(Default, Clone)]
pub struct PermissionManager;

impl PermissionManager {
    pub fn new() -> Self {
        Self
    }
}

impl PermissionAuthority for PermissionManager {
    fn confirmation_verdict(&self, ctx: &Context, care_score: f64, action: &str) -> Option<Verdict> {
        // Exact port of dasein/src/core/mod.rs:389-391 (behavior-preserving).
        if care_score > 0.8 && ctx.permissions.max_level() < PermissionLevel::SystemChange {
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
```

```rust
// crates/runtime/src/core/mod.rs — add
pub mod permission_manager;
```

> Confirm the exact import paths for `PermissionLevel`/`RiskLevel`/`Verdict` in
> `runtime` (they come from `base`; match however `dasein` imports them at
> `crates/dasein/src/core/mod.rs`).

- [ ] **Step 4: Run — expected PASS.** `cargo test -p runtime permission_manager`

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/core/permission_manager.rs crates/runtime/src/core/mod.rs
git commit -m "feat(runtime): PermissionManager implements PermissionAuthority (ports Self's rule)"
```

### Task 3: `SelfField` delegates the permission verdict

**Files:** modify `crates/dasein/src/core/mod.rs`.

- [ ] **Step 1: Write the failing test** (delegation: an installed authority's verdict is returned)

```rust
// crates/dasein/src/core/mod.rs tests module (reuse make_intent / minimal_ctx / default_config)
use base::policy::permission_authority::PermissionAuthority;
use std::sync::Arc;

struct StubAuthority;
impl PermissionAuthority for StubAuthority {
    fn confirmation_verdict(&self, _ctx: &base::context::Context, _care: f64, action: &str) -> Option<Verdict> {
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
    // An action that passes policy-bridge + boundary and reaches the permission stage.
    let intent = make_intent("settings.update", "update a setting");
    let ctx = minimal_ctx();
    let verdict = sf.review(&intent, &ctx).await.unwrap();
    assert!(matches!(verdict, Verdict::RequireConfirmation { .. }),
        "authority verdict must be honored, got {verdict:?}");
}
```

- [ ] **Step 2: Run — expected FAIL** (`set_permission_authority` undefined; no delegation).

Run: `cargo test -p dasein review_delegates_permission_verdict_to_authority`

- [ ] **Step 3a: Add the field + setter** to `SelfField`

```rust
// struct SelfField { ... } — add
    permission_authority: Option<std::sync::Arc<dyn base::policy::permission_authority::PermissionAuthority>>,
```

```rust
// in SelfField::new(...) — initialize
    permission_authority: None,
```

```rust
// impl SelfField
/// Install the Runtime's permission authority. Without it, the inline rule is used.
pub fn set_permission_authority(
    &mut self,
    authority: std::sync::Arc<dyn base::policy::permission_authority::PermissionAuthority>,
) {
    self.permission_authority = Some(authority);
}
```

- [ ] **Step 3b: Delegate at the permission stage** (`mod.rs:388-405`)

Replace the `if care_score > 0.8 { if ctx.permissions.max_level() < SystemChange { ... } }`
block with delegation + preserved fallback:

```rust
// 5. Permission check — delegate to the Runtime authority if installed,
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

- [ ] **Step 4: Run — expected PASS.** Also `cargo test -p dasein` (all existing `review_*` tests must still pass — they use no authority, so the fallback path is exercised unchanged).

- [ ] **Step 5: Commit**

```bash
git add crates/dasein/src/core/mod.rs
git commit -m "feat(dasein): delegate permission verdict to Runtime authority (inline fallback preserved)"
```

### Task 4: Wire the manager onto `SelfField` in the Runtime

**Files:** modify `crates/runtime/src/core/verdict_handler.rs` (or wherever the `SelfField` is constructed for the daemon).

- [ ] **Step 1: Locate the `SelfField` construction site**

Run: `rg -n "SelfField::new|SelfField::" crates/runtime/src` — find where the daemon builds the Self layer.

- [ ] **Step 2: Install the manager right after construction**

```rust
use crate::core::permission_manager::PermissionManager;
// after `let mut self_field = SelfField::new(cfg);`
self_field.set_permission_authority(std::sync::Arc::new(PermissionManager::new()));
```

- [ ] **Step 3: Build + behavior check**

Run: `cargo build --workspace`
Manual: a `SystemChange`-level action with high care still triggers confirmation;
a read-only action still passes without prompting (unchanged from before — now the
verdict comes from the Runtime manager instead of inline Self code).

- [ ] **Step 4: Commit**

```bash
git add crates/runtime/src/core/verdict_handler.rs
git commit -m "feat(runtime): install PermissionManager on the daemon's SelfField"
```

---

## Phase 2b — `RuntimeHost` trait (daemon becomes one host)

**Design.** Define a `RuntimeHost` trait and a `DaemonHost` that wraps today's
`daemon::run`. This proves the seam **without** extracting the full `RuntimeCore`
yet (a large, later refactor) and without touching the wire protocol. The
`aletheond` bin calls the host instead of `daemon::run` directly.

### Task 5: `RuntimeHost` trait + `DaemonHost`

**Files:** create `crates/runtime/src/host/mod.rs`; expose it from the crate root; modify `crates/runtime/src/bin/aletheond.rs`.

- [ ] **Step 1: Write the failing test** (trait is object-safe / drivable via a test double)

```rust
// crates/runtime/src/host/mod.rs (tests at bottom)
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct CountingHost { inited: Arc<AtomicUsize>, shut: Arc<AtomicUsize> }
    #[async_trait::async_trait]
    impl RuntimeHost for CountingHost {
        async fn init(&mut self) -> anyhow::Result<()> { self.inited.fetch_add(1, Ordering::SeqCst); Ok(()) }
        async fn serve(self: Box<Self>) -> anyhow::Result<()> { Ok(()) }
        async fn shutdown(&mut self) -> anyhow::Result<()> { self.shut.fetch_add(1, Ordering::SeqCst); Ok(()) }
    }

    #[tokio::test]
    async fn host_lifecycle_is_drivable() {
        let inited = Arc::new(AtomicUsize::new(0));
        let shut = Arc::new(AtomicUsize::new(0));
        let mut host = CountingHost { inited: inited.clone(), shut: shut.clone() };
        host.init().await.unwrap();
        host.shutdown().await.unwrap();
        assert_eq!(inited.load(Ordering::SeqCst), 1);
        assert_eq!(shut.load(Ordering::SeqCst), 1);
    }
}
```

- [ ] **Step 2: Run — expected FAIL** (`RuntimeHost` undefined).

Run: `cargo test -p runtime host::tests::host_lifecycle_is_drivable`

- [ ] **Step 3: Implement trait + `DaemonHost`**

```rust
// crates/runtime/src/host/mod.rs
//! Host abstraction (Tier 2b). A `RuntimeHost` is a deployment form of the
//! runtime; `DaemonHost` is the Unix-socket daemon. Additional hosts
//! (systemd/container) are M-F, built on this trait.

use anyhow::Result;
use std::path::PathBuf;

/// A deployment host for the runtime. `init` prepares resources, `serve` runs to
/// completion (blocking on the host's event loop), `shutdown` releases resources.
#[async_trait::async_trait]
pub trait RuntimeHost: Send {
    async fn init(&mut self) -> Result<()>;
    async fn serve(self: Box<Self>) -> Result<()>;
    async fn shutdown(&mut self) -> Result<()>;
}

/// The Unix-socket daemon host — wraps today's `daemon::run` unchanged.
pub struct DaemonHost {
    config_path: Option<PathBuf>,
    env_path: Option<PathBuf>,
    socket: PathBuf,
}

impl DaemonHost {
    pub fn new(config_path: Option<PathBuf>, env_path: Option<PathBuf>, socket: PathBuf) -> Self {
        Self { config_path, env_path, socket }
    }
}

#[async_trait::async_trait]
impl RuntimeHost for DaemonHost {
    async fn init(&mut self) -> Result<()> {
        // Socket dir creation etc. currently happens inside daemon::run; keep it there
        // for Phase 1 so behavior is identical. This hook exists for future hosts.
        Ok(())
    }

    async fn serve(self: Box<Self>) -> Result<()> {
        // Delegate to the existing, unchanged daemon entry point.
        crate::r#impl::daemon::run(self.config_path, self.env_path, self.socket).await
    }

    async fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}
```

```rust
// expose the module — in crates/runtime/src/lib.rs (or the crate root module file)
pub mod host;
```

> Verify the exact path to `daemon::run` from the crate root (the bin calls
> `runtime::r#impl::…::run`). Match that path in `DaemonHost::serve`.

- [ ] **Step 4: Rewire the `aletheond` bin**

```rust
// crates/runtime/src/bin/aletheond.rs — replace the direct daemon::run call
use runtime::host::{DaemonHost, RuntimeHost};

let mut host = DaemonHost::new(args.config, args.env, args.socket);
host.init().await?;
Box::new(host).serve().await
```

- [ ] **Step 5: Build + smoke**

Run: `cargo build --workspace` and `cargo test -p runtime host`
Manual smoke: start `aletheond`, connect over the socket, confirm it serves and
shuts down exactly as before (no protocol change).

- [ ] **Step 6: Commit**

```bash
git add crates/runtime/src/host/mod.rs crates/runtime/src/lib.rs crates/runtime/src/bin/aletheond.rs
git commit -m "feat(runtime): RuntimeHost trait + DaemonHost wrapping daemon::run (Tier 2b seam)"
```

---

## Phase 2c — Break the `cognit → corpus / interact` inversion

**Design.** The entire coupling is 2 imports in one file
(`cognit/src/impl/grounding/vision.rs:7-8`). Move the 3 shared types
(`Image`+`Bounds`, and `GroundingProvider`+`GroundingResult`+`MockGroundingProvider`)
into `base`, re-export them from their old homes for back-compat, point
`vision.rs` at `base`, and drop `corpus`+`interact` from `cognit/Cargo.toml`. This
costs `base` two tiny pure-Rust deps (`png`, `base64`, already used elsewhere).

### Task 6: Move `Image`/`Bounds` (+ codec) into `base`

**Files:** create `crates/base/src/types/vision.rs`; modify `crates/base/Cargo.toml`, `crates/base/src/lib.rs`, `crates/corpus/src/drivers/driver/types.rs`.

- [ ] **Step 1: Write the failing test**

```rust
// crates/base/src/types/vision.rs (tests at bottom)
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_encodes_to_base64_png() {
        let img = Image { width: 2, height: 2, data: vec![0u8; 2 * 2 * 3] };
        let (media_type, b64) = img.to_base64_png().unwrap();
        assert_eq!(media_type, "image/png");
        assert!(!b64.is_empty());
    }
    #[test]
    fn bounds_from_center() {
        let b = Bounds { x: 0, y: 0, width: 10, height: 10 };
        assert_eq!(b.width, 10);
    }
}
```

- [ ] **Step 2: Run — expected FAIL** (`base::types::vision` missing).

Run: `cargo test -p base types::vision`

- [ ] **Step 3a: Add deps to `base`**

```toml
# crates/base/Cargo.toml [dependencies]
png = "0.17"
base64 = "0.22"
```

- [ ] **Step 3b: Move the `Image`/`Bounds` definitions verbatim** from
`crates/corpus/src/drivers/driver/types.rs` into `crates/base/src/types/vision.rs`
(struct defs + the `impl Image` block with `to_base64_png` and any decode method +
their `use png; use base64::Engine;`). Register the module:

```rust
// crates/base/src/lib.rs — with the other `pub use types::...`
pub use types::vision;
```
```rust
// crates/base/src/types/mod.rs (or lib.rs mod list) — declare
pub mod vision;
```

- [ ] **Step 3c: Re-export from the old corpus path for back-compat**

```rust
// crates/corpus/src/drivers/driver/types.rs — replace the moved defs with:
pub use base::types::vision::{Bounds, Image};
```

> Every existing `corpus::drivers::driver::types::{Image, Bounds}` user keeps
> compiling via this re-export. `corpus` now depends on `base` for these (it
> already depends on `base`).

- [ ] **Step 4: Run — expected PASS.** `cargo test -p base types::vision` and `cargo build -p corpus`.

- [ ] **Step 5: Commit**

```bash
git add crates/base/Cargo.toml crates/base/src/types/vision.rs crates/base/src/lib.rs crates/base/src/types/mod.rs crates/corpus/src/drivers/driver/types.rs
git commit -m "refactor(base): move Image/Bounds into base; corpus re-exports (2c prep)"
```

### Task 7: Move grounding trait/types into `base`; sever cognit's corpus/interact deps

**Files:** create `crates/base/src/types/grounding.rs`; modify `crates/base/src/lib.rs`, `crates/interact/src/acix/grounding.rs`, `crates/cognit/src/impl/grounding/vision.rs`, `crates/cognit/Cargo.toml`.

- [ ] **Step 1: Write the failing test** (cognit imports grounding from `base`, not `interact`)

```rust
// crates/cognit/src/impl/grounding/vision.rs — this compiling with base-only imports IS the test.
// Add a small unit test to lock it in:
#[cfg(test)]
mod grounding_source_test {
    // Importing from base (not interact/corpus) must resolve:
    use base::types::grounding::{GroundingProvider, GroundingResult};
    use base::types::vision::Image;
    #[test]
    fn grounding_types_come_from_base() {
        let _r = GroundingResult { x: 1, y: 2, width: 3, height: 4, confidence: 0.5, label: "x".into() };
        let _i = Image { width: 1, height: 1, data: vec![0u8; 3] };
        let _ = std::marker::PhantomData::<dyn GroundingProvider>;
    }
}
```

- [ ] **Step 2: Run — expected FAIL** (`base::types::grounding` missing; cognit still imports from interact/corpus).

Run: `cargo test -p cognit grounding_source_test`

- [ ] **Step 3a: Move grounding defs verbatim** from
`crates/interact/src/acix/grounding.rs` into `crates/base/src/types/grounding.rs`
(the `GroundingResult` struct + impls, `GroundingProvider` trait, `MockGroundingProvider`,
and the `#[cfg(test)]` tests), changing its imports to `use crate::types::vision::{Bounds, Image};`.
Register:

```rust
// crates/base/src/lib.rs
pub use types::grounding;
// crates/base/src/types/mod.rs
pub mod grounding;
```

- [ ] **Step 3b: Re-export from the old interact path**

```rust
// crates/interact/src/acix/grounding.rs — replace moved defs with:
pub use base::types::grounding::{GroundingProvider, GroundingResult, MockGroundingProvider};
```

- [ ] **Step 3c: Point cognit at `base` and drop the deps**

```rust
// crates/cognit/src/impl/grounding/vision.rs:7-8 — replace
use base::types::grounding::{GroundingProvider, GroundingResult};
use base::types::vision::Image;
```

```toml
# crates/cognit/Cargo.toml — REMOVE these two lines
# corpus = { path = "../corpus", features = ["input", "display", "a11y", "ocr"] }
# interact = { path = "../interact" }
```

- [ ] **Step 4: Run — expected PASS + prove the inversion is gone**

Run:
```bash
cargo test -p cognit grounding_source_test
cargo build --workspace
cargo tree -p cognit -e normal --prefix none | grep -E '^(corpus|interact) ' && echo "STILL COUPLED (bad)" || echo "OK: cognit no longer depends on corpus/interact"
```
Expected: build passes; the `cargo tree` grep prints `OK:` (cognit's only in-workspace dep is now `base`).

- [ ] **Step 5: Commit**

```bash
git add crates/base/src/types/grounding.rs crates/base/src/lib.rs crates/base/src/types/mod.rs crates/interact/src/acix/grounding.rs crates/cognit/src/impl/grounding/vision.rs crates/cognit/Cargo.toml
git commit -m "refactor(cognit): depend only on base — grounding trait/types moved to base (2c)"
```

---

## Self-review checklist (done at plan-write time)

- **Spec coverage:** 2a (Tasks 1–4) ↔ "Security Policy in Runtime … `PermissionManager` … `dasein.review()` delegates via a trait so `dasein` doesn't depend on `runtime`"; 2b (Task 5) ↔ "`RuntimeHost` trait … refactor the current daemon into a `DaemonHost` … deliver the trait + `DaemonHost` only"; 2c (Tasks 6–7) ↔ "invert the dependency: move the shared contract into `base` as a trait … `cognit` then depends only on `base`".
- **Placeholder scan:** none — real traits, real ports of existing rules, real re-exports, exact `cargo`/`git` commands. Line-anchored to verified source.
- **Type consistency:** `PermissionAuthority::confirmation_verdict(&Context, f64, &str) -> Option<Verdict>` matches the `runtime` impl, the `dasein` call site, and both tests; `RuntimeHost` is object-safe (`serve(self: Box<Self>)`); 2c re-exports keep every existing `corpus::…::{Image,Bounds}` and `interact::acix::grounding::…` path valid.

## Risks / notes for the implementer

- **2a behavior preservation is load-bearing.** The `else if care_score > 0.8`
  fallback must stay until the manager is installed everywhere; the existing
  `dasein` `review_*` tests (no authority) guard it. Confirm `Verdict`/`RiskLevel`/
  `PermissionLevel` import paths in each crate before compiling (they originate in
  `base`; `dasein` uses bare `Verdict`/`RiskLevel`).
- **2a scope guard:** this phase moves only the *confirmation* decision. Whitelist
  and sandbox-policy selection are future `PermissionManager` methods — do not
  expand scope here; keep the port behavior-identical.
- **2b is a seam, not the full core extraction.** `RuntimeCore` (host-agnostic
  session/task/memory/provider/permission bundle) is deliberately NOT extracted
  here — that is the heavy Tier 4 refactor. `DaemonHost` just wraps `daemon::run`.
  Keep socket-dir/PID-file creation inside `daemon::run` for now so behavior is
  identical; migrate it into `init()` only when a second host needs it (M-F).
- **2c dep cost:** `base` gains `png`+`base64` (tiny, pure-Rust). If the team wants
  `base` to stay dependency-lean, the alternative is to keep `Image`'s codec as an
  extension trait in `corpus` — but that would force `cognit` to import from
  `corpus` again, defeating 2c. Moving the codec to `base` is the correct trade.
- **2c is behavior-neutral** — pure type relocation + re-exports; `cargo build
  --workspace` + `cargo tree` are the real gates. Watch for any *other* crate that
  imported these types via `cognit` (unlikely; they were defined in corpus/interact).
- **Ordering:** 2c unblocks the Tier 4 multi-repo extraction of `cognit`; 2b
  unblocks M-F hosts; 2a unblocks the M-D self-evolution permission gate. Land 2c
  first (lowest risk, highest structural payoff), then 2a, then 2b.

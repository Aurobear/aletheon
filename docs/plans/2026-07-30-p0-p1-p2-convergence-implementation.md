# P0/P1/P2 Convergence Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore architecture, runtime, documentation, toolchain, maintainability, and release acceptance across all approved P0/P1/P2 findings.

**Architecture:** Preserve public behavior while tightening boundaries: move broad `fabric` imports to owning modules, make fallible construction typed, inject time, extract cohesive helpers from oversized modules, and make documentation/release gates executable. Validation proceeds from crate-local tests to architecture gates and finally the installed system runtime.

**Tech Stack:** Rust 1.85, Tokio, GitHub Actions, Bash, Python 3, systemd, SHA-256.

---

### Task 1: Restore the Fabric root-export freeze

**Files:**
- Modify: `crates/fabric/src/lib.rs:333-359`
- Modify: call sites found by `rg 'fabric::(CanonicalEventBus|CommunicationBus|Observable|SubsystemStatus)' crates`

- [x] Audit root imports and keep external call sites on justified compatibility paths:

```rust
fabric::ipc::bus::kernel_bus::CanonicalEventBus
fabric::ipc::bus::communication_bus::CommunicationBus
fabric::kernel::observable::{Observable, SubsystemStatus}
```

- [x] Remove four unused root `pub use` statements: protocol, transport, debug, and debug bus.
- [x] Run `bash scripts/aletheon.sh test architecture`; expect no new findings.
- [x] Run focused affected-package checks through the repository Cargo wrapper.

### Task 2: Repair the public security policy

**Files:**
- Modify: `SECURITY.md:9-37`

- [x] Replace the placeholder contact with the repository advisory URL:

```markdown
Use [GitHub Private Vulnerability Reporting](https://github.com/Aurobear/aletheon/security/advisories/new).
```

- [x] Remove response-time promises that have no published operational owner.
- [x] Verify the security policy contains no placeholder contact markers.

### Task 3: Make harness construction fallible

**Files:**
- Modify: `crates/cognit/src/harness/mod.rs:67-83`
- Modify: `crates/cognit/tests/harness_construction.rs`

- [x] Add a typed error and return a `Result`:

```rust
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HarnessBuildError {
    #[error("{kind:?} harness requires Executive-owned ports")]
    RequiresExecutivePorts { kind: HarnessKind },
}
```

- [x] Assert `Linear` constructs successfully and `Robot` returns `RequiresExecutivePorts` without panicking.
- [x] Run `bash scripts/cargo-agent.sh test -p cognit --test harness_construction` and expect PASS.

### Task 4: Inject monotonic time into world-state observation

**Files:**
- Modify: `crates/executive/src/application/world_state.rs:19-107`
- Modify: `crates/executive/tests/world_state.rs`

- [x] Store `Arc<dyn fabric::Clock>` in `EmbodimentWorldState`.
- [x] Require the runtime's shared `Arc<dyn Clock>` in `new(max_devices, clock)`; do not construct an independent clock inside the adapter.
- [x] Replace `MonoTime(0)` with `self.clock.mono_now()`.
- [x] Add a mutable fixed test clock and verify an expired deadline returns `None` without waiting.
- [x] Run `bash scripts/cargo-agent.sh test -p executive --test world_state` and expect PASS.

- [x] Create one `kernel::chronos::SystemClock` in each official machine/user runtime composition root and pass the same `Arc<dyn Clock>` to the request handler, scheduler, perception manager, and socket server.

### Task 5: Close placeholder driver boundaries

**Files:**
- Modify: `crates/corpus/src/drivers/mod.rs:1-6`
- Delete if unreferenced: `crates/corpus/src/drivers/io/mod.rs`
- Delete if unreferenced: `crates/corpus/src/drivers/proc/mod.rs`

- [x] Confirm no current call sites with `rg 'drivers::(io|proc)' crates`.
- [x] Remove the empty public modules and their one-line placeholder files.
- [x] Run `bash scripts/cargo-agent.sh check -p corpus` and expect PASS.

### Task 6: Make README capability claims executable

**Files:**
- Modify: `README.md:105-277`
- Modify: `README.md:489-512`
- Create: `scripts/check-doc-paths.py`
- Modify: `.github/workflows/ci.yml:27-55`

- [x] List all 16 current crates and their actual dependency roles.
- [x] Replace every stale code anchor in the capability matrix with an existing current path.
- [x] Label eBPF, offline inference, FUSE, Android, and embedded behavior as experimental/planned everywhere they appear.
- [x] Implement `scripts/check-doc-paths.py` to reject missing repository paths and report `path:line` failures.
- [x] Run `python3 scripts/check-doc-paths.py`; expect `documentation paths: pass`.
- [x] Add that command to the architecture CI job.

### Task 7: Unify MSRV and repository build commands

**Files:**
- Modify: `CONTRIBUTING.md:13-40`
- Modify: `README.md:504-512`
- Modify: `.github/workflows/ci.yml:111-120`
- Modify: `.github/workflows/release.yml:46-50`

- [x] Set documentation and the CI MSRV job to Rust 1.85.
- [x] Replace direct repository Cargo examples with `bash scripts/cargo-agent.sh`.
- [x] Rename the CI job to `Rust 1.85 MSRV` and invoke `+1.85.0`.
- [x] Verify repository documentation and workflows use the Cargo wrapper for builds and checks.

### Task 8: Strengthen release artifacts

**Files:**
- Modify: `.github/workflows/release.yml`
- Create: `scripts/release_metadata.py`
- Create: `scripts/tests/test_release_metadata.py`

- [x] Add formatting and package tests before the release build.
- [x] Build through `bash scripts/cargo-agent.sh +stable build --release --target ... -p aletheon`.
- [x] Generate an SPDX-shaped JSON inventory from `Cargo.lock`, repository commit, target, and binary digest without installing network tooling.
- [x] Generate `SHA256SUMS` for every archive and metadata artifact.
- [x] Upload archives, metadata JSON, and `SHA256SUMS`; fail when any expected artifact is absent.
- [x] Unit-test metadata determinism with `python3 -m unittest scripts/tests/test_release_metadata.py`.

### Task 9: Extract cohesive units from oversized modules

**Files:**
- Modify: `crates/corpus/src/tools/tools/change_transaction.rs`
- Create: `crates/corpus/src/tools/tools/change_transaction/restore.rs`
- Create: `crates/corpus/src/tools/tools/change_transaction/validation.rs`
- Modify: `crates/cognit/src/harness/linear/mod.rs`
- Move existing execution module content as needed under: `crates/cognit/src/harness/linear/`
- Modify: `crates/executive/src/application/agent_control/mod.rs`
- Create: `crates/executive/src/application/agent_control/identity.rs`
- Create: `crates/executive/src/application/agent_control/lifecycle_hooks.rs`

- [x] Move baseline restore types/functions and validation projection/plan derivation without behavior changes.
- [x] Move linear-loop exploration policy and compaction metrics into focused submodules with private imports.
- [x] Move agent identity/runtime capability mapping and lifecycle hook adapter code into focused submodules.
- [x] Run the existing focused tests for each package after every extraction.
- [x] Confirm each original file is smaller and no new `fabric` root exports were added.

### Task 10: Final verification and installed acceptance

**Files:**
- Modify only defects found by validation.

- [x] Run `bash scripts/cargo-agent.sh fmt --all -- --check`.
- [x] Run focused tests from Tasks 1-9.
- [x] Run `bash scripts/aletheon.sh test architecture`.
- [x] Run `bash scripts/aletheon.sh acceptance architecture`.
- [x] As integration owner, run `bash scripts/cargo-agent.sh test --workspace` once.
- [x] Run `sudo bash scripts/aletheon.sh deploy` and require success.
- [x] Compare SHA-256 for `target/release/aletheon`, `/usr/bin/aletheon`, and both running system/user daemon executables.
- [x] Observe unchanged systemd restart counters across a stability interval.
- [x] Complete a real LLM request with `/usr/bin/aletheon` over the official user socket and reject any rendered/provider inference error.

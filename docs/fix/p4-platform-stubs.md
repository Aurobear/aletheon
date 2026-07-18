# P4 — Platform Stubs

Status: **Open** | Priority: Low (future phases)

---

## P4.1 io Driver Module is Entirely Stub

- **File:** `crates/corpus/src/drivers/io/mod.rs:1`
- **Severity:** P4
- **Description:** The entire io driver bindings module is a TODO stub. Marked for Phase 7/8 implementation.
- **Impact:** No hardware I/O device access through the corpus driver layer.
- **Fix direction:** Implement per Phase 7/8 plan; requires hardware abstraction layer design.

---

## P4.2 proc Driver Module is Entirely Stub

- **File:** `crates/corpus/src/drivers/proc/mod.rs:1`
- **Severity:** P4
- **Description:** The entire proc driver bindings module is a TODO stub. Marked for Phase 7/8 implementation.
- **Impact:** No /proc filesystem access through the corpus driver layer.
- **Fix direction:** Implement per Phase 7/8 plan.

---

## P4.3 Android Platform Driver is Explicit Stub

- **File:** `crates/corpus/src/platform/android.rs:3,25`
- **Severity:** P4
- **Description:** The Android platform driver is an explicit stub with no implementation.
- **Impact:** Aletheon cannot run on Android.
- **Fix direction:** Implement per multiplatform plan (`docs/plans/deepseek/2026-07-17-platform-a-os-multiplatform-detailed-plan.md`).

---

## P4.4 Cross-Process Shared Memory IPC is Single-Process Only

- **File:** `crates/fabric/src/ipc/shared_mem.rs`
- **Severity:** P4
- **Description:** The shared memory IPC uses `memfd_create` + `mmap` but does not pass file descriptors across processes. Effectively single-process only.
- **Impact:** Cannot use shared memory for daemon↔exec-server communication.
- **Fix direction:** Implement cross-process fd passing via Unix domain socket ancillary data (`SCM_RIGHTS`).

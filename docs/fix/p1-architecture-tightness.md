# P1 — Architecture Tight Coupling

Status: **Open** | Blocked by: conscious-core R2/R3 plan

---

## P1.1 TurnPipeline Semi-Concrete Coupling

- **File:** `crates/executive/src/impl/daemon/turn_pipeline.rs:42-59`
- **Severity:** P1
- **Description:** 7 of 14 fields are concrete types (not traits), making the pipeline hard to test and replace in isolation.
- **Impact:** Unit testing requires full concrete dependency setup; swapping implementations requires recompilation of the entire pipeline.
- **Fix direction:** Extract traits for the concrete fields; inject through constructor.

---

## P1.2 DaemonTurnOrchestrator God Object

- **File:** `crates/executive/src/impl/daemon/orchestrator.rs:22-30`
- **Severity:** P1
- **Description:** 7 of 7 fields are concrete types. Exposes `Arc<Mutex<>>` as public API. Single struct holds all orchestration concerns.
- **Impact:** Impossible to test individual concerns; all state is globally mutable; high contention on Mutex.
- **Fix direction:** Split into focused sub-components (turn admission, execution dispatch, result collection); hide locks behind async interfaces.

---

## P1.3 TurnRuntimeResources Leaks 17 Concrete Types

- **File:** `crates/executive/src/impl/daemon/turn_runtime_ports.rs:105-135`
- **Severity:** P1
- **Description:** 17 concrete types + 8 Mutex fields, all `pub(crate)`. No abstraction boundary between the port definitions and their implementations.
- **Impact:** Any change to a resource type ripples through all consumers; no way to mock resources for testing.
- **Fix direction:** Define resource traits; make TurnRuntimeResources hold trait objects; provide a test-kit constructor.

---

## P1.4 KernelRuntime Getter Lies About Immutability

- **File:** `crates/kernel/src/runtime.rs:189-209`
- **Severity:** P1
- **Description:** Getters return `Arc<ConcreteMutableType>` despite documentation claiming "immutable snapshots." Callers receive shared mutable state.
- **Impact:** Doc-contract mismatch; callers may mutate state through the Arc unexpectedly; thread-safety relies on the inner Mutex rather than the API guarantee.
- **Fix direction:** Either return true snapshots (clone on read) or update documentation to reflect shared mutable access.

# F01 Domain Facade Authority Implementation Plan

> **Status:** Reopened / Partial — facade contracts and static gates exist, but production services still retain concrete domain state.

**Goal:** Make Metacog, Cognit and Corpus production behavior reachable through one typed facade per domain and remove concrete-domain imports from request handlers.

**Architecture:** Each domain owns one typed facade and private adapter; Executive composition builds adapters, while request and turn paths retain only facade objects.

**Tech Stack:** Rust, async-trait, Executive composition root, architecture source gates

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1106-1187` and `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:815-817`.

**Prerequisites:** X02 and E03.

## Current status reconciliation (2026-07-18)

- Facade contracts and the dependency-boundary gate are present at `crates/executive/tests/domain_facade_authority.rs:20-76` and `scripts/architecture-check.sh:124-160`.
- F01 is not complete while production request services own concrete `AletheonExecutive`, `EpisodicMemory`, and `SelfField` handles at `crates/executive/src/service/request_use_cases.rs:70-101`.
- F01 is not complete while execution and turn-runtime services construct or retain concrete `ToolRunnerWithGuard`, `SelfField`, and `AletheonExecutive` state at `crates/executive/src/service/exec_session.rs:16-23` and `crates/executive/src/service/turn_runtime_ports.rs:105-128`.

## Invariants and non-goals

- This slice does not merge domain crates.
- Domain implementation types remain usable in domain-local tests.
- Retry policy is typed at facade boundaries rather than inferred from strings.

## Task 1: Keep the dependency-boundary characterization gate authoritative

**Modify:** `crates/executive/tests/domain_facade_authority.rs`
**Modify:** `scripts/architecture-check.sh`

- [x] List allowed production imports for each domain facade.
- [ ] Ensure the gate covers request, execution-session, and turn-runtime concrete handles rather than only import spelling.
- [ ] Exempt composition-root modules and domain-local tests only.

## Task 2: Retain authoritative domain operations

- [x] Metacog exposes typed service operations.
- [x] Cognit exposes session and configuration contracts.
- [x] Corpus exposes governed catalog and execution operations.
- [ ] Re-verify that production callers retain only those contracts after Task 3.

## Task 3: Migrate remaining production composition and delete bypasses

**Modify:** `crates/executive/src/service/request_use_cases.rs`
**Modify:** `crates/executive/src/service/exec_session.rs`
**Modify:** `crates/executive/src/service/turn_runtime_ports.rs`
**Modify:** private bootstrap composition modules as required

- [ ] Replace concrete domain state retained by request use cases with typed use-case/facade ports.
- [ ] Move concrete runner construction out of execution-session behavior and behind the authoritative Corpus boundary.
- [ ] Replace concrete Dasein/orchestrator handles retained by turn-runtime services with typed ports.
- [ ] Keep concrete adapter construction private to bootstrap composition.

## Final verification

Run focused boundary checks once after the migration batch:

```bash
bash scripts/architecture-check.sh
cargo test -p executive --test domain_facade_authority --no-fail-fast
```

## Completion evidence

- [ ] Every production operation crosses one domain facade.
- [ ] Executive request, execution-session, and turn-runtime services contain no concrete store, registry, runner, or harness implementation ownership.
- [ ] Domain error and retry classification is explicit at each facade.

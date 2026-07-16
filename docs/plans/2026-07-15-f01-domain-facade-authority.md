# F01 Domain Facade Authority Implementation Plan

**Goal:** Make Metacog, Cognit and Corpus production behavior reachable through one typed facade per domain and remove concrete-domain imports from request handlers.

**Architecture:** Each domain owns one typed facade and private adapter; Executive composition builds adapters, while request and turn paths retain only facade objects.

**Tech Stack:** Rust, async-trait, Executive composition root, architecture source gates

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1065-1115` and `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:815-817`.

**Prerequisites:** X02 and E03.

## Current-code anchors

- Executive `DomainPorts` contains only Agora at `crates/executive/src/core/domain_ports.rs:8-18`.
- Daemon bootstrap directly constructs Metacog, Cognit, Corpus, Dasein and Mnemosyne concrete types in `crates/executive/src/impl/daemon/bootstrap/request.rs:24-59`.
- Cognit exposes `CognitiveSession` at `crates/cognit/src/harness/session.rs:36-45`.
- Corpus has a governed execution adapter at `crates/corpus/src/tools/capability_executor.rs:41`.

## Invariants and non-goals

- This slice does not merge domain crates.
- Domain implementation types remain usable in domain-local tests.
- Retry policy is typed at facade boundaries rather than inferred from strings.

## Key contracts

```rust
#[async_trait] pub trait MetacogService: Send + Sync { async fn verify(&self, req: VerifyMutation) -> Result<VerificationReceipt, MetacogError>; async fn apply(&self, req: ApplyMutation) -> Result<MutationReceipt, MetacogError>; }
#[async_trait] pub trait CorpusService: Send + Sync { async fn catalog(&self, scope: CapabilityScope) -> Result<ExtensionSnapshot, CorpusError>; async fn invoke(&self, req: GovernedInvocation) -> Result<CapabilityOutcome, CorpusError>; }
```

## Task 1: Add a dependency-boundary characterization gate

**Create:** `crates/executive/tests/domain_facade_authority.rs`
**Modify:** `scripts/architecture-check.sh`

- [ ] List allowed production imports for each domain facade.
- [ ] Fail on Executive handler imports of Mnemosyne stores, Corpus registries/runners, Metacog pipelines or Cognit concrete harness internals.
- [ ] Exempt composition-root modules and domain-local tests only.

Run: `bash scripts/architecture-check.sh`

Expected before migration: the gate reports current imports in daemon bootstrap/request paths.

## Task 2: Define authoritative Metacog operations

**Create:** `crates/metacog/src/service.rs`
**Modify:** `crates/metacog/src/lib.rs`
**Create:** `crates/metacog/tests/service_contract.rs`

- [ ] Define typed verify, apply, rollback and status requests, receipts and errors.
- [ ] Persist one mutation lineage and migration state behind `MetacogService`.
- [ ] Require governed capability/approval evidence before apply or rollback.
- [ ] Keep morphogenesis and runtime internals private to the adapter.

Run: `cargo test -p metacog --test service_contract`

## Task 3: Narrow Cognit to session and configuration contracts

**Modify:** `crates/cognit/src/harness/session.rs`
**Modify:** `crates/cognit/src/config/mod.rs`
**Modify:** `crates/cognit/src/lib.rs`
**Create:** `crates/cognit/tests/facade_contract.rs`

- [ ] Make `CognitiveSession`/factory the only production inference-loop boundary.
- [ ] Retain typed Cognit sub-configuration but remove application-level layered loading under Q01.
- [ ] Remove concrete Kernel construction and require injected clock, cancellation, events and capability ports.
- [ ] Map provider/runtime failures to typed retryable or terminal domain errors.

Run: `cargo test -p cognit --test facade_contract`

## Task 4: Consolidate Corpus catalogs and execution facade

**Create:** `crates/corpus/src/service.rs`
**Modify:** `crates/corpus/src/lib.rs`
**Modify:** `crates/corpus/src/tools/capability_executor.rs`
**Create:** `crates/corpus/tests/service_contract.rs`

- [ ] Define catalog discovery, scoped activation and governed invocation operations.
- [ ] Put tools, skills, hooks, plugins and MCP entries behind stable IDs and capability scopes.
- [ ] Keep runner, sandbox, credential and registry implementations private.
- [ ] Reject eager default exposure that lacks a session/Agent capability grant.

Run: `cargo test -p corpus --test service_contract`

## Task 5: Migrate production composition and delete bypasses

**Modify:** `crates/executive/src/core/domain_ports.rs`
**Modify:** `crates/executive/src/impl/daemon/bootstrap/request.rs`
**Modify:** `crates/executive/src/service/turn_pipeline.rs`

- [ ] Add Metacog, Cognit and Corpus facade objects to private composition state.
- [ ] Build concrete adapters only in bootstrap composition modules.
- [ ] Migrate request/turn/goal paths to facades and remove direct store/registry/pipeline handles.
- [ ] Route remaining Scratchpad/direct Agora mutations through authoritative service operations.

Run: `bash scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

## Final verification and commit

Run: `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

Inspect the staged diff, then commit with subject `refactor(domains): enforce authoritative facades` and a body that records the source requirement, authority/bypass problem, implemented boundaries, focused tests and deletion evidence.

## Completion evidence

- [ ] Every production operation crosses one domain facade.
- [ ] Executive request handlers contain no concrete store, registry, runner or harness implementation imports.
- [ ] Domain error and retry classification is explicit at each facade.

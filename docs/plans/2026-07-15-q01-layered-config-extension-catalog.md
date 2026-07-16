# Q01 Layered Config and Extension Catalog Implementation Plan

**Goal:** Give Executive sole ownership of layered application configuration and expose all extensions through one policy-scoped catalog.

**Architecture:** Executive merges and validates all application layers with per-leaf provenance; Corpus indexes extension metadata and E03 authorizes session-specific activation.

**Tech Stack:** Rust, Serde, JSON Schema, Corpus extension loaders, E03 capability service

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1141-1151`.

**Prerequisites:** X02, E03 and R01.

## Current-code anchors

- Executive `AppConfig` and `load_layered` exist at `crates/executive/src/core/config/mod.rs:23-163`.
- Cognit duplicates application `AppConfig` and layered loading at `crates/cognit/src/config/mod.rs:28` and `crates/cognit/src/config/mod.rs:1154`.
- Corpus publicly exposes separate hook, skill and tool modules at `crates/corpus/src/lib.rs:3-21`.
- E03 supplies governed invocation contracts at `crates/executive/src/service/governed_capability.rs:21-64`.

## Invariants and non-goals

- Domain crates keep typed sub-configs but do not read application layers.
- Catalog discovery does not imply activation.
- Secrets never appear in schema snapshots or provenance output.

## Key contracts

```rust
pub struct Provenanced<T> { pub value: T, pub source: ConfigSource }
pub struct ExtensionDescriptor { pub id: ExtensionId, pub kind: ExtensionKind, pub version: String, pub capabilities: Vec<CapabilityId>, pub origin: ExtensionOrigin }
pub trait ExtensionCatalog { fn snapshot(&self) -> ExtensionSnapshot; }
```

## Task 1: Freeze merge, provenance and schema behavior

**Create:** `crates/executive/tests/layered_config_contract.rs`

- [ ] Cover defaults, system, user, project, environment and CLI precedence.
- [ ] Record source locator for every effective leaf and every validation error.
- [ ] Assert unknown fields, invalid types and secret rendering fail safely.
- [ ] Snapshot a deterministic generated JSON schema.

Run: `cargo test -p executive --test layered_config_contract`

## Task 2: Make Executive the sole application config loader

**Modify:** `crates/executive/src/core/config/mod.rs`
**Create:** `crates/executive/src/core/config/provenance.rs`
**Create:** `crates/executive/src/core/config/schema.rs`
**Modify:** `crates/cognit/src/config/mod.rs`

- [ ] Define typed domain sub-configs and one Executive root schema.
- [ ] Merge layers structurally while retaining provenance and redacting secret values.
- [ ] Pass validated Cognit/Corpus/Mnemosyne/Dasein/Agora sub-configs to composition.
- [ ] Delete Cognit's application-level file/environment loading and retain only typed domain config.

Run: `cargo test -p executive --test layered_config_contract && cargo test -p cognit --lib config`

## Task 3: Build one extension catalog contract

**Create:** `crates/fabric/src/types/extension.rs`
**Modify:** `crates/fabric/src/types/mod.rs`
**Create:** `crates/corpus/src/catalog/mod.rs`
**Create:** `crates/corpus/tests/extension_catalog.rs`

- [ ] Define stable extension ID, kind, version, origin, declared capabilities and activation constraints.
- [ ] Index skills, plugins, MCP tools and hooks without eagerly activating them.
- [ ] Reject duplicate identities and incompatible schema/capability declarations deterministically.
- [ ] Return immutable catalog snapshots for session-specific activation.

Run: `cargo test -p corpus --test extension_catalog`

## Task 4: Enforce scoped activation through E03

**Create:** `crates/executive/src/service/extension_service.rs`
**Create:** `crates/executive/tests/extension_activation_policy.rs`
**Modify:** `crates/executive/src/impl/daemon/bootstrap/request.rs`

- [ ] Resolve requested extensions against effective config and Agent/session policy.
- [ ] Materialize only capability grants approved by E03.
- [ ] Prevent hooks/plugins/MCP entries from bypassing capability invocation or approval.
- [ ] Record activation decisions and config provenance as versioned R01 events.

Run: `cargo test -p executive --test extension_activation_policy`

## Task 5: Add CI drift gates

**Create:** `config/schema/aletheon-config.schema.json`
**Modify:** `scripts/architecture-check.sh`
**Modify:** `justfile`

- [ ] Regenerate schema deterministically and fail when checked-in output differs.
- [ ] Reject layered config loaders outside Executive.
- [ ] Reject direct extension execution outside E03 adapters.

Run: `just check-architecture && git diff --exit-code -- config/schema/aletheon-config.schema.json`

## Final verification and commit

Run: `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

Inspect the staged diff, then commit with subject `feat(config): unify config and extension catalog` and a body that records the source requirement, authority/bypass problem, implemented boundaries, focused tests and deletion evidence.

## Completion evidence

- [ ] Effective values can be traced to a source without leaking secrets.
- [ ] Cognit no longer owns application config loading.
- [ ] Every active extension has an explicit scoped capability decision.

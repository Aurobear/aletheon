# Phase 1 Fabric Contract Purification Implementation Plan

> **For DeepSeek:** Execute this plan task-by-task. Do not reinterpret the architecture or combine stages. Check each box only after its evidence exists.

**Goal:** Remove provider-specific Google/Gmail identity, scope, event, and information-source types from Fabric while preserving wire, persistence, and authorization behavior.

**Architecture:** Introduce provider-neutral IDs and mail/calendar/file contracts first, add compatibility conversion and dual-read persistence, migrate adapters/callers, then delete provider-specific Fabric exports.

**Tech Stack:** Rust 1.85+, Bash, Python 3, Cargo via `scripts/cargo-agent.sh`, repository architecture gates.

---

## Global execution constraints

- Treat `docs/arch/CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md` as the architecture source of truth.
- Re-read that document and every cited symbol before editing; record changed line anchors in the task report.
- Do not modify files outside the declared paths. Stop if a required change crosses the boundary and report it.
- Preserve unrelated working-tree changes. Never use `git reset --hard`, `git checkout --`, or broad cleanup commands.
- Never invoke Cargo directly. Use `bash scripts/cargo-agent.sh <cargo arguments>` and the narrowest package/test target.
- Do not run concurrent Executive or workspace builds. Only the final integration owner runs workspace-wide commands.
- Keep security-sensitive behavior fail-closed. Do not weaken credential, scope, sandbox, network, lease, or trust checks.
- Each non-trivial commit must use a conventional subject, blank line, problem/solution context, and concrete bullets.
- Before each commit run `git diff --cached --check` and inspect the complete staged diff.
- A task is incomplete if tests pass but its architecture gate, compatibility evidence, or inventory update is missing.

## Prerequisites and owned paths

Prerequisite: Phase 0 wire/persistence inventories committed.

Primary ownership:

- Modify: `crates/fabric/src/types/external_identity.rs`
- Modify: `crates/fabric/src/types/external_event.rs`
- Replace/retire: `crates/fabric/src/types/google.rs`
- Modify: `crates/fabric/src/types/mod.rs`, `crates/fabric/src/lib.rs`
- Modify adapter conversions under `crates/corpus/src/tools/google/`
- Modify consumers under `crates/executive/src/impl/google/`, `impl/external/`, and Gmail channel handlers
- Add Fabric contract and compatibility tests
- Update Phase 0 inventories and architecture baselines

## Task 1: Lock current schema and security behavior

- [ ] Add serialization fixtures for current external identity, grants, scopes, and every `ExternalEventEnvelope` payload variant.
- [ ] Add regression tests for `ExternalScope::is_write`, `is_m6_allowed`, grant validation, read-only OAuth requests, and write-scope rejection.
- [ ] Add round-trip tests for current persisted `envelope_json` rows.
- [ ] Run the tests and capture PASS before refactoring; these fixtures define compatibility behavior.

Run narrow targets:

```bash
bash scripts/cargo-agent.sh test -p fabric external_identity
bash scripts/cargo-agent.sh test -p corpus --test google_read_only
bash scripts/cargo-agent.sh test -p executive google
```

## Task 2: Introduce provider-neutral identity contracts

Add bounded validated newtypes:

```rust
pub struct ExternalProviderId(String);
pub struct ExternalCapabilityId(String);
```

- [ ] Constructors reject empty, oversized, control-character, or non-canonical values.
- [ ] `ExternalIdentity` stores provider ID without a closed provider enum.
- [ ] `CapabilityGrant` stores canonical capability IDs.
- [ ] Authorization policy maps capability IDs to read/write/security semantics in the provider adapter/policy owner, not by string parsing in Fabric.
- [ ] Keep explicit legacy deserialization/conversion for `google` and old scope names.

## Task 3: Introduce neutral information-source DTOs

Add bounded contracts for:

```text
MailQuery / MailMessageSummary / MailMessage
CalendarQuery / CalendarEntry
ExternalFileMetadata / ExternalChangeBatch
OpaqueProviderObjectId / OpaqueCursor
```

- [ ] No type or field name contains Google, Gmail, Drive, or vendor scope URLs.
- [ ] DTO validation preserves current size, page, account, timestamp, and content-integrity bounds.
- [ ] Provider-specific cursors and identifiers remain opaque outside adapters.

## Task 4: Version ExternalEvent and add dual-read

- [ ] Define the next external-event schema version.
- [ ] Keep a legacy V1 decoder matching committed fixtures.
- [ ] Convert V1 Google payloads into canonical neutral payloads.
- [ ] Write only the new version after migration.
- [ ] Unknown versions fail closed without deleting or overwriting source data.
- [ ] Add SQLite migration/dual-read tests against `executive/src/impl/google/store.rs` fixtures.

## Task 5: Migrate Google adapters and application consumers

- [ ] Corpus Google code converts wire responses into neutral DTOs.
- [ ] Executive application consumes neutral identity/event/source ports.
- [ ] Google OAuth scope mapping and M6/write gating stay inside the adapter/security policy boundary.
- [ ] Remove application imports of `GoogleEvent`, `GmailMessageSummary`, and provider-specific scope variants.
- [ ] Do not rename adapter-local Google modules; provider names are legal there.

## Task 6: Remove provider-specific Fabric public surface

- [ ] Remove `pub mod google` and `pub use types::google` after all callers migrate.
- [ ] Delete legacy concrete types only after compatibility readers are isolated.
- [ ] Architecture gate shows Fabric provider-type count reaches zero.
- [ ] Update wire/persistence inventory and compatibility-debt counts.

## Validation

```bash
bash scripts/cargo-agent.sh test -p fabric
bash scripts/cargo-agent.sh test -p corpus --test google_read_only
bash scripts/cargo-agent.sh test -p corpus --test google_delta_sync
bash scripts/cargo-agent.sh test -p executive google
bash tests/architecture_check.sh
```

Expected: PASS; legacy fixtures decode; new writes use the canonical schema; scope security behavior is unchanged.

## Commit stages

1. `test(fabric): lock external identity and event compatibility`
2. `refactor(fabric): add provider-neutral external contracts`
3. `refactor(integrations): migrate Google adapters to neutral DTOs`
4. `refactor(fabric): remove provider-specific public contracts`
5. `chore(arch): enforce provider-neutral Fabric boundary`

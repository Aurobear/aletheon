# Phase 5 Supplemental Memory Generalization Implementation Plan

> **For DeepSeek:** Execute this plan task-by-task. Do not reinterpret the architecture or combine stages. Check each box only after its evidence exists.

**Goal:** Remove GBrain product naming from memory core, Executive application, health, quota, and configuration while preserving the MCP-based adapter and existing spool data.

**Architecture:** Introduce canonical supplemental-memory contracts/config/status, dual-read old config and spool metadata, migrate core callers, then confine GBrain naming to compatibility/deploy/adapter diagnostics.

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

Prerequisite: Phase 2 and Phase 0 persistence inventory.

- Modify: Mnemosyne supplemental backend/contracts/application paths
- Modify: Executive memory bootstrap/status/quota/application paths
- Modify: Cognit legacy config only through Phase 4 normalization boundary
- Modify: Corpus MCP adapter integration as required
- Modify: GBrain spool migrations with idempotent compatibility
- Add memory, Executive, config, migration, and architecture tests

## Task 1: Freeze current memory and spool behavior

- [x] Add fixtures for current GBrain config, schema fixture, spool DB schema/version, retry policy, reconciliation state, health keys, quotas, and legacy outbox migration.
- [x] Prove disabled supplemental memory degrades to local memory.
- [x] Prove configured-but-invalid memory fails according to required/optional policy.
- [x] Prove spool migrations are idempotent and source data survives failure.

## Task 2: Canonical contracts and names

Define/reuse:

```text
SupplementalMemoryConfig
SupplementalMemoryTransport
SupplementalHit / Document / DocumentId
SupplementalMemoryStatus
SupplementalMemoryError
SupplementalSpoolPolicy
```

- [x] Mnemosyne domain/application paths contain no GBrain name.
- [x] Contract is independent of MCP method/tool names.
- [x] MCP tool mapping stays in adapter config/implementation.

## Task 3: Config and persistence compatibility

- [x] Canonical config key is `memory.supplemental`.
- [x] Old `memory.gbrain` input normalizes deterministically but is never re-emitted.
- [x] Old spool tables/metadata remain readable; new writes use canonical schema/names only if a safe migration is justified.
- [x] If table renaming has no functional benefit, keep physical legacy table names behind repository implementation and document that they are persistence compatibility, not core vocabulary.
- [x] Unknown schema versions fail closed.

## Task 4: Migrate application, health, and quota callers

- [x] Replace `gbrain_spool` application health/status keys with canonical supplemental keys plus compatibility output only where externally required.
- [x] Rename quota fields canonically through config normalization.
- [x] Executive application depends only on supplemental status/transport ports.
- [x] Deployment instance/server name remains data and may still be `gbrain`.

## Task 5: Adapter isolation

- [x] MCP-specific tool names, schema fixture validation, HTTP client, and diagnostics stay under adapter.
- [x] Mnemosyne crate root no longer publicly exports `backends::gbrain`.
- [x] Compatibility alias has a counted exit condition.
- [x] Architecture metric for core GBrain names reaches zero.

## Validation

```bash
bash scripts/cargo-agent.sh test -p mnemosyne --test gbrain_spool
bash scripts/cargo-agent.sh test -p mnemosyne --test gbrain_backend_contract
bash scripts/cargo-agent.sh test -p mnemosyne --test gbrain_reconciliation
bash scripts/cargo-agent.sh test -p executive --test gbrain_bootstrap
bash scripts/cargo-agent.sh test -p executive --test gbrain_mcp_adapter
bash scripts/cargo-agent.sh test -p executive --test layered_config_contract
bash tests/architecture_check.sh
```

## Commit stages

1. `test(memory): lock supplemental spool compatibility`
2. `refactor(memory): add product-neutral supplemental contracts`
3. `refactor(config): normalize legacy shared-memory settings`
4. `refactor(executive): migrate supplemental health and quotas`
5. `chore(arch): confine product names to memory adapters`

## Completion evidence (2026-07-23)

- Mnemosyne exposes product-neutral `SupplementalMemoryBackend`, transport, document, spool, reconciliation, error, status, and observability names under `backends::supplemental`; the former public `backends::gbrain` module no longer exists.
- Executive application health, worker lifecycle, runtime inputs, quota fields, and composition configuration use supplemental-memory vocabulary. `[memory.gbrain]`, old quota keys, enum labels, and physical SQLite table names remain read-compatible and are never emitted as canonical configuration.
- Physical `gbrain_*` SQLite tables remain private persistence compatibility to avoid a no-value risky rename; their exact occurrences and Phase 10 exit decision are ratcheted. Unknown future schema versions continue to fail closed.
- MCP method/schema details and deployment instance naming remain in the Executive adapter boundary. Spool, backend, reconciliation, bootstrap, MCP adapter, layered-config, and architecture validations pass.

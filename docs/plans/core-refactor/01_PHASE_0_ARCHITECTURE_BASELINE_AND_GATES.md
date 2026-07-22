# Phase 0 Architecture Baseline and Gates Implementation Plan

> **For DeepSeek:** Execute this plan task-by-task. Do not reinterpret the architecture or combine stages. Check each box only after its evidence exists.

**Goal:** Freeze current architecture ownership, wire/persistence surfaces, compatibility debt, and measurable anti-regression gates without changing business behavior.

**Architecture:** Create machine-readable inventories under `config/architecture/`, extend the existing Bash gate, and add fixture-driven tests. This phase records violations as ratcheted baselines rather than attempting later refactors early.

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

## Owned paths

- Create: `config/architecture/external-identifiers.txt`
- Create: `config/architecture/module-boundaries.txt`
- Create: `config/architecture/wire-surfaces.tsv`
- Create: `config/architecture/persistence-surfaces.tsv`
- Create: `config/architecture/compatibility-debt.tsv`
- Create: `config/architecture/metrics.env`
- Modify: `scripts/architecture-check.sh`
- Modify: `tests/architecture_check.sh`
- Create: `tests/fixtures/architecture/README.md`
- Modify: `docs/arch/CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md` only to link frozen inventory artifacts

## Task 1: Freeze repository and ownership inventory

- [ ] Record `git rev-parse HEAD` at the top of every generated inventory.
- [ ] Enumerate all workspace crates from `Cargo.toml`, their public modules, local-crate dependencies, `impl/` directories, and concrete adapter directories.
- [ ] Record Runtime ownership exactly: `runtime` owns manifest/capability/selector contracts; Kernel owns governed lifecycle; Executive owns runtime instances/composition; Platform owns OS capabilities.
- [ ] Add a test that fails if a new workspace crate or top-level `impl/` appears without an inventory update.

Run:

```bash
bash tests/architecture_check.sh
```

Expected: PASS after inventory files match the current tree.

## Task 2: Produce wire-surface inventory

Every TSV row must contain:

```text
symbol\tpath\tparticipants\tprotocol_owner\tversion_mechanism\tcompatibility_rule\tphase
```

- [ ] Cover Fabric client protocol and `CLIENT_PROTOCOL_VERSION`.
- [ ] Cover Executive/execd JSON-RPC and handshake `protocol_version`.
- [ ] Cover Hardware protobuf/gRPC service and generated Rust boundary.
- [ ] Cover MCP protocol plus configured tool schema.
- [ ] Cover serialized external events, agent-run state, memory/spool, audit and artifact formats.
- [ ] Classify every Fabric public DTO referenced by protocol/persistence code as `wire-exposed` or `internal-shared`.
- [ ] Add a fixture proving an unowned wire symbol fails the gate.

## Task 3: Produce persistence-surface inventory

Every row must contain:

```text
name\towner\tschema_path\tversion_path\treaders\twriters\tmigration_rule\taffected_phase
```

At minimum inspect:

- `crates/executive/src/impl/google/store.rs`
- `crates/mnemosyne/src/backends/gbrain/migrations.rs`
- `crates/executive/src/service/agent_control/sqlite_repository.rs`
- Corpus credential/token storage
- Mnemosyne repositories/backends
- audit/HIL/artifact stores introduced in the current baseline

- [ ] Add a gate that refuses a persistence migration file without an inventory row.
- [ ] Document whether each format supports version detection, idempotent migration, backup/rollback, and fail-closed rejection.

## Task 4: External-name and dependency ratchets

- [ ] Define external product identifiers and allowed adapter/compatibility/deploy/test regions.
- [ ] Count current hits in Fabric, domain/contract/application paths, Executive service/goal paths, Cognit harness, and Kernel.
- [ ] Count public `r#impl` exports and cross-crate `impl` references.
- [ ] Count `contains(provider)`, `match provider_name`, URL provider inference, and provider-error text matching.
- [ ] Count forbidden infrastructure dependencies in domain/application code.
- [ ] Store exact baselines in `metrics.env`; a higher count fails, a lower count requires lowering the baseline.
- [ ] Do not create broad directory allowlists. Every exception records file, reason, canonical replacement, current count, and exit condition.

## Task 5: Fixture-driven architecture gates

Create isolated fixture mutations inside the test script and prove rejection for:

- [ ] external name added to a core path;
- [ ] application importing an adapter;
- [ ] crate root publicly exporting an adapter or `r#impl`;
- [ ] provider-name business branch;
- [ ] unregistered wire/persistence surface;
- [ ] compatibility count exceeding baseline;
- [ ] legal composition factory matching adapter ID remains allowed;
- [ ] opaque JSON payload in an adapter remains allowed while core field inspection fails.

## Task 6: Validation and commit

Run:

```bash
bash tests/architecture_check.sh
bash tests/architecture_path_inventory.sh
bash scripts/architecture-check.sh
```

Expected: all PASS with no business source changes.

Commit stages:

1. `docs(arch): freeze wire and persistence ownership inventories`
2. `test(arch): cover architecture boundary regressions`
3. `chore(arch): enforce ratcheted dependency and naming gates`

## Exit evidence

- All six inventory/baseline files exist and name the frozen commit.
- Every later phase has explicit wire/persistence rows to consult.
- Gate fixtures demonstrate both rejection and legal exceptions.
- No Rust business behavior changed.

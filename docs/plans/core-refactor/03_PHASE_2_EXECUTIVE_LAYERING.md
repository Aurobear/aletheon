# Phase 2 Executive Layering Implementation Plan

> **For DeepSeek:** Execute this plan task-by-task. Do not reinterpret the architecture or combine stages. Check each box only after its evidence exists.

**Goal:** Separate Executive application, adapters, composition, compatibility, and host responsibilities without changing business behavior or public client protocol.

**Architecture:** Create stable facades and ports before moving implementation modules. Migrate one responsibility family at a time, retaining compatibility re-exports with ratcheted call counts.

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

Prerequisite: Phase 0. Do not combine with Phase 3–7 changes.

Primary paths:

- `crates/executive/src/lib.rs`
- `crates/executive/src/core/`
- `crates/executive/src/service/`
- `crates/executive/src/impl/`
- `crates/executive/src/host/`
- `crates/executive/src/user_runtime/`
- Executive architecture/facade tests

## Task 1: Freeze behavior and dependency map

- [ ] Generate a file-to-layer mapping for every Executive production module.
- [ ] Classify each as domain, application, adapter, composition, host, or compatibility.
- [ ] Identify concrete store/provider/parser imports in service/use-case paths.
- [ ] Add tests that freeze request, turn, goal, agent-control, daemon bootstrap, client protocol, and host-launch behavior.
- [ ] Confirm no persistence/wire changes are included in this phase.

## Task 2: Create target facades and application ports

Create only needed roots:

```text
application/
adapters/
composition/
host/
compatibility/
```

- [ ] Define application ports around existing concrete repository/provider dependencies.
- [ ] Re-export stable use-case/facade types from crate root, never whole module trees.
- [ ] Keep domain state in its owning domain crate when possible; do not duplicate it in Executive.
- [ ] Add compile-time tests proving application modules cannot import adapters.

## Task 3: Move composition ownership

Move or facade:

- AppConfig parse/merge/normalization
- environment business variable parsing
- secret resolution
- provider/runtime/backend factories
- registries and bootstrap dependency construction

- [ ] `match adapter_id` exists only in composition registry/factory.
- [ ] Composition produces validated DomainConfig and adapter constructor inputs.
- [ ] Business environment reads remain exclusively in composition.

## Task 4: Move concrete adapters

Classify and migrate concrete code from `impl/`:

```text
external/google/channel/gbrain/runtime repositories and clients -> adapters
```

- [ ] Application sees trait objects or generic ports, not concrete types.
- [ ] Adapter errors normalize at the boundary.
- [ ] Concrete SQLite repositories live under adapters.
- [ ] No adapter owns authorization decisions.

## Task 5: Consolidate host lifecycle

- [ ] Move daemon server, RPC transport, signal/process lifecycle, and host launch wiring under host.
- [ ] Host delegates every use case to application ports.
- [ ] Host does not contain domain rules, provider selection, authorization, or storage-policy name matching.
- [ ] Preserve `CLIENT_PROTOCOL_VERSION` behavior and socket/systemd paths.

## Task 6: Compatibility paths and ratchets

- [ ] Add temporary deprecated re-exports only where current cross-crate callers require them.
- [ ] Every re-export has canonical path, call count, and deletion phase.
- [ ] New imports of `executive::r#impl` fail the architecture gate.
- [ ] Do not physically remove all `impl/` content yet if Phases 3–7 still own semantic migrations.

## Validation

```bash
bash scripts/cargo-agent.sh test -p executive --test private_composition_root
bash scripts/cargo-agent.sh test -p executive --test request_use_case_boundaries
bash scripts/cargo-agent.sh test -p executive --test core_user_boundary
bash scripts/cargo-agent.sh test -p executive --test turn_engine_parity
bash tests/architecture_check.sh
bash tests/systemd_runtime_boundary.sh
```

Expected: PASS with unchanged external behavior and decreasing concrete imports in application paths.

## Commit stages

1. `test(executive): lock application and host behavior`
2. `refactor(executive): establish application and composition facades`
3. `refactor(executive): isolate concrete adapters`
4. `refactor(executive): separate daemon host lifecycle`
5. `chore(arch): prevent Executive layer regressions`

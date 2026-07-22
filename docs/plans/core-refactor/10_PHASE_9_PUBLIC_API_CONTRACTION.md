# Phase 9 Public API Contraction Implementation Plan

> **For DeepSeek:** Execute this plan task-by-task. Do not reinterpret the architecture or combine stages. Check each box only after its evidence exists.

**Goal:** Remove public implementation trees and cross-crate impl paths after semantic migrations are complete, leaving explicit stable facades and counted compatibility exits.

**Architecture:** Inventory all public exports and downstream imports, migrate callers crate-by-crate, make adapters/repositories/parsers private, then physically remove the required impl containers.

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

Prerequisites: Phases 1–7. Phase 8 need not be globally complete, but no active Phase 8 branch may overlap moved paths.

- Crate roots for Cognit, Executive, Mnemosyne, Dasein, Metacog
- Canonical layer/facade module files
- All workspace cross-crate imports
- compatibility debt inventory and architecture gates

## Task 1: Export/import inventory

- [ ] Enumerate every `pub mod`, `pub use`, public concrete adapter/repository/parser, and cross-crate `r#impl` import.
- [ ] Assign a canonical public path or mark private.
- [ ] Record downstream call count and deletion order.
- [ ] Add compile-fail/static gate fixtures for forbidden exports/imports.

## Task 2: Stable crate facades

Each facade may export only:

```text
domain DTO/value objects
stable contract/ports/errors/capabilities
required host/client facade types
testing fakes under an explicit testing feature/module
```

- [ ] No whole adapter/application internal tree is public.
- [ ] No newtype wrapper is added solely to hide an otherwise unstable concrete type.
- [ ] Public docs identify owner and stability of every facade group.

## Task 3: Migrate downstream imports

Order:

1. Cognit callers
2. Mnemosyne callers
3. Executive callers and binary host
4. Dasein/Metacog public callers

- [ ] Replace `crate::r#impl` and concrete adapter paths with canonical facade/port paths.
- [ ] Run the narrow consuming package after each migration.
- [ ] Lower ratchet counts immediately when imports decrease.

## Task 4: Physical impl cleanup

- [ ] Cognit: remove top-level `impl/`, distribute remaining modules into actual needed layers.
- [ ] Executive: remove top-level `impl/` after all adapter/application/composition/host moves complete.
- [ ] Mnemosyne: remove top-level `impl/` and private concrete backends.
- [ ] Dasein/Metacog: stop crate-root public `r#impl`; physical directory split is not mandatory in this phase.
- [ ] Remove compatibility re-exports whose count is zero.

## Task 5: Gate final public surface

- [ ] All five crate roots lack `pub mod r#impl` and `pub use r#impl::*`.
- [ ] Cross-crate `impl` references equal zero.
- [ ] Concrete provider/repository/wire/parser exports equal zero except explicitly stable host facade types.
- [ ] Public API snapshots or rustdoc checks are updated.

## Validation

```bash
bash scripts/cargo-agent.sh check -p cognit
bash scripts/cargo-agent.sh check -p mnemosyne
bash scripts/cargo-agent.sh check -p executive
bash scripts/cargo-agent.sh check -p dasein
bash scripts/cargo-agent.sh check -p metacog
bash tests/architecture_check.sh
bash scripts/cargo-agent.sh doc -p executive --no-deps
```

## Commit stages

1. `docs(api): inventory canonical crate facades`
2. `refactor(api): migrate cross-crate implementation imports`
3. `refactor(cognit): remove public implementation tree`
4. `refactor(mnemosyne): remove public implementation tree`
5. `refactor(executive): remove public implementation tree`
6. `refactor(api): close Dasein and Metacog implementation exports`
7. `chore(arch): enforce stable public facades`

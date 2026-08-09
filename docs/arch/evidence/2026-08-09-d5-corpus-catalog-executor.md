# Agent Kernel V2：D5 Corpus catalog/executor split

Date: 2026-08-09
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-08-domain-authority-and-adapter-extraction.md` §D5
State: Corpus catalog/executor split seam established; no writer cutover, no Executive exec_corpus leak

## Context receipt (runbook §3.1)

```text
Slice: D5 Corpus catalog/executor split
Baseline commit: 0bf690b2
Plan revision: domain-authority §D5
Direct prerequisites: D1 + D2-D4 + K2 (sealed registry) — done
Current authoritative writer: unchanged legacy Corpus ExtensionCatalog + Executive exec_corpus/corpus_group
Target owner/writer: Corpus (catalog) + adapters (executor); cutover later
IDs minted here: none
Production callers: none yet (ports additive)
Test-only callers: 1 catalog/executor test
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: Executive exec_corpus/corpus_group → XRET-02
Unknowns/blockers: none for the seam
Expected files: crates/corpus/src/catalog/ports.rs, catalog/mod.rs, D5 gate
Out-of-scope files: platform/provider/process adapter moves, Kernel registry seal, Executive exec_corpus deletion
```

## 1. What was created

`crates/corpus/src/catalog/ports.rs`:

- **`CatalogEntry`** — rich catalog entry (stays in Corpus).
- **`CatalogPort`** — reads the rich catalog (Corpus owns).
- **`ExecutorPort`** — execution boundary, separated from the catalog (platform/provider/process adapters implement).
- **`ExecutorError`** — typed EntryNotFound / ExecutorUnavailable.
- **`InMemoryCatalog`** / **`EchoExecutor`** — test split.

## 2. Rules honoured (D5)

- **Rich catalog stays in Corpus**: `CatalogPort` is Corpus-owned.
- **platform/provider/process → adapter**: `ExecutorPort` is the separated execution boundary.
- **Kernel registry bootstrap-only, sealed before serving**: the `CapabilityRegistry` (K2) is the sealed bootstrap-only registry the executor binding resolves through (documented; wiring at K4/K6).
- **Delete Executive `exec_corpus`/`corpus_group` business logic**: not in this seam (D5 gate rejects those names); deletion at XRET-02.
- Catalog/executor are separate ports (test-verified).

## 3. D5 gate (architecture-check.sh)

`ARCH_SKIP_D5_GATES` rejects any `exec_corpus`/`corpus_group` reference in the catalog ports production code; requires both `CatalogPort` + `ExecutorPort`.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p corpus        PASS
bash scripts/cargo-agent.sh test -p corpus --lib catalog::ports  PASS (1 passed)
  - catalog_and_executor_are_separate_ports
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Rollback

Delete `ports.rs` + the catalog/mod.rs module line + the D5 gate → exact baseline. No schema, no writer, no executor change.

## 6. Next

D6 (non-extension Fabric root-surface closeout) is gated on E7 (extension Fabric rows cleared) — both are cutover/cleanup slices.

# Phase 9 Public API Contraction Inventory

## Purpose and decision basis

This inventory is the evidence artifact for Phase 9. The governing rule is that a crate facade may expose domain values, contracts/errors/capabilities, required host composition handles, and explicit test support; implementation containers, provider wire models, repositories, and parsers are not stable API (`CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md:508-531`). The removal order is facade first, downstream migration second, visibility contraction third, and physical cleanup last (`CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md:924-938`).

The contraction is necessary because a public module path becomes an architectural dependency even when it was introduced only for convenience. Such paths prevent internal movement, let application code instantiate infrastructure, and propagate provider vocabulary. The corrected dependency direction is:

```text
consumer -> stable crate facade -> application port
                              host -> composition -> private adapter
```

## Final crate surfaces

| Owner | Stable/required facade | Private implementation boundary | Evidence |
|---|---|---|---|
| Cognit | domain components, harness contracts, inference/learning/event/policy host facades | `adapters` is crate-private; `application` is private; no `impl` container | `crates/cognit/src/lib.rs:20-27`, `crates/cognit/src/lib.rs:50-88` |
| Mnemosyne | memory DTO/ports, recall/retention contracts, required local-memory and supplemental host handles | adapters, application, backends, domain and recall internals are private; no `impl` container | `crates/mnemosyne/src/lib.rs:8-28`, `crates/mnemosyne/src/lib.rs:105-139` |
| Executive | application contracts, composition/host entry points, selected root DTOs | adapters and compatibility are crate-private; old `service`, `user_runtime`, and `impl` roots are absent | `crates/executive/src/lib.rs:13-21`, `crates/executive/src/lib.rs:24-40` |
| Dasein | SelfField plus explicit perception/mutation host facades | physical `impl` remains private as allowed for the non-primary refactor scope | `crates/dasein/src/lib.rs:61-85` |
| Metacog | meta-runtime DTO/service contracts and evolution facade | physical `impl` remains private as allowed for the non-primary refactor scope | `crates/metacog/src/lib.rs:1-15` |

`executive::testing` is doc-hidden test characterization access, not a production stability promise (`crates/executive/src/lib.rs:64-95`). Production code must receive the corresponding ports from composition. It must not be used by non-test workspace crates.

## Removed paths and downstream migration order

| Deleted/closed path group | Canonical replacement | Downstream count at exit | Deletion order and reason |
|---|---|---:|---|
| `cognit::impl`, legacy LLM/provider factory facades | `cognit::{inference,learning,event_handlers,policy}` and composition factory | 0 | Cognit first; provider construction must not leak into consumers |
| `mnemosyne::impl` | root memory contracts plus `runtime`/`supplemental` host facades | 0 | Mnemosyne second; storage implementations remain owner-private |
| `executive::impl` | `application`, `composition`, `host`, root DTO facade | 0 | Executive third after domain facades existed |
| `executive::service` | `executive::application` | 0 | test and workspace callers migrated before facade deletion |
| `executive::core::config` | `executive::composition::config` | 0 | config is composition input, not a domain-core concern |
| `executive::user_runtime` | `executive::composition::user_runtime` | 0 | runtime assembly belongs to composition |
| application-owned concrete turn/session/repository constructors | composition functions or injected ports | 0 | removed after black-box callers used `executive::testing` helpers |
| Dasein/Metacog public `r#impl` roots | explicit crate-root facades | 0 | closed last; physical split intentionally deferred by scope |

The machine-counted compatibility ledger now contains no Phase 9 exits (`config/architecture/compatibility-debt.tsv:1-6`). Persisted wire/schema names remain only as Phase 10 migration debt because renaming them without a value-preserving migration would corrupt compatibility.

## Static gates and snapshots

- Root/public implementation exports and cross-crate implementation imports are counted by `scripts/aletheon.sh acceptance architecture` and ratcheted in `config/architecture/metrics.env`.
- The complete Executive layer snapshot is `config/architecture/executive-layers.tsv`; crate public-module snapshots are `config/architecture/module-boundaries.txt`.
- Canonical path ownership is frozen in `config/architecture-path-inventory.txt`; exceptions are counted in `config/architecture-allowlist.txt` and `config/architecture/compatibility-debt.tsv`.
- Phase 9 exit values are `CROSS_CRATE_IMPL_REFERENCES=0` and `PUBLIC_IMPL_ADAPTER_EXPORTS=0` (`config/architecture/metrics.env`).

## Validation evidence

Executed on 2026-07-23:

- `bash scripts/cargo-agent.sh check -p cognit` — passed.
- `bash scripts/cargo-agent.sh check -p mnemosyne` — passed.
- `bash scripts/cargo-agent.sh check -p executive` — passed.
- `bash scripts/cargo-agent.sh check -p dasein` — passed.
- `bash scripts/cargo-agent.sh check -p metacog` — passed.
- `bash scripts/cargo-agent.sh test -p executive --no-run` — passed.
- `ARCH_PRINT_PHASE0_METRICS=1 bash scripts/aletheon.sh acceptance architecture` — passed with zero cross-crate impl references and zero public impl/adapter exports.
- `bash scripts/aletheon.sh test architecture` — passed.
- `bash scripts/cargo-agent.sh doc -p executive --no-deps` — passed; a stale rustdoc link found during the run was corrected to the host facade.

The full workspace build/test matrix and final architecture audit belong to Phase 10 and must not be inferred from these focused results.

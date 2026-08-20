# Agent Kernel V2：CGP-01 aletheon composition skeleton

Date: 2026-08-09
Baseline: `0bf690b2`
State: composition skeleton established as an additive seam; **legacy launcher still the live path** (no official socket cutover)

## Context receipt (runbook §3.1)

```text
Slice: CGP-01 aletheon composition skeleton
Baseline commit: 0bf690b2
Direct prerequisites: RA-01 (Runtime facade) + K1/K2 + APX-01 (Application) — done
Current authoritative composition: legacy executive::host::launcher::run_daemon (main.rs:442)
Target owner/writer: aletheon composition root; not yet the live path
IDs minted here: none
Production callers: none yet (skeleton additive; legacy launcher untouched)
Test-only callers: 2 skeleton tests
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none
Expected files: crates/aletheon/src/composition.rs, lib.rs mod
Out-of-scope files: all socket cutovers, CGP-04+
```

## 1. What was created

`crates/aletheon/src/composition.rs` — the CGP-01 composition skeleton:

- **`LifecyclePhase`**: Preflight → Open → Compose → Bootstrap → Serve → Drain.
- **`OwnerConfig`**: per-owner normalized config (data_dir), not one Executive `AppConfig`.
- **`ComponentHandle`**: explicit typed handle (`name` + `owner`) — **no `ComponentGraph`/`ServiceBag`**.
- **`ComposedRuntime`**: pure result of `compose` (holds explicit handles).
- **`CompositionSkeleton`**: `preflight` (pure validation) → `open` (accept explicit handles) → `compose` (**pure wiring, no I/O/spawn**) → `mark_ready` (bootstrap placeholder). `serve`/`drain` are declared but not implemented — the legacy launcher remains authoritative.

## 2. Rules honoured (CGP-01)

- **No ComponentGraph/ServiceBag**: every component is an explicit `ComponentHandle` (CGP-01 gate rejects both names in code).
- **`compose` does no I/O and no spawn**: it only clones explicit handles into `ComposedRuntime` (gate rejects `Command::new`/`.spawn()`/`std::fs`/`UnixStream`/`UnixListener`/`tokio::net`).
- Config normalized per owner, not a single Executive `AppConfig` (`OwnerConfig.owner` slice).
- Explicit lifecycle phases: `preflight/open/compose/bootstrap/serve/drain` present.
- Legacy launcher one-way: the skeleton is additive; `main.rs` still calls `executive::host::launcher::run_daemon` (no official socket cutover).

## 3. CGP-01 gate (architecture-check.sh)

`ARCH_SKIP_CGP01_GATES` rejects (Rust code only, comments exempt):
- any `ComponentGraph`/`ServiceBag` symbol;
- any I/O/spawn in compose (`Command::new`/`.spawn()`/fs/socket/tcp);
- missing any of the six lifecycle phases.

Mutation-tested: adding `pub struct ComponentGraph;` rejected.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p aletheon       PASS
bash scripts/cargo-agent.sh test -p aletheon --lib composition  PASS (2 passed)
  - preflight_rejects_empty_owner
  - compose_is_pure_and_returns_handles
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Rollback

Delete `composition.rs` + the lib.rs `pub mod composition;` + the CGP-01 gate → exact baseline. The legacy launcher path is untouched by this seam.

## 6. Next

CGP-01 done → **CGP-03** (typed route handlers), **CGP-04** (official socket cutover) are unblocked on the composition axis; CGP-02 (gateway protocol/client) already landed.

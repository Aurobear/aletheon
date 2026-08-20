# Agent Kernel V2：D1 contracts seed（ownerless primitives）

Date: 2026-08-09
Baseline: `0bf690b2`
State: gated ownerless-primitive seed established in fabric; no package rename, no rich-type move, no writer cutover

## Context receipt (runbook §3.1)

```text
Slice: D1 minimal contracts seed
Baseline commit: 0bf690b2
Direct prerequisites: D0 (B2/B4 closure) — done
Current authoritative writer: unchanged legacy Fabric rich-type surface
Target owner/writer: unchanged (seed is additive re-export only)
IDs minted here: none
Production callers: none new (re-export of existing types)
Test-only callers: none
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none
Expected files: crates/fabric/src/contracts.rs, lib.rs mod, D1 gate
Out-of-scope files: all rich types, repositories, workflows, AK2-25 rename
```

## 1. What was created

`crates/fabric/src/contracts.rs` — a gated ownerless-primitive module that **re-exports only** existing ownerless value semantics:

- `PermitId`, `PrincipalId` (admission)
- `RuntimeId` (attempt)
- `MessageId` (channel)
- `OperationId`, `ProcessId` (operation)
- `AgentId`, `NamespaceId` (process)
- `TurnId` (session)
- `SessionId` (space)
- `SchemaId` (envelope_v2)
- `Version` (subsystem)

Registered as `pub mod contracts;` in `crates/fabric/src/lib.rs`.

## 2. Constraints honoured (plan §11.1 / §10.3)

| constraint | status |
|---|---|
| establish ownerless primitive module inside fabric | done |
| no package rename in this commit | honored (no Cargo change; `AK2-25` remains separate) |
| no rich aggregate/repository/service/workflow move | honored (pure re-export) |
| contracts seed has no workspace dependency | honored (re-exports within fabric only) |
| Fabric rich public surface does not grow | **verified**: `fabric-public-types.tsv` stays 1110; contracts.rs adds 0 new public symbols |
| every symbol proven ownerless value semantics | 10/10 re-exports are ID wrappers / `Version`; no codec/repository/policy/live-permit/state-machine/UI-wire |

## 3. D1 gate (architecture-check.sh)

`ARCH_SKIP_D1_GATES` block rejects:
- any new `pub struct/enum/trait/type` declaration in contracts.rs (must be re-export only);
- any import from a rich module (`types::repository/tool/permission/workspace/goal/objective`, `policy`, `security`, `kernel`, `ipc::bus`, `events::spine`);
- any `impl` block (behavior);
- any re-export outside the exact allowed ownerless-primitive list.

Mutation-tested: adding `pub use crate::types::repository::Repository` and `pub struct NewPrimitive` both rejected.

## 4. Fabric count change (D0 gate updated)

Fabric source set 174 → 175 (the D1 seed). This is the **only** permitted fabric source growth before AK2-25. Public surface (1110 symbols) unchanged. D0 gate updated to expect 175 with a comment pinning the D1 seed as the sole allowed growth.

## 5. Validation

```text
bash scripts/cargo-agent.sh check -p fabric        PASS (4.85s)
bash scripts/cargo-agent.sh check -p gateway-client PASS (0.29s)
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps, 1110 public types)
```

## 6. Rollback

Remove `contracts.rs` + the `pub mod contracts;` line + the D1 gate block → exact baseline restored. No wire tag, no schema, no writer touched.

## 7. Next

D1 done → **RA-01** (Runtime commands/events/queries/ID owner) and **K1** (durable Operation authority) are now unblocked on the D0/D1 axis.

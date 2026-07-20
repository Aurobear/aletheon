# Aletheon Current Architecture & Coupling Analysis

> **Status:** Current verified snapshot
>
> **Verified:** 2026-07-19
>
> **Scope:** Workspace crate boundaries and direct local dependencies after the
> crate-consolidation pass. Claims about the current tree are anchored to code.

## 1. Summary

Aletheon currently has 16 production or explicitly experimental domain crates
and two workspace examples. The canonical member list is `Cargo.toml:3-21`.

The consolidation removed layer-shaped crates and retained domain-shaped
boundaries:

```text
platform-api + platform-host + per-OS crates  -> platform
runtime-api + runtime-broker                  -> runtime
hardware-api + hardware-sim                   -> hardware
exec-server                                   -> execd
aletheon-bin                                  -> aletheon
aletheon-kernel                               -> kernel
```

`mcp-types`, `state-authority` and `coding-bench` were removed because they had
no production caller and duplicated existing ownership.

The architecture is now smaller, but consolidation alone does not prove that a
boundary is production-ready. `hardware` and most of the new `runtime`
selection surface remain incomplete; they are tracked as experimental rather
than counted as delivered capability.

## 2. Direct Dependency Shape

```text
aletheon
  |-- executive
  |-- interact --------> executive
  `-- fabric

executive
  |-- kernel ----------> fabric
  |-- runtime
  |-- cognit ----------> kernel, fabric
  |-- corpus ----------> cognit, kernel, mnemosyne, platform, fabric
  |-- dasein ----------> kernel, fabric
  |-- agora -----------> kernel, fabric
  |-- mnemosyne -------> kernel, fabric
  |-- metacog ---------> kernel, fabric
  |-- gateway ---------> fabric
  `-- fabric

execd -----------------> corpus

platform                (no local dependency)
runtime                 (no local dependency)
hardware                (no local dependency)
```

Evidence:

- Executive's direct domain dependencies are declared at
  `crates/executive/Cargo.toml:9-19`.
- Corpus currently depends on Cognit, Mnemosyne and Platform at
  `crates/corpus/Cargo.toml:9-14`.
- Execd currently depends on the whole Corpus crate at
  `crates/execd/Cargo.toml:8-15`.
- Platform owns its contracts and OS backends in one crate at
  `crates/platform/src/lib.rs:1-38`.
- Runtime owns lifecycle contracts, registry and selector at
  `crates/runtime/src/lib.rs:1-31`.
- Hardware currently exposes device contracts and simulator at
  `crates/hardware/src/lib.rs:1-29`.

## 3. Crate Inventory

The source-size figures below count checked-in Rust files and lines in the
current tree. They are diagnostic signals, not quality scores.

| Crate | Rust files | Rust lines | Direct local dependencies | Role |
|---|---:|---:|---|---|
| `aletheon` | 11 | 1,449 | executive, fabric, interact | user entry and assembly |
| `executive` | 399 | 109,293 | 10 domain crates | composition and orchestration |
| `kernel` | 25 | 5,681 | fabric | lifecycle and governance primitives |
| `runtime` | 8 | 313 | none | external executor contract and selection |
| `cognit` | 68 | 19,358 | fabric, kernel | cognition and harness |
| `corpus` | 147 | 45,927 | cognit, fabric, kernel, mnemosyne, platform | tools and capability execution |
| `dasein` | 77 | 19,160 | fabric, kernel | identity and continuity |
| `agora` | 16 | 5,167 | fabric, kernel | shared cognitive workspace |
| `mnemosyne` | 90 | 24,201 | fabric, kernel | memory and experience |
| `metacog` | 28 | 4,408 | fabric, kernel | governed evaluation and evolution |
| `fabric` | 150 | 31,257 | none | shared protocol and compatibility infrastructure |
| `gateway` | 16 | 3,242 | fabric | external request adapter |
| `interact` | 52 | 12,815 | executive, fabric | TUI and interaction adapter |
| `platform` | 38 | 2,011 | none | host OS contracts and backends |
| `execd` | 6 | 2,447 | corpus | isolated low-level side effects |
| `hardware` | 2 | 191 | none | experimental device contract and simulator |

## 4. Coupling Findings

### 4.1 Executive is intentionally broad, but still oversized

Executive depends on almost every active domain
(`crates/executive/Cargo.toml:9-19`). That breadth is expected for a composition
root. Its 109,293 Rust lines show that it also owns substantial implementation,
not only assembly.

The boundary is healthy only if new work follows this rule:

```text
Executive may coordinate a domain.
Executive must not reimplement that domain's mechanism.
```

Examples:

- process/admission/supervision mechanisms belong to Kernel;
- reasoning algorithms belong to Cognit;
- concrete tools belong to Corpus;
- host OS primitives belong to Platform;
- external executor lifecycle contracts belong to Runtime.

### 4.2 Fabric remains a large dependency funnel

Fabric has no local dependency, but most established domains depend on it. It
contains 31,257 Rust lines, so it is materially larger than a pure ID/schema
crate.

This is a controlled legacy boundary, not permission to add arbitrary shared
types. New public contracts must have a verified cross-domain owner and must not
be placed in Fabric merely to avoid deciding ownership.

### 4.3 Corpus still leaks configuration ownership into Cognit

Corpus owns the MCP client, transport, authentication and manager, but re-exports
its server configuration from Cognit at
`crates/corpus/src/tools/mcp/config.rs:54-56`. This creates the direct
`corpus -> cognit` edge visible at `crates/corpus/Cargo.toml:10`.

Current reality and intended ownership disagree:

| Concern | Current owner | Correct domain owner |
|---|---|---|
| MCP execution | Corpus | Corpus |
| MCP transport/auth | Corpus | Corpus |
| MCP configuration schema | Cognit | Corpus |
| top-level configuration assembly | Executive | Executive |

This edge should be removed by moving the canonical MCP schema into Corpus, not
by recreating a standalone `mcp-types` crate.

### 4.4 Execd has a justified process boundary but an oversized library edge

Execd is a separate process because it provides failure and permission
isolation. That justifies an independent binary and crate. It now depends on the
minimal Platform filesystem/patch contract rather than all of Corpus.

Target dependency shape:

```text
Executive -> execd -> platform + minimal patch contract
```

The target must not create `platform -> corpus`, because Corpus already depends
on Platform.

### 4.5 Runtime and Executive now have distinct semantics

Runtime exports only capability manifests and deterministic selector contracts
(`crates/runtime/src/lib.rs:1-8`). Executive owns the registry, runtime instances
and the real Pi adapter; Fabric `AgentResult` owns bounded output/usage/evidence,
while Executive Goal verification owns coding report/diff persistence.

The separation is:

```text
Executive: choose, admit, supervise, verify and settle globally.
Runtime:   describe selectable external runtime capabilities.
Adapter:   execute under Executive and return Fabric AgentResult.
```

AgentResult and coding evidence are not final authority. Executive independently
persists and verifies attempt identity and diff hashes before settlement.

### 4.6 Platform is correctly consolidated

Platform exposes host contracts, selector, registry and all three OS backend
modules from one crate (`crates/platform/src/lib.rs:6-38`). Per-OS modules are
compile-time implementation details, not independent domains.

A future backend may become a separate crate only when a concrete system SDK,
independent release cycle or build isolation requirement makes that boundary
measurably useful.

### 4.7 Hardware is experimental

Hardware has only two Rust files and no production caller. Its source-level
contract and simulator are useful as a design seed, but they do not establish a
production Hardware Control Platform.

It must remain experimental until a vertical path exists:

```text
Executive Operation
  -> Kernel Capability Permit
  -> Hardware lease/deadline/safety validation
  -> provider or simulator
  -> independent receipt and evidence
```

No ROS, CAN, serial or vendor crate should be created before that path has a
real integration requirement.

## 5. Dependency Invariants

The following constraints are enforced or intended to be enforced by
`scripts/architecture-check.sh` and
`config/architecture-dependencies.txt`:

1. Fabric must not depend on higher-level domains.
2. Kernel may depend on Fabric, but not Cognit, Corpus, Executive or Platform.
3. Runtime contracts must not depend on Executive.
4. Platform must not depend on Corpus or Executive.
5. Hardware core must remain independent of ROS, CAN, serial and vendor SDKs.
6. Corpus may invoke Platform but does not grant itself permission.
7. Execd performs approved operations but does not own Agent or task policy.
8. Executive is the composition root and global completion authority.

## 6. Current Priorities

In dependency order:

1. Move MCP configuration ownership from Cognit to Corpus.
2. Reduce Execd's dependency from full Corpus to the smallest real patch
   contract without creating a cycle.
3. Wire Runtime registry/selection into the real external-runtime production
   path while retaining Executive verification.
4. Prove Hardware through one governed simulator vertical slice before adding
   providers.
5. Build a real coding benchmark under `tests/coding` using fixture repositories,
   Executive execution, independent acceptance commands and replayable receipts.
6. Continue shrinking Executive implementation ownership without creating
   layer-shaped crates.

## 7. Crate Admission Gate

A new crate is rejected unless its proposal identifies all of:

- stable domain owner;
- real production caller;
- dependency direction;
- reason an internal module is insufficient;
- independent compilation, dependency isolation, deployment or security value;
- deterministic validation command;
- production or explicit experimental status.

Names such as `*-api`, `*-types`, `*-common`, `*-broker` and speculative provider
names are not boundaries by themselves.

## 8. Conclusion

The workspace is no longer suffering from the specific crate explosion created
by the discarded plans. The remaining risk is semantic duplication inside the
larger established crates and incomplete production wiring in the smaller new
domains.

The next phase must optimize for one owner and one real call path per system
semantic, not for a larger module inventory.

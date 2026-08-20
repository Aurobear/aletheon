# Agent Kernel V2：K2 sealed descriptor + `CapabilityExecutor`

Date: 2026-08-09
Baseline: `0bf690b2`
State: sealed descriptor + CapabilityExecutor registry established; legacy `DefaultCapabilityInvoker` stays authoritative; no traffic cut

## Context receipt (runbook §3.1)

```text
Slice: K2 sealed descriptor + CapabilityExecutor
Baseline commit: 0bf690b2
Direct prerequisites: K1 (durable seam) + D1 (contracts) — done
Current authoritative writer/executor: unchanged legacy DefaultCapabilityInvoker (capability/mod.rs)
Target owner/writer: Kernel (sealed CapabilityRegistry); not yet wired
IDs minted here: none — reuses fabric::CapabilityId (no new ID type)
Production callers: none yet (registry is additive; legacy invoker untouched)
Test-only callers: 3 registry tests (NoopExecutor)
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none (NoopExecutor is side-effect-free)
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none
Expected files: crates/kernel/src/capability/registry.rs, capability/mod.rs, K2 gate
Out-of-scope files: all writer/executor cutovers, K3+
```

## 1. What was created (kernel plan §10 K2)

`crates/kernel/src/capability/registry.rs`:

- **`SealedDescriptor`**: `capability: fabric::CapabilityId`, `version: u64`, `digest: InvocationDigest` — pins the exact executable semantics.
- **`CapabilityExecutor` trait**: `execute(&SealedDescriptor, input) -> Value`.
- **`NoopExecutor`**: the representative side-effect-free executor (K2: "先接一个无外部副作用的 representative executor").
- **`CapabilityRegistry`**: `register` (rejects duplicate id, version 0, empty digest) + `seal` (immutable after seal) + `resolve`.

## 2. Rules honoured (kernel plan §10 K2)

- Composition root builds the registry and validates duplicate/version/digest, then seals (`seal()` makes bindings immutable — `register` after seal fails).
- Representative NoopExecutor first; Corpus/Gmail/Hardware/Robot come at the joint K6/E-series cutovers (E2-K6a/E4-K6b/E5-K6c/E6-K6d).
- **No arbitrary-executor injection**: after seal, `resolve` only returns sealed bindings; nothing can replace them (K2: "删除 Runtime/Executive 传入 arbitrary executor 的能力").
- No new/double writer: legacy `DefaultCapabilityInvoker` stays authoritative; this registry is not wired into the invocation path yet.
- Reuses `fabric::CapabilityId` — no duplicate ID type (id-collisions gate clean).

## 3. K2 gate (architecture-check.sh)

`ARCH_SKIP_K2_GATES` requires the registry to contain: post-seal immutability guard, version>=1 validation, digest validation, duplicate rejection, `seal()`, `resolve()`, and the module declared in `capability/mod.rs`. Mutation-tested via `seal_is_immutable` unit test.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p kernel        PASS (0.66s)
bash scripts/cargo-agent.sh test -p kernel --lib registry  PASS (3 passed)
  - register_seal_resolve_roundtrip
  - duplicate_capability_rejected
  - seal_is_immutable
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Rollback

Delete `registry.rs` + the `pub mod registry;` line + the K2 gate → exact baseline. No schema, no writer, no executor, no daemon change.

## 6. Next

K2 done → **K3** (authorization evidence verifier). The registry/seal seam is the K4 single admit/invoke/receipt/recovery path's descriptor source.

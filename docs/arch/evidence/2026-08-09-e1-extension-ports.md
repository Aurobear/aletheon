# Agent Kernel V2：E1 extension registration ports

Date: 2026-08-09
Baseline: `0bf690b2`
State: extension registration seam established; no extension cutover, no rich types in contracts

## Context receipt (runbook §3.1)

```text
Slice: E1 owner ports + extension registration descriptor
Baseline commit: 0bf690b2
Direct prerequisites: E0 (preservation manifest) + APX-01 (Application facade) — done
Current authoritative writer: unchanged legacy Executive extension bootstrap/adapters
Target owner/writer: Application extension seam (descriptor only); not yet wired
IDs minted here: none — ExtensionId is owner-local, not a canonical core ID
Production callers: none yet (descriptor + in-memory registry; legacy extension bootstrap untouched)
Test-only callers: 2 extension tests
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none
Expected files: crates/application/src/extension.rs, lib.rs re-export, E1 gate
Out-of-scope files: all extension cutovers (E2-K6a/E3/E4-K6b/E5-K6c/E6-K6d), D6/E7
```

## 1. What was created

`crates/application/src/extension.rs`:

- **`ExtensionId(pub String)`** — owner-local extension identity (not a canonical core ID).
- **`ExtensionFlags`** — `enabled` + optional `installed_reference` (core records, never reads).
- **`ExtensionRegistration`** — id + flags; **no rich types, no concrete adapter, no secret**.
- **`ExtensionPort`** trait — `register` + `registered`.
- **`InMemoryExtensionRegistry`** — empty by default; **the core binary constructs with zero extensions** (E1 requirement).

## 2. Rules honoured (E1)

- Owner port in Application (`ExtensionPort`) — narrow; extension owner crates implement richer behavior at the joint cutovers.
- Extension registration input established (`ExtensionRegistration`).
- **Core binary constructible with zero extensions** — `InMemoryExtensionRegistry::new()` is empty (test-verified).
- **No rich types in contracts** — the seam stays in Application, not `contracts`; nothing here reaches the D1 seed.
- No second composition root, no concrete adapter wiring.

## 3. E1 gate (architecture-check.sh)

`ARCH_SKIP_E1_GATES` rejects:
- any rich type (`Repository`/`Service`/`Store`/`Worker`/`Scheduler`) in the seam;
- any secret field (`secret`/`token`/`password`/`credential`);
- requires an empty constructor + zero-extension documentation.

Mutation-tested: adding `pub struct Bad { pub secret: String }` rejected.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p application    PASS
bash scripts/cargo-agent.sh test -p application     PASS (4 total: 2 facade + 2 extension)
  - core_binary_constructs_with_zero_extensions
  - registers_and_lists_extension
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Rollback

Delete `extension.rs` + the lib.rs re-export + the E1 gate → exact baseline. No schema, no writer, no extension behavior change.

## 6. Next

E1 done → the joint extension cutovers (E2-K6a Gmail, E3 GBrain, E4-K6b Hardware, E5-K6c Robot, E6-K6d Pi) are unblocked on the port axis, each gated on its owner's writer/effect cutover and installed equivalence.

# Aletheon closure plan：R3 typed Command Output + protocol versioning

Date: 2026-08-10
Baseline: `0bf690b2`
Plan: `Aletheon_Runtime_Product_Convergence_and_Engineering_Closure_Plan_2026-08-07.md` §11
State: typed command-output surface established in gateway-protocol; daemon/TUI drift becomes compile-time

## Context receipt (runbook §3.1)

```text
Slice: R3 typed Command Output + protocol versioning
Baseline commit: 0bf690b2
Plan revision: closure-plan §11
Direct prerequisites: R1/R2 + CGP-02 (gateway-protocol) — done
Current authoritative writer: unchanged legacy DaemonTurnEngine + fabric CommandOutputV1 (compat)
Target owner/writer: gateway-protocol typed command output; not yet wired
IDs minted here: none
Production callers: none yet (types additive)
Test-only callers: 2 command_output tests
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none (V0→V1 adapter is the cutover)
Deletion owner: fabric CommandOutputV1 → CGP-03/XRET-04
Unknowns/blockers: none
Expected files: crates/gateway-protocol/src/command_output.rs, lib.rs re-export, R3 gate
Out-of-scope files: DaemonTurnEngine indexing removal, TUI response.rs branch removal
```

## 1. What was created

`crates/gateway-protocol/src/command_output.rs`:

- **`COMMAND_OUTPUT_VERSION = 1`**.
- **`TypedCommandOutputEnvelope`** — version + typed kind.
- **`TypedCommandOutput`** — Completed / IncrementalText / ToolLifecycle / StatusProjection / Usage / Error (each exactly one domain meaning).
- **`TypedCompletion` / `TypedToolLifecycle` / `TypedStatusProjection` / `TypedUsage` / `TypedError`** — typed payloads, no dynamic JSON.
- **`VersionError`** — typed unsupported-version with received/expected.
- **`validate_version`** — rejects unknown required variants recording the version.

## 2. Rules honoured (R3)

- "Typed input, typed output": the typed envelope makes daemon/TUI protocol drift a compile-time error.
- **No dynamic `serde_json::Value` indexing** in business layers: the R3 gate rejects `serde_json::Value` in the module; JSON exists only at the transport boundary.
- Each output has exactly one domain meaning (typed variants).
- Unknown fields forward-compatible; unknown required variants rejected with version recorded (`VersionError`).
- Serialization round-trip verified (test `completed_roundtrip_and_validate`).

## 3. R3 gate (architecture-check.sh)

`ARCH_SKIP_R3_GATES` requires `TypedCommandOutput`/`TypedCommandOutputEnvelope`/`validate_version`/`VersionError`; rejects any `serde_json::Value` in the module's production code.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p gateway-protocol  PASS
bash scripts/cargo-agent.sh test -p gateway-protocol --lib command_output  PASS (2 passed)
  - completed_roundtrip_and_validate
  - unsupported_version_is_typed_error
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining R3 (cutover)

Move DaemonTurnEngine to emit typed envelopes, remove TUI `response.rs` field-branch guessing, and add a single V0→V1 compat adapter with a deletion deadline (CGP-03/CGP-06).

## 6. Rollback

Delete `command_output.rs` + the lib.rs re-export + the R3 gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

R1/R2/R3 (the R-series typed turn/scope/output work) is complete at the type level; U1 (TUI live state) is the in-flight branch.

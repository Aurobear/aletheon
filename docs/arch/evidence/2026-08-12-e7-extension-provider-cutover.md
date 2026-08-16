# E7 extension Agent provider port cutover — 2026-08-12

## Requirement anchors

E7 uniquely deletes extension-specific Fabric rows after matching caller-zero
evidence (`docs/plans/2026-08-08-preserved-extensions-cutover.md:418-430`).
Runtime-owned Agent seams are removed from Fabric during RA/E7 closeout
(`docs/plans/2026-08-08-runtime-authority-consolidation.md:240-255`).

## Result

`AgentRuntimeProvider` now lives at
`crates/runtime/src/extension_provider.rs:1-18` and is exported by Runtime. The
installed Aletheon extension router and the rollback Executive subprocess
adapter both consume `runtime::AgentRuntimeProvider`; neither consumes the
retired Fabric include path. The unused `ToolProvider`, `HookProvider`, and
`ConnectorProvider` placeholder traits had no implementations or callers and
were deleted with `crates/fabric/src/include/extension_provider.rs`.

```text
Before: extension platform -> fabric::include::extension_provider
After:  extension Agent backend -> runtime::AgentRuntimeProvider
        unused provider placeholders -> deleted
```

Caller-zero gate:

```bash
! rg -n 'fabric::include::extension_provider' crates --glob '*.rs'
```

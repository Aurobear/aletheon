# Architecture documentation

Current architecture references:

- [Architecture overview](../design/architecture-overview.md)
- [Coupling diagnosis (2026-08-16)](evidence/2026-08-16-architecture-coupling-diagnosis.md)
- [Coupling closeout plan](../plans/2026-08-16-architecture-coupling-closeout.md)
- [Public API contraction inventory (historical Phase 9 snapshot)](PUBLIC_API_CONTRACTION_INVENTORY.md)
- [Metacog feature architecture](../../crates/metacog/README.md)
- [Metacog persistence operations](../deployment/metacog-problem-ledger.md)

## Metacog boundary

```text
capability domain -> Contracts metacognition ABI -> Metacog
                                                |
                                                v
                                 governed evolution boundary
```

Metacog uses feature-owned modules directly under `crates/metacog/src/`
(`genome`, `governance`, `evolution`, `evaluation`, `evidence`, `experience`,
`improvement`, `problem`, `reflection`, `adapters`). The old technical-layer
roots `core`, `bridge`, and `impl` were removed during the architecture
decoupling refactor and must not return.
Coding-specific contracts remain in the host runtime adapter under
`crates/aletheon/src/wiring/adapters/runtime/`.

# Architecture documentation

Current architecture references:

- [Architecture overview](../design/architecture-overview.md)
- [Public API contraction inventory](PUBLIC_API_CONTRACTION_INVENTORY.md)
- [Metacog feature architecture](../../crates/metacog/README.md)
- [Metacog persistence operations](../deployment/metacog-problem-ledger.md)

## Metacog boundary

```text
capability domain -> Fabric metacognition ABI -> Metacog
                                                |
                                                v
                                 governed evolution boundary
```

Metacog uses feature-owned modules directly under `crates/metacog/src/`
(`genome`, `governance`, `evolution`, `evaluation`, `evidence`, `experience`,
`improvement`, `problem`, `reflection`, `adapters`). The old technical-layer
roots `core`, `bridge`, and `impl` were removed during the architecture
decoupling refactor and must not return.
Coding-specific contracts remain in the Executive-side adapter.

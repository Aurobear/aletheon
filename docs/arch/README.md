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

Metacog uses feature-owned modules. Deleted technical-layer roots named
`core`, `bridge`, and `impl` must not return under `crates/metacog/src`.
Coding-specific contracts remain in the Executive-side adapter.

# Brain Crate (`aletheon-brain`)

> The reasoning and inference layer — cognitive engine, hybrid inference routing, and LLM provider management.

**Crate:** `aletheon-brain`
**Source:** `crates/aletheon-brain/`
**Last updated:** 2026-06-14

---

## Crate Structure

```
crates/aletheon-brain/
├── core/           — Shared types and traits
├── bridge/         — Cross-crate integration points
├── impl/
│   ├── inference/  — IntentClassifier, InferenceRouter, ProviderConfig
│   └── mod.rs
└── testing/        — Test utilities
```

## Documents

| Document | Scope |
|----------|-------|
| [cognitive-engine.md](cognitive-engine.md) | ReAct reasoning loop, content-block message protocol, streaming |
| [inference.md](inference.md) | Hybrid inference — local/cloud routing, intent classification, provider config |

## Internal Pattern

Each `impl/` module follows the core/bridge/impl/testing pattern:

- **core/** — shared types and trait definitions
- **bridge/** — cross-crate integration points
- **impl/** — concrete implementations
- **testing/** — test utilities and mocks

# Cognit Crate — Cognitive Engine

> The reasoning and inference layer — cognitive engine, hybrid inference routing, and LLM provider management.

**Crate:** `cognit`
**Source:** `crates/cognit/`
**Last updated:** 2026-07-31

---

## Crate Structure

```
crates/cognit/
├── core/              — Shared types and traits (Reasoner, Planner, Critic, Reflector, Learner)
├── bridge/            — Cross-crate integration (inference, dual-model, LLM, learning)
├── application/       — Use cases (inference router, classifier, provider config)
├── adapters/          — External integrations (inference providers, policy)
├── harness/           — Cognitive sessions (linear/ReAct, robot)
│   ├── linear/        — ReAct loop: step, session, circuit_breaker, goal_tracker, etc.
│   └── robot/         — RobotHarness deterministic state machine
├── composition/       — DI assembly, provider registry
├── config/            — Harness configuration
└── ports/             — Public traits for external consumers
```

The old `impl/` directory was removed during the architecture decoupling
refactor. Concrete implementations now live in `application/` and `adapters/`.

## Documents

| Document | Scope |
|----------|-------|
| [cognitive-engine.md](cognitive-engine.md) | ReAct reasoning loop, content-block message protocol, streaming |
| [inference.md](inference.md) | Hybrid inference — local/cloud routing, intent classification, provider config |

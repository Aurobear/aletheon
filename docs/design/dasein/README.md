# Dasein Crate — Self Model

> The self-awareness and self-protection layer — hooks, security, resilience, and perception.

**Crate:** `dasein`
**Source:** `crates/dasein/`
**Last updated:** 2026-06-14

---

## Crate Structure

```
crates/dasein/
├── core/           — Shared types and traits
├── bridge/         — Cross-crate integration points
├── impl/
│   ├── hook/       — Hook system (21 event types, trust model, config layers)
│   ├── resilience/ — DaemonGuardian, WatchdogTimer, SafeMode, crash recovery
│   ├── security/   — InputSanitizer, ResourceGovernor, EmergencyKillswitch, IntegrityMonitor
│   ├── perception/ — PerceptionManager, EventAggregator, sources, FUSE
│   └── mod.rs
└── testing/        — Test utilities
```

## Documents

| Document | Scope |
|----------|-------|
| [self-field.md](self-field.md) | SelfField architecture — 8 internal layers, review() pipeline |
| [hook-system.md](hook-system.md) | Hook event types, trust model, config layers, command execution |
| [loop-detector.md](loop-detector.md) | LoopDetector, RiskClassifier, CircuitBreaker, OutputGuardrail, per-agent isolation |
| [writable-root.md](writable-root.md) | WritableRoot path isolation, FileSystemSandboxPolicy, PathAccessGuard |
| [self-protection.md](self-protection.md) | InputSanitizer, ResourceGovernor, EmergencyKillswitch, IntegrityMonitor |
| [resilience.md](resilience.md) | Error handling, panic recovery, rate limiting, backpressure |
| [perception-sources.md](perception-sources.md) | System service management, perception source integration |

## Internal Pattern

Each `impl/` module follows the core/bridge/impl/testing pattern:

- **core/** — shared types and trait definitions
- **bridge/** — cross-crate integration points
- **impl/** — concrete implementations
- **testing/** — test utilities and mocks

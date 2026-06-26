> New document — code paths reflect aletheon-* crate structure

# SelfField Architecture

> The self-awareness layer — 8 internal layers for identity, boundary, care, narrative, conflict, attention, continuity, and mutation.

**Crate:** `aletheon-self`
**Source:** `crates/aletheon-self/`
**Last updated:** 2026-06-14

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| Hook system | ✅ Implemented | `impl/hook/` | 21 event types, 3-layer TOML config |
| Resilience | ✅ Implemented | `impl/resilience/` | Guardian, watchdog, safe mode |
| Security (self-protection) | ✅ Implemented | `impl/security/` | InputSanitizer, ResourceGovernor, EmergencyKillswitch, IntegrityMonitor |
| Perception | ✅ Implemented | `impl/perception/` | PerceptionManager, EventAggregator, sources |
| Core types | ✅ Implemented | `core/` | Shared types and traits |
| Bridge | ✅ Implemented | `bridge/` | Cross-crate integration |

---

## 1. Overview

The `aletheon-self` crate implements the agent's self-awareness and self-protection capabilities. It contains 8 conceptual internal layers organized around the agent's sense of self:

```
┌─────────────────────────────────────────────────────────┐
│                    SelfField                             │
│                                                         │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐             │
│  │ Identity │  │ Boundary │  │ Care     │             │
│  │          │  │          │  │          │             │
│  │ Who am I │  │ What can │  │ What     │             │
│  │ What am  │  │ I touch  │  │ matters  │             │
│  │ I        │  │ What     │  │ to me    │             │
│  │          │  │ can't I  │  │          │             │
│  └──────────┘  └──────────┘  └──────────┘             │
│                                                         │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐             │
│  │ Narrative│  │ Conflict │  │ Attention│             │
│  │          │  │          │  │          │             │
│  │ My story │  │ Internal │  │ What I   │             │
│  │ memory   │  │ tensions │  │ focus on │             │
│  └──────────┘  └──────────┘  └──────────┘             │
│                                                         │
│  ┌──────────┐  ┌──────────┐                            │
│  │Continuity│  │ Mutation │                            │
│  │          │  │          │                            │
│  │ Persistent│  │ How I    │                            │
│  │ identity │  │ change   │                            │
│  └──────────┘  └──────────┘                            │
└─────────────────────────────────────────────────────────┘
```

## 2. The 8 Internal Layers

### 2.1 Identity Layer

Defines "who the agent is" — its core identity, capabilities, and constraints. Implemented through configuration and the hook system's trust model.

### 2.2 Boundary Layer

Defines what the agent can and cannot touch. Implemented through:
- Security policy (see [body/security.md](../body/security.md))
- WritableRoot path isolation (see [writable-root.md](writable-root.md))
- Sandbox execution (see [body/sandbox.md](../body/sandbox.md))

### 2.3 Care Layer

Defines what matters to the agent — priorities, values, and protection of critical resources. Implemented through:
- ResourceGovernor (resource limits and throttling)
- EmergencyKillswitch (multi-trigger emergency stop)
- PolicyEngine (permission levels L0-L3)

### 2.4 Narrative Layer

The agent's story — memory of what happened, what was done, and what was learned. Implemented through:
- Hook system event logging
- Audit trail (AuditLogger)
- Experience memory (see [body/acix.md](../body/acix.md))

### 2.5 Conflict Layer

Internal tensions — when goals conflict, when safety conflicts with capability. Implemented through:
- LoopDetector (see [loop-detector.md](loop-detector.md))
- CircuitBreaker (consecutive block detection)
- RiskClassifier (risk-based threshold adjustment)

### 2.6 Attention Layer

What the agent focuses on — perception filtering, priority management, event routing. Implemented through:
- PerceptionManager (event routing)
- EventAggregator (deduplication, batching, priority boost)
- BackpressureController (flow control)

### 2.7 Continuity Layer

Persistent identity across sessions and restarts. Implemented through:
- Session persistence (checkpoint/recovery)
- IntegrityMonitor (file hash baseline tracking)
- DaemonGuardian (crash recovery)

### 2.8 Mutation Layer

How the agent changes over time — learning, adaptation, self-update. Currently limited to:
- Experience memory accumulation
- Skill loading and hot-reload
- SelfUpdateManager (planned)

## 3. The review() Pipeline

The SelfField's core operation is the `review()` pipeline — a decision chain that evaluates every significant action before execution:

```
Action Request
    │
    ▼
┌──────────────┐
│ Hook         │  PreToolUse hooks can block or modify
└──────┬───────┘
       ▼
┌──────────────┐
│ Policy       │  PolicyEngine checks permission level (L0-L3)
└──────┬───────┘
       ▼
┌──────────────┐
│ Boundary     │  PathAccessGuard + WritableRoot checks
└──────┬───────┘
       ▼
┌──────────────┐
│ Care         │  ResourceGovernor checks resource limits
└──────┬───────┘
       ▼
┌──────────────┐
│ Permissions  │  RiskClassifier + LoopDetector pre-check
└──────┬───────┘
       ▼
┌──────────────┐
│ Narrative    │  AuditLogger records the decision
└──────┬───────┘
       ▼
┌──────────────┐
│ Verdict      │  Allow / Warn / Block / Escalate / InterruptTurn
└──────────────┘
```

## 4. Crate Structure

```
crates/aletheon-self/
├── core/           — Shared types and traits
├── bridge/         — Cross-crate integration points
├── impl/
│   ├── hook/       — Hook system (21 event types, trust model, config layers)
│   ├── resilience/ — DaemonGuardian, WatchdogTimer, SafeMode, crash recovery
│   ├── security/   — InputSanitizer, ResourceGovernor, EmergencyKillswitch, IntegrityMonitor
│   ├── perception/ — PerceptionManager, EventAggregator, perception sources, FUSE
│   └── mod.rs
└── testing/        — Test utilities
```

## 5. Related Documents

| Document | Scope |
|----------|-------|
| [hook-system.md](hook-system.md) | Hook event types, trust model, config layers, execution |
| [loop-detector.md](loop-detector.md) | LoopDetector, RiskClassifier, CircuitBreaker, OutputGuardrail |
| [writable-root.md](writable-root.md) | WritableRoot, FileSystemSandboxPolicy, PathAccessGuard |
| [self-protection.md](self-protection.md) | InputSanitizer, ResourceGovernor, EmergencyKillswitch, IntegrityMonitor |
| [resilience.md](resilience.md) | Error handling, panic recovery, rate limiting, backpressure |
| [perception-sources.md](perception-sources.md) | System service management, perception source integration |

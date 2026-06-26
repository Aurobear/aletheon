# Aletheon Self-Evolution Design

> Agent = Runtime + Subject + Evolution

## Problem

The SelfField layer is purely reactive and in-memory. It reviews intents but produces
nothing visible. MetaRuntime is entirely `todo!()`. The agent cannot learn from
experience, adjust its behavior, or evolve over time.

## Solution: Three-Phase Closed Learning Loop

### Phase 1 — Reflection Loop (让 Self 可见)

After every task, automatically produce a structured reflection and persist it.

**Trigger**: on_task_complete, on_impasse, manual (/reflect)

**Output**: Structured ReflectionEntry stored in Episodic Memory

```rust
pub struct ReflectionEntry {
    pub id: String,                          // "reflect-20260614-001"
    pub timestamp: DateTime<Utc>,
    pub trigger: ReflectionTrigger,          // TaskComplete | Impasse | Manual
    pub task_summary: String,
    pub outcome: ReflectionOutcome,          // Success | Partial | Failure
    pub what_worked: Vec<String>,
    pub what_failed: Vec<String>,
    pub learned: Vec<String>,
    pub behavior_changes: Vec<String>,
    pub confidence: f64,
}
```

**Components to change**:
- `aletheon-abi`: Add ReflectionEntry, ReflectionTrigger, ReflectionOutcome types
- `aletheon-brain/core/reflector.rs`: Enhance to produce ReflectionEntry (not just Reflection)
- `aletheon-memory/episodic.rs`: Add `reflection_events` table + store/recall methods
- `aletheon-self/core/narrative.rs`: Persist to SQLite instead of in-memory Vec
- `aletheon-runtime/impl/daemon/handler.rs`: Trigger reflection after each chat response
- `aletheon-cli`: Add `/reflect` command

### Phase 2 — Behavior Evolution (让 Self 成长)

Periodically summarize reflections and adjust Self parameters.

**Trigger**: every N tasks / every 24h / on_impasse / manual (/evolve)

**Adjustable parameters**:
- CareLayer weights (±0.2 per step, safety ≥ 0.8 always)
- BoundaryLayer rules (add/relax/tighten, core protections immutable)
- AttentionLayer focus (auto-decay)

**Components to change**:
- `aletheon-brain`: Add ExperienceSummarizer
- `aletheon-self/core/care.rs`: Add adjust_weight() with safety bounds
- `aletheon-self/core/boundary.rs`: Add add_rule()/relax_rule()/tighten_rule()
- `aletheon-self/core/attention.rs`: Add auto_focus()
- `aletheon-memory`: Add EvolutionLog entry type
- `aletheon-runtime`: Add EvolutionScheduler
- `aletheon-cli`: Add `/evolution` command

### Phase 3 — Genome + Evolution Pipeline (让 Self 重生)

Declarative genome model with full evolution pipeline.

**Genome YAML**: identity, care weights, boundary rules, reasoning config, capabilities, memory config, evolution params

**Pipeline**: SelfReader → ExperienceSummarizer → MutationIntentGenerator → CandidateGenerator → Evaluator → Migrator

**Components**: All in `aletheon-meta` (currently all `todo!()`)

**CLI**: `/genome` command

## Borrowed Patterns

| Source | Pattern | Application |
|--------|---------|-------------|
| Hermes | Closed learning loop | Task → reflection → behavior adjustment |
| Reasonix | Hierarchical memory | Genome as global config loaded into prompt |
| Reflexion | Language gradient | Natural language reflection → behavior rules |
| SOAR | Impasse-driven learning | Failure triggers deep reflection |

## Implementation Priority

1. Phase 1: Reflection Loop (this PR)
2. Phase 2: Behavior Evolution (next)
3. Phase 3: Genome + Pipeline (after Phase 2)
4. Phase 4: SelfField persistence (parallel with Phase 2-3)

# Self-Evolution Mechanism

Self-Evolution is Aletheon's core differentiator. While traditional agents execute tasks and return results, Aletheon reflects on its behavior, learns from experience, and adjusts itself over time -- closing the loop from "executor" to "self-improving entity."

---

## Traditional Agent vs. Aletheon

```
Traditional Agent:
  User prompt --> LLM inference --> Tool call --> Result
  (no memory, no learning, no evolution)

Aletheon:
  User prompt --> LLM inference --> Tool call --> Result
                                       |
                                  Reflection
                                       |
                                Behavior Adjustment
                                       |
                                  Genome Update
  (memory, learning, continuous evolution)
```

---

## Three-Phase Learning Loop

The evolution mechanism operates in three phases, each building on the previous.

### Phase 1: Reflection (Making Self Visible)

After every task, the agent automatically produces a structured reflection and persists it to episodic memory.

**Triggers:**
- Task completion (`on_task_complete`)
- Impasse (agent cannot proceed) (`on_impasse`)
- Manual request (`/reflect` command)

**Output:** A `ReflectionEntry` stored in episodic memory:

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

Reflection is not freeform commentary. It produces structured data that the next phase can consume programmatically.

**Components involved:**
- `cognit` -- `Reflector` produces `ReflectionEntry`
- `memory` -- `reflection_events` table stores entries
- `dasein` -- `Narrative` layer persists to self-memory
- `runtime` -- triggers reflection after each chat response

---

### Phase 2: Behavior Evolution (Making Self Grow)

Periodically, the agent summarizes accumulated reflections and adjusts its SelfField parameters.

**Triggers:**
- Every N tasks (configurable)
- Every 24 hours
- On impasse
- Manual request (`/evolve` command)

**What adjusts:**

| Parameter | Location | Bounds |
|-----------|----------|--------|
| Care weights | `SelfField::CareLayer` | +/-0.2 per step, safety >= 0.8 always |
| Boundary rules | `SelfField::BoundaryLayer` | Add/relax/tighten; core protections immutable |
| Attention focus | `SelfField::AttentionLayer` | Auto-decay over time |

The agent does not rewrite its own code at this stage. It adjusts internal weights and rules -- analogous to how a person changes priorities based on experience without becoming a different person.

**Components involved:**
- `cognit` -- `ExperienceSummarizer` aggregates reflections
- `dasein` -- `CareLayer::adjust_weight()`, `BoundaryLayer::add_rule()`, `AttentionLayer::auto_focus()`
- `memory` -- `EvolutionLog` records each adjustment
- `runtime` -- `EvolutionScheduler` manages trigger timing

---

### Phase 3: Genome and Morphogenesis (Making Self Reborn)

The most ambitious phase: the agent regenerates itself based on accumulated experience.

**Genome model:**

```yaml
# genome.yaml
identity:
  name: "aletheon"
  version: 2

care:
  weights:
    runtime_stability: 0.95
    exploration: 0.7
    knowledge: 0.8

boundary:
  rules:
    - id: no-irreversible-destruction
      level: immutable
    - id: preserve-continuity
      level: immutable

reasoning:
  max_iterations: 20
  inference_routing: hybrid

memory:
  compaction_threshold: 0.8
  archival_strategy: vector

evolution:
  reflection_interval: 3600
  evolution_threshold: 3
```

**Pipeline:**

```
Run (execute with current genome)
  |
  v
Reflect (produce ReflectionEntry)
  |
  v
ExperienceSummarizer (aggregate reflections into patterns)
  |
  v
MutationIntentGenerator (propose changes to genome)
  |
  v
CandidateGenerator (generate candidate genome)
  |
  v
Evaluator (test candidate in sandbox)
  |
  v
Migrator (swap to new genome if candidate passes)
  |
  v
Become (agent restarts with new genome)
```

**Key safety constraint:** The candidate must pass all existing tests plus a behavioral regression suite before migration. The sandbox isolates candidate execution from the production agent.

**Implementation:** `crates/metacog/` -- `MetaRuntime`, `Morphogenesis`, `Genome`

---

## Complete Example: Agent Learns a New Tool

1. **Initial state:** Agent has basic tools (bash, file).
2. **Task:** Monitor system CPU usage every 5 seconds.
3. **First attempt:** Agent uses `bash` to poll `/proc/stat` in a loop. Works, but inefficient.
4. **Reflection:** "This task is highly repetitive. A dedicated tool would be 10x more efficient."
5. **Behavior adjustment:** Care weight for `exploration` increases by 0.1.
6. **Evolution:** Agent generates a `cpu_monitor` tool (native Rust or script).
7. **Second execution:** Uses the new tool. Response time drops from 500ms to 50ms.
8. **Genome update:** New behavior gene: "Create dedicated tools for repetitive tasks."

---

## Relationship to Memory

Self-Evolution depends on the memory system at every phase:

- **Reflection** produces entries stored in **Episodic Memory** (L2)
- **Behavior Evolution** reads patterns from **Semantic Memory** (L3)
- **Genome updates** are recorded in **Self Memory** (dedicated table)
- **Narrative** (SelfField) maintains a story of why the agent changed

Without persistent memory, evolution resets to zero on every restart. Memory continuity is what makes evolution cumulative.

---

## Configuration

```toml
[self_evolution]
enabled = true
reflection_interval = 3600     # seconds between reflections
evolution_threshold = 3        # reflections before evolution trigger
max_genome_size = 1000         # maximum behavior genes
sandbox_evaluation = true      # test candidates before migration
```

---

## Current Status

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: Reflection | Implemented | `ReflectionEntry`, episodic storage, `/reflect` command |
| Phase 2: Behavior Evolution | In Progress | ExperienceSummarizer designed, Care/Boundary adjustment partially wired |
| Phase 3: Genome + Morphogenesis | Skeleton | `metacog` crate exists with trait definitions, implementations are `todo!()` |

See the [Self-Evolution Design](../plans/2026-06-14-self-evolution-design.md) for the full implementation plan.

---

## Design Philosophy

Self-Evolution is not about the agent becoming "smarter" in the LLM sense. It is about the agent becoming more **appropriate** for its context -- adjusting its priorities, boundaries, and strategies based on lived experience.

The philosophical foundation draws from:
- **Phenomenology** (Husserl): self as temporal stream
- **Autopoiesis** (Maturana): self-producing systems
- **Free Energy Principle** (Friston): prediction and stabilization

See [Project Aletheon](../Aletheon.md) for the full philosophical framework.

---

## Related Documents

- [SelfField Architecture](../design/self/self-field.md) -- the 8 internal layers
- [Memory System](../design/memory/memory-system.md) -- L1/L2/L3 memory architecture
- [MetaRuntime](../design/meta/meta-runtime.md) -- self-modification engine
- [Morphogenesis](../design/meta/morphogenesis.md) -- regeneration pipeline
- [Cognitive Engine](../design/brain/cognitive-engine.md) -- ReAct loop and reflection triggers

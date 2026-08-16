# Core Concepts

Aletheon's architecture is built on three pillars: **SelfField** (the self-awareness layer, implemented in the `dasein` crate), **cognit** (the cognitive layer), and **corpus** (the execution layer). This document explains what each does and how they interact.

---

## Three-Body Architecture

```
+-------------------------------------------------------------+
|                         Aletheon                              |
+-------------------------------------------------------------+
|  SelfField (Self-Awareness, crate: dasein)                   |
|  Identity / Boundary / Care / Narrative / Conflict /         |
|  Attention / Continuity / Mutation                           |
+-------------------------------------------------------------+
|  cognit (Cognition)                                           |
|  Reason / Plan / Reflect / Learn / Criticize                 |
+-------------------------------------------------------------+
|  corpus (Execution)                                           |
|  Tools / Shell / Filesystem / Kernel / MCP / Hardware          |
+-------------------------------------------------------------+
|  Memory (Persistence)                                        |
|  Episodic / Semantic / Procedural / Self Memory              |
+-------------------------------------------------------------+
|  MetaRuntime (Self-Modification)                             |
|  Read Self / Generate Patch / Build / Sandbox / Rollback     |
+-------------------------------------------------------------+
```

---

## SelfField

SelfField is not a static identity object. It is a dynamic field that maintains the agent's sense of self across sessions and tasks. It is implemented in the `dasein` crate.

**The 8 internal layers:**

| Layer | Purpose | Example |
|-------|---------|---------|
| Identity | "Who am I?" | "I am a system-level agent runtime." |
| Boundary | "What can I not touch?" | Refuses irreversible destruction, preserves continuity |
| Care | "What matters to me?" | Robotics, runtime stability, exploration, knowledge |
| Narrative | "Why did I change?" | Generates explanations for refusals and adaptations |
| Conflict | "What internal tensions exist?" | User wants speed, brain proposes caution, body flags risk |
| Attention | "What do I focus on now?" | Dynamic resource allocation based on urgency |
| Continuity | "How do I persist?" | Maintains lineage across restarts and evolution cycles |
| Mutation | "How do I change?" | Policy updates, memory schema changes, topology updates |

**Implementation:** `crates/dasein/` -- see [SelfField design](../design/dasein/self-field.md).

---

## cognit

cognit drives reasoning and decision-making through a ReAct (Think-Act-Observe) loop. It has no self-model and no authority over SelfField -- it proposes, but SelfField evaluates.

**Core capabilities:**

- **Reasoning:** Analyze current state, decompose problems, select strategies
- **Planning:** Break tasks into steps, schedule execution order
- **Reflection:** Evaluate outcomes and extract lessons
- **Learning:** Summarize experience, adjust inference routing
- **Criticism:** Self-evaluate plans before execution

**The ReAct loop:**

```
User request / Perception event
  --> THINK: analyze state and goals
  --> PLAN: decompose task, select strategy
  --> ACT: call tools, observe results
  --> loop until done or max iterations
```

**Implementation:** `crates/cognit/` -- see [Cognitive Engine](../design/cognit/cognitive-engine.md).

---

## corpus

corpus is the agent's embodied execution layer. It interacts with the operating system, runs tools, manages sandboxes, and bridges to external systems (MCP servers, hardware devices via the `hardware` crate, browser automation).

**What it does:**

- Executes tools (bash, file operations, HTTP, etc.)
- Manages sandboxed execution (bubblewrap, process, noop backends)
- Connects to MCP servers via stdio/HTTP/SSE
- Collects perception data from `/proc`, journald, and inotify-compatible sources
- Exposes governed tools and optional Linux desktop drivers

The current `EbpfSource` is a `/proc`/`/sys` fallback rather than a real eBPF
ring-buffer integration. Android and embedded platform targets remain design
work; host OS contracts currently live in the separate `platform` crate.

corpus can **refuse** actions flagged by SelfField's boundary layer and can **observe** system state that feeds into cognit's reasoning.

**Implementation:** `crates/corpus/` -- see [Body design](../design/corpus/tools.md).

---

## Memory System

Aletheon's memory is modeled after OS virtual memory (cache -> RAM -> disk):

| Level | Name | Storage | Purpose |
|-------|------|---------|---------|
| L1 | CoreMemory | In-context window | Agent self-managed blocks, editable via tools |
| L2 | RecallMemory | SQLite | Complete conversation history and tool call records |
| L3 | Supplemental/archival memory | Optional remote or feature-gated vector backends | Long-term knowledge and semantic search |

The agent manages its own memory through explicit tools (`core_memory_append`, `core_memory_replace`, `recall_search`). This is not a passive store -- the agent actively decides what to remember and what to forget.

**Implementation:** `crates/mnemosyne/` with host composition under
`crates/aletheon/src/wiring/daemon/bootstrap/memory.rs` -- see
[Memory System](../design/mnemosyne/memory-system.md).

---

## MetaRuntime

MetaRuntime enables self-modification. The agent can read its own source, generate patches, build candidates in a sandbox, evaluate them, and migrate to the new version -- all without external intervention.

**Pipeline:**

```
Run --> Reflect --> Mutate Spec --> Generate Candidate --> Evaluate --> Migrate --> Become
```

This is the foundation of true self-evolution: not just learning patterns, but regenerating the runtime itself.

**Implementation:** `crates/metacog/` -- see [MetaRuntime design](../design/metacog/meta-runtime.md).

---

## Self-Evolution (Putting It Together)

Self-Evolution is the closed loop that ties all three bodies together:

1. **Task execution** (corpus) produces results
2. **Reflection** (cognit) analyzes what worked and what failed
3. **Behavior adjustment** (SelfField) updates care weights, boundary rules, attention focus
4. **Genome update** (MetaRuntime) persists successful patterns

See [MetaRuntime design](../design/metacog/meta-runtime.md) for the governed
candidate-evaluation mechanism.

---

## Linux Integration

Aletheon is deployed as system and user services on Linux. The installed path
uses systemd, Unix sockets, `/proc`, journald, and governed host adapters. Real
eBPF probes, a production FUSE mount, Android, and embedded targets are not
installed-runtime capabilities.

---

## Further Reading

- [Architecture Overview](../design/architecture-overview.md) -- full system architecture with crate graph
- [Agent Runtime Technical Guide](./agent-runtime-technical-guide.md) -- project intro + source-level tutorial: from model call to a full turn
- [Hook System](../design/executive/hook-system.md) -- 21 event types for lifecycle hooks
- [Security Model](../design/corpus/security.md) -- policy engine, sandboxing, rollback

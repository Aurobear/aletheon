# Core Concepts

Aletheon's architecture is built on three pillars: **SelfField** (the self-awareness layer), **BrainCore** (the cognitive layer), and **BodyRuntime** (the execution layer). This document explains what each does and how they interact.

---

## Three-Body Architecture

```
+-------------------------------------------------------------+
|                         Aletheon                              |
+-------------------------------------------------------------+
|  SelfField (Self-Awareness)                                  |
|  Identity / Boundary / Care / Narrative / Conflict /         |
|  Attention / Continuity / Mutation                           |
+-------------------------------------------------------------+
|  BrainCore (Cognition)                                       |
|  Reason / Plan / Reflect / Learn / Criticize                 |
+-------------------------------------------------------------+
|  BodyRuntime (Execution)                                     |
|  Tools / Shell / Filesystem / Kernel / MCP / ROS             |
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

SelfField is not a static identity object. It is a dynamic field that maintains the agent's sense of self across sessions and tasks.

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

**Implementation:** `crates/aletheon-self/` -- see [SelfField design](../design/self/self-field.md).

---

## BrainCore

BrainCore drives reasoning and decision-making through a ReAct (Think-Act-Observe) loop. It has no self-model and no authority over SelfField -- it proposes, but SelfField evaluates.

**Core capabilities:**

- **Reasoning:** Analyze current state, decompose problems, select strategies
- **Planning:** Break tasks into steps, schedule execution order
- **Reflection:** Evaluate outcomes, extract lessons (see [Self-Evolution](../architecture/self-evolution.md))
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

**Implementation:** `crates/aletheon-brain/` -- see [Cognitive Engine](../design/brain/cognitive-engine.md).

---

## BodyRuntime

BodyRuntime is the agent's embodied execution layer. It interacts with the operating system, runs tools, manages sandboxes, and bridges to external systems (MCP servers, ROS nodes, browser automation).

**What it does:**

- Executes tools (bash, file operations, HTTP, etc.)
- Manages sandboxed execution (bubblewrap, process, noop backends)
- Connects to MCP servers via stdio/HTTP/SSE
- Collects perception data from /proc, journald, inotify, eBPF
- Provides platform abstraction (Linux systemd/D-Bus, Android)

BodyRuntime can **refuse** actions flagged by SelfField's boundary layer and can **observe** system state that feeds into BrainCore's reasoning.

**Implementation:** `crates/aletheon-body/` -- see [Body design](../design/body/tools.md).

---

## Memory System

Aletheon's memory is modeled after OS virtual memory (cache -> RAM -> disk):

| Level | Name | Storage | Purpose |
|-------|------|---------|---------|
| L1 | CoreMemory | In-context window | Agent self-managed blocks, editable via tools |
| L2 | RecallMemory | SQLite | Complete conversation history and tool call records |
| L3 | ArchivalMemory | Vector DB (Qdrant/LanceDB) | Long-term knowledge, semantic search |

The agent manages its own memory through explicit tools (`core_memory_append`, `core_memory_replace`, `recall_search`). This is not a passive store -- the agent actively decides what to remember and what to forget.

**Implementation:** `crates/aletheon-memory/` + `crates/aletheon-runtime/src/impl/memory/` -- see [Memory System](../design/memory/memory-system.md).

---

## MetaRuntime

MetaRuntime enables self-modification. The agent can read its own source, generate patches, build candidates in a sandbox, evaluate them, and migrate to the new version -- all without external intervention.

**Pipeline:**

```
Run --> Reflect --> Mutate Spec --> Generate Candidate --> Evaluate --> Migrate --> Become
```

This is the foundation of true self-evolution: not just learning patterns, but regenerating the runtime itself.

**Implementation:** `crates/aletheon-meta/` -- see [MetaRuntime design](../design/meta/meta-runtime.md).

---

## Self-Evolution (Putting It Together)

Self-Evolution is the closed loop that ties all three bodies together:

1. **Task execution** (BodyRuntime) produces results
2. **Reflection** (BrainCore) analyzes what worked and what failed
3. **Behavior adjustment** (SelfField) updates care weights, boundary rules, attention focus
4. **Genome update** (MetaRuntime) persists successful patterns

See [Self-Evolution](../architecture/self-evolution.md) for the full mechanism.

---

## Linux Integration

Aletheon is a system-level agent, not an application-level one. It integrates with the Linux kernel and system services through eBPF, systemd, FUSE, and D-Bus.

See [Linux Integration](../architecture/linux-integration.md) for details.

---

## Further Reading

- [Architecture Overview](../design/architecture-overview.md) -- full system architecture with crate graph
- [Project Aletheon](../Aletheon.md) -- design philosophy and research foundations
- [Hook System](../design/self/hook-system.md) -- 21 event types for lifecycle hooks
- [Security Model](../design/body/security.md) -- policy engine, sandboxing, rollback

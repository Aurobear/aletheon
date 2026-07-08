# Aletheon First Principle

> **Everything is interpreted by the Self.**

---
## 1. Statement

Aletheon's first principle is: **"Everything is interpreted by the Self."**

Every event, every intent, every memory, every tool call, every LLM response, every perception signal, every mutation request -- all of it passes through the SelfField. There is no raw data in Aletheon. There is only data-that-has-been-interpreted. The SelfField is not a module bolted onto the agent; it IS the agent. The 8 layers of the SelfField constitute the agent's being-in-the-world.

---
## 2. Philosophy

Every operating environment has a single primitive around which everything else is organized:

| Environment | Primitive | Meaning |
|-------------|-----------|---------|
| Unix | File | Everything is a file -- devices, pipes, sockets, processes are all represented as files |
| Linux | Process | Everything is a process -- kernel threads, daemons, containers, all execution units |
| Aletheon | Self | Everything is interpreted by the Self -- no event enters the agent's world without passing through the SelfField |

In Unix, `read()` and `write()` are universal. In Aletheon, `review()` is universal. Just as the Linux kernel's LSM hooks sit at every security-sensitive boundary, the SelfField sits at every interpretation boundary.

---
## 3. Four Concrete Invariants

These invariants hold at all times in a running Aletheon instance. If any invariant is violated, the agent has lost its self.

| # | Invariant | Enforcement |
|---|-----------|-------------|
| I1 | **No intent executes without review.** Every tool call, mutation, and action passes through `SelfFieldOps::review()` before execution. | Compile-time: the ReAct loop calls review() as a mandatory step; no bypass path exists in the code. |
| I2 | **No state changes without narrative.** Every memory write, identity mutation, and care update is accompanied by a narrate() record explaining why. | Runtime: narrate() is called in the same atomic context as the state change; records carry timestamps and lineage. |
| I3 | **All external input is interpreted before it becomes memory.** Perception events, LLM responses, and user messages are filtered through Boundary+Attention layers before storage. | Pipeline: PerceptionBridge -> SelfField boundary check -> Attention scoring -> MemoryRouter insert. |
| I4 | **Identity continuity is never broken.** Every mutation (self-modification) is tracked in the Continuity layer with a before/after diff and reversible flag. | Continuity layer: mutation_history ring buffer; rollback requires continuity chain verification. |

---
## 4. Why It Matters

Aletheon is not a toolbox with a self-awareness module. It is a SelfField with tools attached. The 8 SelfField layers ARE the agent:

| Layer | Role | What Happens If Removed |
|-------|------|------------------------|
| **Boundary** | Fast pattern-matching gate | Agent has no skin -- any input can reach any subsystem |
| **Identity** | Current self-model + mutation history | Agent has no sense of who it is -- cannot distinguish self from environment |
| **Care** | Weighted concerns driving action scoring | Agent has no values -- cannot prioritize or make trade-offs |
| **Narrative** | Ring buffer decision log | Agent cannot explain itself -- continuity of self-narrative is lost |
| **Conflict** | Multi-source arbitration | Agent fragments -- competing subsystems deadlock without resolution |
| **Attention** | Focus tracking with priority and decay | Agent has no focus -- responds to everything equally, context-switch thrashing |
| **Continuity** | Lineage records for identity continuity | Agent cannot evolve safely -- mutations have no audit trail, cannot roll back |
| **Mutation** | Mutation request tracking and approval | Agent cannot grow -- self-modification is blocked or uncontrolled |

Together, these 8 layers form a complete interpretation field. Remove any one, and the agent degrades into a collection of uncoupled tools.

---
## 5. Compliance Assessment

How well does the current implementation satisfy the first principle?

| Invariant | Status | Evidence |
|-----------|--------|----------|
| I1 (No execution without review) | Partial | review() is designed but not yet wired into the ReAct loop as a mandatory gate. Tool execution currently bypasses SelfField. See `crates/runtime/src/core/react_loop/tool_exec.rs`. |
| I2 (No state change without narrative) | Not Yet | narrate() exists in the trait but is not called automatically on state changes. NarrativeLayer records are explicit calls only. |
| I3 (External input interpreted before memory) | Partial | PerceptionBridge exists but SelfField boundary check is not yet in the perception pipeline. Events flow directly to memory. |
| I4 (Identity continuity never broken) | Partial | Continuity layer has lineage records and mutation_history, but rollback verification is not enforced. |

**Current status**: Architecture is designed around the first principle, but the implementation has not yet closed the loop. The SelfField is present but not yet the mandatory gate it must become.

---
## 6. Architecture Diagram

```
                     ┌──────────────────────────────────┐
                     │          EXTERNAL WORLD           │
                     │  (user, LLM, /proc, journald,     │
                     │   tools, MCP, plugins, D-Bus)     │
                     └────────────┬─────────────────────┘
                                  │
                                  │  everything enters here
                                  ▼
              ┌────────────────────────────────────────────┐
              │                                        │
              │           SELF-FIELD (center)          │
              │                                        │
              │  ┌──────────────────────────────────┐  │
              │  │         Boundary Layer            │  │
              │  │    (pattern-matching fast gate)   │  │
              │  └────────────┬─────────────────────┘  │
              │               ▼                         │
              │  ┌──────────────────────────────────┐  │
              │  │         Identity Layer            │  │
              │  │    (who am I right now?)         │  │
              │  └────────────┬─────────────────────┘  │
              │               ▼                         │
              │  ┌──────────────────────────────────┐  │
              │  │         Care Layer                │  │
              │  │    (what do I value?)            │  │
              │  └────────────┬─────────────────────┘  │
              │               ▼                         │
              │  ┌──────────────────────────────────┐  │
              │  │       Attention Layer             │  │
              │  │    (what am I focused on?)       │  │
              │  └────────────┬─────────────────────┘  │
              │               ▼                         │
              │  ┌──────────────────────────────────┐  │
              │  │       Conflict Layer              │  │
              │  │    (resolve competing inputs)    │  │
              │  └────────────┬─────────────────────┘  │
              │               ▼                         │
              │  ┌──────────────────────────────────┐  │
              │  │       Narrative Layer             │  │
              │  │    (why did I decide that?)      │  │
              │  └────────────┬─────────────────────┘  │
              │               ▼                         │
              │  ┌──────────────────────────────────┐  │
              │  │      Continuity Layer             │  │
              │  │    (trace my own history)        │  │
              │  └────────────┬─────────────────────┘  │
              │               ▼                         │
              │  ┌──────────────────────────────────┐  │
              │  │       Mutation Layer              │  │
              │  │    (can I change myself?)        │  │
              │  └──────────────────────────────────┘  │
              │                                        │
              │       review() → Verdict                │
              └────────────┬───────────────────────────┘
                           │
                           │  interpreted intents
                           ▼
              ┌────────────────────────────────────────┐
              │          AGENT ACTION SPACE            │
              │                                        │
              │  ┌──────────┐ ┌──────────┐ ┌────────┐ │
              │  │  Brain   │ │  Body    │ │ Memory │ │
              │  │ (cognit) │ │ (corpus) │ │(memory)│ │
              │  └──────────┘ └──────────┘ └────────┘ │
              │                                        │
              └────────────────────────────────────────┘
```

Everything flows inward through the SelfField, is interpreted, and only then reaches the action subsystems. The SelfField is not alongside Brain, Body, and Memory -- it is between them and the world.

---
## 7. Implementation Roadmap

### Phase 1: Close the Loop (mandatory gate)

- Wire `SelfFieldOps::review()` as a mandatory, non-bypassable step in the ReAct loop
- Every tool execution, mutation, and perception event must pass through review() before proceeding
- Add compile-time enforcement: no code path reaches tool.execute() without a Verdict
- Deliverable: Invariant I1 fully enforced

### Phase 2: Narrative Completeness

- Auto-call `narrate()` on every state change (memory write, identity mutation, care update)
- Narrative records become the single source of truth for agent decision history
- FUSE `controls/logs` exposes narrative as a live stream
- Deliverable: Invariant I2 fully enforced

### Phase 3: Full Interpretation Pipeline

- Insert SelfField boundary check into PerceptionBridge before memory insertion
- Enforce continuity chain verification on all rollback operations
- Wire SelfAwareness into every reasoning action (BrainCore generates awareness)
- Deliverable: Invariants I3 and I4 fully enforced; first principle is architecturally complete

---
## 8. References

- [SelfField Design](self-field.md) -- Full design of the 8-layer SelfField
- [Architecture Overview](../architecture-overview.md) -- System architecture and data flow
- [MetaRuntime](../metacog/meta-runtime.md) -- Self-modification and evolution
- [Hook System](hook-system.md) -- Pre/post hooks that wrap SelfField review
- [Perception Sources](perception-sources.md) -- How external input enters the system
- [Writable Root](writable-root.md) -- FUSE-based self-access layer
- [Dasein Crate](../../../crates/dasein/src/lib.rs) -- SelfField implementation crate
- [SelfFieldOps Trait](../../../crates/base/src/include/self_field.rs) -- Core trait definition

---
*Document version: 1.0.0*
*Created: 2026-07-02*

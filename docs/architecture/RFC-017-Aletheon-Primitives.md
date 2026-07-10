# RFC-017 Aletheon Primitives

> **Status:** Canonical vocabulary. This is the reference every other subsystem is
> built against. Linux has Process / Thread / File / Socket / Signal; ROS has
> Node / Topic / Service / Action; Aletheon has the primitives below. Once these are
> stable, code, communication protocols, and the plugin system evolve *around* them
> rather than redefining them.
>
> **Rule:** every subsystem communicates using these primitives instead of each
> other's concrete implementations. If you need a new cross-subsystem type, add it
> here first — do not leak an internal struct across a boundary.
>
> **Source of truth in code:** `crates/fabric/src/primitives/` (cognitive objects +
> communication) and `crates/fabric/src/ops.rs` (subsystem ops traits). This document
> and that code must agree; if they drift, the code wins and this document is fixed.

---

## 1. Subsystem primitives

The seven cognitive subsystems plus the fabric they communicate over. Each owns its
state; none may mutate another's — interaction is only through the subsystem's ops
trait (`crates/fabric/src/ops.rs`).

| Primitive | Role | Owns | Ops trait |
|-----------|------|------|-----------|
| **Executive** | Orchestrates; never reasons. | Lifecycle, Scheduling, Supervision, Resource, Authority | — (drives the others) |
| **Cognit** | The cognitive core — how to think. | Planner, Reasoner, Verifier, Reflector, Learner, Harness | `CognitOps` |
| **Dasein** | The self — identity & boundaries. | Identity, Values, Boundary, Narrative, Authority-to-refuse | `DaseinOps` |
| **Agora** | Shared cognitive workspace (working memory). | Blackboard, Attention, Task graph, Scratchpad, Trace | `AgoraOps` |
| **Mnemosyne** | Experience across time. | Episodic/Semantic/Procedural/Self memory, Index, Association, Replay | `MnemosyneOps` |
| **Corpus** | The body — acting on the world. | Tools, Skills, Hooks, Sandbox, Drivers | `CorpusOps` |
| **Metacog** | Self-modification. | Evaluation, Morphogenesis, Migration, Rollback | (meta-runtime) |
| **fabric** | The ABI + communication substrate. | Primitives, ops traits, Envelope, EventBus | (the layer itself) |

**Invariants**
- The Executive contains none of Lifecycle/Scheduling/etc.'s *cognitive* logic — it
  delegates. If a thing reasons, plans, remembers, or decides, it is **not** in the
  Executive. (See [RFC-001 §5](RFC-001-Philosophy.md).)
- Agora is **session-scoped and in-memory**; it never persists by itself. Mnemosyne is
  the only subsystem that persists cognitive state across restarts.
- A harness (in Cognit) is pluggable: `CognitiveHarness` in `ops.rs`. ReAct is one
  harness, not the fixed architecture.

---

## 2. Cognitive objects

The data that flows *through* reasoning. Defined/re-exported at
`crates/fabric/src/primitives/cognitive.rs`.

| Object | Meaning | Shape (canonical fields) | Home |
|--------|---------|--------------------------|------|
| **Intent** | What the system means to do. | (existing) | `include/self_field.rs` |
| **Observation** | Something perceived from the world. | (existing) | `include/brain.rs` |
| **Hypothesis** | A tentative explanation awaiting verification. | `id, statement, confidence: f64, evidence_ids: Vec<String>` | `primitives/cognitive.rs` |
| **Evidence** | A datum bearing on a hypothesis/decision. | `id, source, content, weight: f64` | `primitives/cognitive.rs` |
| **Plan** | An ordered set of steps toward a goal. | (existing) | `include/brain.rs` |
| **Decision** | A committed choice among options. | (existing) | `policy/execpolicy.rs` |
| **Experience** | A completed episode: intent → outcome. | (existing) | `include/brain.rs` |
| **Narrative** | A running self-narrative summary. | `id, summary, entries: Vec<String>` | `primitives/cognitive.rs` |
| **Commitment** | A promise the system intends to honor. | `id, statement, created_at, status: {Open, Fulfilled, Abandoned}` | `primitives/cognitive.rs` |

**Invariants**
- Existing objects (Intent, Observation, Plan, Decision, Experience) are **re-exported
  from their homes, not redefined** — one definition, one source of truth.
- The four objects that had no home before (Hypothesis, Evidence, Narrative,
  Commitment) live in `cognitive.rs` as plain `serde` structs — pure data, no logic.
- Confidence and weight are `f64` in `[0.0, 1.0]`.

---

## 3. Communication primitives

How subsystems talk. Typed wrappers over the wire `Envelope`, at
`crates/fabric/src/primitives/comm.rs`. The wire format is `Envelope`
(`crates/fabric/src/ipc/envelope.rs`); the four verbs make *intent* explicit at the
type level and lower to an `Envelope` with the correct `Pattern`.

| Primitive | Semantics | Wire pattern | Fields |
|-----------|-----------|--------------|--------|
| **Envelope** | The universal message (like `sk_buff`). Everything lowers to this. | — | `id, source, target, pattern, priority, ttl, payload, timestamp` |
| **Command** | "Do this." No response awaited. | `FireAndForget` | `target, payload` |
| **Query** | "Tell me this." Response expected within a timeout. | `Request { timeout_ms }` | `target, payload, timeout` |
| **Event** | "This happened." Async broadcast to a topic. | `Publish` → `Target::Topic` | `topic, payload` |
| **Stream** | Continuous data flow keyed by session. | `Stream { session_id }` | `target, session_id, payload` |
| **Mailbox** | An endpoint you `send()` to and `recv()` from. Backed by the CommunicationBus. | — | trait: `send(Envelope)`, `recv() -> Option<Envelope>` |

**Why four verbs, not "everything is an Event"**
Collapsing all messaging into Event loses the wait-semantics that callers depend on:
- Command is fire-and-forget — the caller must *not* block.
- Query has a response and a timeout — the caller *must* wait, and can time out.
- Event is broadcast — zero or many receivers, no reply.
- Stream is long-lived and ordered — backpressure and session identity matter.

One `Envelope::Pattern` cannot be all four at once without the caller guessing. The
verbs encode the contract so it can't be guessed wrong.

**Invariants**
- Every verb lowers to exactly one `Envelope` via `into_envelope(source)`; the wire
  format never forks.
- `Event` is exposed as `fabric::primitives::Event` (not re-exported at the fabric
  crate root) to avoid colliding with the existing `events::event::Event` trait.

---

## 4. How the primitives compose (one turn)

```
Intent ─▶ Cognit builds Context ─▶ Mnemosyne.recall() ─▶ publish to Agora (blackboard)
      │                                                          │
      ▼                                                          ▼
   Planner ─▶ Reasoner ─▶ (Hypothesis + Evidence on the blackboard) ─▶ Verifier ─▶ Decision
                                                                                    │
                                                          Command/Query to Corpus ◀─┘
                                                                    │
                                              Tool results ─▶ Agora.trace ─▶ Reflector
                                                                    │
                                          Experience ─▶ Agora.snapshot() ─▶ Mnemosyne.store()
```

Every arrow crossing a subsystem boundary carries a **primitive from this document** —
never an internal type. That is the whole point: the vocabulary is the contract, and a
stable vocabulary lets the implementations behind it change freely.

---

## 5. Stability contract

Adding a primitive is a deliberate act, not an incidental one:

1. It must be needed by **more than one** subsystem (otherwise it is an internal type).
2. It is added to `crates/fabric/src/primitives/` (or `ops.rs` for a trait) **and** to
   this document, in the same change.
3. Removing or renaming a primitive is a breaking change to the whole system and gets
   its own RFC.

Related: [RFC-001 Philosophy](RFC-001-Philosophy.md) §3.5 (Primitives over
implementations), [RFC-012 Communication](RFC-012-Communication-Harness.md),
[RFC-014 Agora](RFC-014-Agora-Architecture.md).

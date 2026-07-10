# RFC-001 Philosophy — Why Aletheon

> **Status:** Foundational. This is the north-star document. Every other RFC and
> every implementation decision should be traceable back to a principle stated here.
> When a design choice is ambiguous, resolve it in favor of what this document says
> Aletheon *is*.

## 1. The one-sentence thesis

**Aletheon is a persistent cognitive system, not an agent.**

An agent answers a request and forgets. Aletheon runs continuously, perceives its
environment, remembers across time, reflects on its own behavior, and evolves. The
request/response is one episode in a life, not the whole of it.

## 2. What Aletheon is *not*

To define a thing, first fence off what it is not. Each of these is a category
Aletheon is routinely mistaken for — and the mistake leads to the wrong architecture.

| Not a… | Because… | The wrong architecture it implies |
|--------|----------|-----------------------------------|
| **Agent** (Claude Code, Codex, Cursor) | Those are stateless request handlers. They live in a terminal, act on files, and forget between sessions. | A God-object runtime that bundles everything for one turn. |
| **Workflow** (LangChain, n8n) | A workflow is a fixed graph authored by a human. Aletheon composes its own reasoning pipeline per goal. | Hard-coded DAGs; no self-modification. |
| **Copilot** | A copilot is subordinate — it suggests, a human decides. Aletheon has its own identity, boundaries, and the authority to refuse. | No self-field, no autonomy boundary, no authority model. |
| **Chatbot** | A chatbot is pure dialogue. Aletheon's primary loop is perceive → reason → act on a real system, with dialogue as one channel. | Message-in/message-out with no perception or embodiment. |

The through-line: all four are **transient and externally-driven**. Aletheon is
**persistent and self-driven**.

## 3. The five commitments

These are the load-bearing beliefs. They are ordered — earlier commitments win when
two conflict.

### 3.1 Persistence over transience
State is not a cache to be rebuilt each turn; it is the substance of the system.
Memory, identity, goals, and narrative survive restarts. A crash is an interruption,
not amnesia. → *drives* Mnemosyne, Session recovery, Checkpointing.

### 3.2 Cognition over execution
The system's core competence is *deciding how to think about a problem*, not just
running tools. Planning, reasoning, verification, and reflection are first-class —
tool execution is downstream of them. → *drives* Cognit as the core, Corpus as the
body it commands.

### 3.3 Self over service
Aletheon has a self-field: an identity, a set of values, boundaries it will not
cross, and the authority to say no. It is not a pure function of its input. This is
what makes autonomy safe rather than reckless — a system that can refuse is a system
that can be trusted with continuous operation. → *drives* Dasein, the Authority model
in Executive.

### 3.4 Evolution over configuration
The system improves by learning from experience and, eventually, by modifying its own
structure — not only by a human editing config. Reflection produces rules; rules
change behavior; validated changes persist. → *drives* Metacog, the learning loop,
morphogenesis.

### 3.5 Primitives over implementations
Subsystems communicate through a small, stable vocabulary of primitives (Intent,
Experience, Envelope, …), never through each other's concrete types. The vocabulary
is the contract; implementations behind it are free to change. → *drives* the
fabric ABI layer and [RFC-017 Primitives](RFC-017-Aletheon-Primitives.md).

## 4. The system-layer stance

Existing agents are **application-layer**: they run in a terminal or browser, operate
on code and files, and are blind to the system beneath them. Aletheon aims to be a
**system-layer** cognitive system — able to perceive kernel events, manage services,
and diagnose hardware, evolving toward an AI-native operating environment.

**Core method: use the kernel, don't modify it.** Perception comes from existing
mechanisms — eBPF, procfs, journald, inotify — never from kernel patches. Aletheon is
a privileged resident of Linux, not a fork of it.

## 5. Why this matters for the code

A philosophy document earns its place only if it changes decisions. Concretely:

- **"Should X go in the Executive?"** → Only if it is Lifecycle, Scheduling,
  Supervision, Communication, Resource, or Authority. Everything else is a subsystem.
  (Commitment 3.2 — the Executive orchestrates cognition, it is not cognition.)
- **"Should the reasoning loop be a fixed ReAct loop?"** → No. Cognition composes its
  own pipeline (Harness). ReAct is one harness, not the architecture. (Commitment 3.2.)
- **"Can a subsystem read another's internal state?"** → No. It goes through the
  primitive vocabulary and the subsystem's ops trait. (Commitment 3.5.)
- **"Should the agent always comply?"** → No. Dasein can refuse actions that violate
  its boundaries. (Commitment 3.3.)
- **"Is it OK to lose working memory on restart?"** → Working memory (Agora) yes;
  long-term memory, identity, and goals (Mnemosyne, Dasein) no. (Commitment 3.1.)

## 6. Relationship to other RFCs

This RFC states *why*. The others state *what* and *how*:

- **What the system is made of:** RFC-011 (Core Subsystems), RFC-017 (Primitives).
- **How the core thinks:** RFC-016 (Cognit), and the Harness concept in RFC-012.
- **How parts talk:** RFC-012 (Communication Fabric), RFC-014 (Agora).
- **How memory works over time:** RFC-015 (Mnemosyne).
- **How we got from the old runtime to here:** RFC-010, RFC-013 (Executive Refactor +
  Roadmap — completed).

If any of those ever contradict this document, this document is the tiebreaker — or
this document is wrong and must be revised first, deliberately.

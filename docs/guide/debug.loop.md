# Aletheon Self-Debug Loop Design

> Version: v0.1
>
> Status: Design Proposal
>
> Purpose:
> Define how Aletheon performs continuous self-debugging with the assistance of external LLMs (Claude, Codex, etc.) without requiring constant human supervision.

---

# Motivation

One of the biggest challenges of building a persistent Agent Runtime is validation.

Unlike a traditional CLI tool, Aletheon is expected to run continuously for hours, days, or even months.

Human developers cannot realistically monitor the runtime 24/7.

Therefore, debugging itself should become an autonomous workflow.

Instead of requiring humans to constantly inspect logs, Aletheon should automatically expose its runtime state, allowing external reasoning agents to inspect, diagnose, and propose fixes.

---

# Design Philosophy

The goal is **not** to make Aletheon automatically rewrite itself.

Instead:

> Aletheon should automatically expose problems.

External reasoning models should automatically analyze them.

Humans remain the final reviewer.

The workflow becomes:

```text
Runtime
    │
Detect Failure
    │
Generate Runtime Context
    │
Claude / Codex Analysis
    │
Patch Proposal
    │
Human Review
    │
Merge
```

This keeps evolution safe while dramatically reducing developer effort.

---

# Human-Governed Self-Improvement

Aletheon is responsible for:

* detecting failures
* recording runtime context
* exposing current state
* replaying execution history

Claude / Codex are responsible for:

* root cause analysis
* reasoning
* bug diagnosis
* patch generation

Humans remain responsible for:

* architectural decisions
* code review
* merge approval
* release

This creates a safe evolution pipeline.

---

# The Session Gateway

The existing TUI and CLI should not be considered the core communication mechanism.

Instead:

```text
Human
Claude
Codex
SubAgent
Automation
        │
        ▼
 Session Gateway
        │
        ▼
 Runtime Session
```

The Session Gateway becomes the universal communication interface.

Every frontend communicates through the same API.

The TUI becomes only one client.

---

# Session

A Session is not simply a chat history.

A Session represents the complete runtime state of an active Agent.

A Session contains:

```text
User conversation

Agent responses

Current objective

Current plan

Reasoning summary

Tool execution history

Runtime status

Memory recall

Pending tasks

Open errors

Current observations
```

It represents the current "mental state" of the Agent.

---

# Context Port

The Context Port allows external agents to inspect a running session.

Conceptually:

```text
Claude

↓

Context Port

↓

Current Agent Context
```

Unlike reading plain logs,

Claude can inspect:

* current goal
* execution progress
* active memories
* current reasoning summary
* pending actions
* runtime health

This allows much higher-quality debugging.

---

# Chat Port

External reasoning agents should also be able to ask questions.

Example:

```text
Claude:

Why did you execute Tool X?

↓

Aletheon

Because Goal Y required capability Z.
```

The conversation continues using the existing session context.

This is fundamentally different from creating a brand new conversation.

---

# Watch Port

The Runtime should continuously publish important events.

Example:

```text
panic

tool_failed

permission_denied

loop_detected

memory_failure

goal_changed

runtime_stalled

resource_exhausted
```

External agents subscribe to these events.

This avoids polling.

Instead:

```text
Runtime

↓

Event Stream

↓

Claude Watcher

↓

Analysis
```

---

# Debug Workflow

The complete debugging workflow becomes:

```text
Aletheon Running

↓

Runtime detects anomaly

↓

Watch Port publishes event

↓

Claude attaches Session

↓

Context Port retrieves runtime snapshot

↓

Claude asks follow-up questions

↓

Runtime answers

↓

Claude generates:

• Root Cause

• Reproduction Steps

• Patch Proposal

↓

Human Review

↓

Merge
```

This creates an autonomous debugging assistant.

---

# Runtime Snapshot

A Runtime Snapshot should include:

```text
Session ID

Current Goal

Current Plan

Current Mode

Recent Events

Recent Tool Calls

Memory Recall

Pending Tasks

Runtime Health

Resource Usage

Open Errors

Current Configuration
```

This snapshot is significantly richer than ordinary logs.

It represents the complete execution context.

---

# Recommended Public API

Session Management

```text
session.create()

session.list()

session.attach(session_id)

session.snapshot(session_id)

session.ask(session_id, actor, message)

session.append_event(session_id, event)

session.watch(session_id)
```

Future APIs

```text
session.fork()

session.replay()

session.diff()

session.export()

session.summarize()

session.rollback()
```

---

# Claude Integration

Claude should never simulate typing into the TUI.

Instead,

Claude directly connects to the Session Gateway.

Architecture:

```text
Claude

↓

Session Gateway

↓

Current Runtime Session
```

Claude becomes another intelligent client.

The TUI is simply one frontend.

---

# Responsibility Boundary

Claude MAY:

* inspect runtime
* inspect memory
* inspect context
* ask questions
* generate bug reports
* generate patch proposals

Claude MUST NOT automatically:

* modify architecture
* merge patches
* disable security
* bypass permissions
* rewrite kernel components

These operations always require explicit approval.

---

# Long-Term Vision

Eventually, every running Aletheon instance should have an associated Debug Agent.

Architecture:

```text
            Human
               │
               ▼
        Review / Approve
               ▲
               │
        Patch Proposal
               ▲
               │
      Claude Debug Agent
               ▲
               │
        Session Gateway
               ▲
               │
        Running Aletheon
```

The human no longer monitors logs continuously.

Instead,

Aletheon exposes its runtime state,

Claude continuously reasons about it,

and humans only intervene when architectural decisions are required.

---

# Core Principle

The purpose of this system is not autonomous evolution.

The purpose is autonomous diagnosis.

Aletheon should become capable of answering:

> "What happened?"

> "Why did it happen?"

> "How can it be fixed?"

before asking a human for help.

This creates a practical and safe path toward a continuously improving Agent Runtime.

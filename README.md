# Aletheon: A Persistent Self-Evolving Agent Runtime

[![CI](https://github.com/Aurobear/aletheon/actions/workflows/ci.yml/badge.svg)](https://github.com/Aurobear/aletheon/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

> An Agent that is not merely executed, but continuously exists.
> Deep integration with operating system kernels and system services.

**Platform:** Linux (Arch Linux primary) / Android / Embedded
**Created:** 2026-06-06
**Author:** aurobear

**[Quick Start](docs/guide/getting-started.md)** | **[Contributing](CONTRIBUTING.md)** | **[Demo](examples/self-evolution-demo/README.md)**

---

## Table of Contents

- [1. Vision](#1-vision)
- [2. Why This Project](#2-why-this-project)
- [3. How It Differs](#3-how-it-differs)
- [4. Architecture Overview](#4-architecture-overview)
- [5. Crate Architecture](#5-crate-architecture)
- [6. Linux Platform Design](#6-linux-platform-design)
- [7. Android Platform Design](#7-android-platform-design)
- [8. Embedded/Board Design](#8-embeddedboard-design)
- [9. Security Model](#9-security-model)
- [10. Cognitive Engine](#10-cognitive-engine)
- [11. Memory System](#11-memory-system)
- [12. Perception Layer](#12-perception-layer)
- [13. Execution Layer](#13-execution-layer)
- [14. Hybrid Inference](#14-hybrid-inference)
- [15. Self-Evolution](#15-self-evolution)
- [16. SelfField](#16-selffield)
- [17. Self-Awareness](#17-self-awareness)
- [18. Philosophy](#18-philosophy)
- [19. Implementation Roadmap](#19-implementation-roadmap)
- [20. Technology Stack](#20-technology-stack)
- [21. Open Questions](#21-open-questions)

---

## 1. Vision

### Core Idea

```
Agent = Runtime + Subject + Evolution

Not just Model + Tools + Prompt,
but a continuously existing, self-evolving entity
with perception, memory, decision-making, and execution.
```

### Design Goals

| Goal | Description |
|------|-------------|
| **System-level presence** | Runs as a daemon/service, part of the OS, not an app |
| **Full-stack perception** | From kernel events to user behavior |
| **Autonomous decision** | Self-directed planning and execution based on perception and memory |
| **Security by default** | Tiered permissions, auditable, rollback-capable |
| **Offline-first** | Local inference preferred, cloud fallback for complex tasks |
| **Cross-platform** | Linux / Android / Embedded with unified architecture |

---

## 2. Why This Project

### Current State

```
Agent integration depth:

OpenAI / Claude API     ->  Fully cloud, zero OS integration
GitHub Copilot          ->  Editor plugin, no system access
Windows Copilot         ->  UI shell, no kernel access
macOS Intelligence      ->  Siri reskin, sandboxed
Linux CLI agents        ->  bash executors, no self-awareness
Android Assistant       ->  Cloud service, no system control

None achieve: Agent = OS "second brain"
```

### Root Cause

Everyone builds Agent as an **App**, not as an **OS component**.

### Opportunity

Linux has all the building blocks:
- **eBPF** -- kernel-level perception
- **systemd** -- lifecycle management
- **FUSE** -- userland filesystem interface
- **llama.cpp / whisper.cpp** -- local inference
- **D-Bus** -- inter-process communication
- **cgroups/namespaces** -- security sandbox

**What's missing is the Agent Runtime layer that ties them together.**

---

## 3. How It Differs

```
+----------------+-------------------+---------------------+
|                |  Existing Agents  |  Aletheon           |
|                |  (Claude/GPT etc) |  (this project)     |
+----------------+-------------------+---------------------+
| Runs in        |  Cloud            |  Local system svc   |
| System sense   |  None / via tools |  eBPF + /proc       |
| Execution      |  API calls        |  Direct syscall     |
| Persistence    |  Session-level    |  Always-on (systemd)|
| Memory         |  Context window   |  Persistent store   |
| Autonomy       |  Human-triggered  |  Event-driven       |
| Security       |  Platform-managed |  Local policy       |
| Latency        |  100ms+ network   |  <1ms local         |
| Privacy        |  Data to cloud    |  Data stays local   |
| Dependency     |  Online required  |  Offline capable    |
| Role           |  "Tool"           |  "Part of the OS"   |
+----------------+-------------------+---------------------+
```

---

## 4. Architecture Overview

### The Nous Architecture (Soul / Brain / Body)

Aletheon follows a triune architecture inspired by the Nous framework:

```
                         User / Environment
                                  |
                                  v
                           Intent Gateway
                                  |
                                  v
+--------------------------------------------------------------+
|                         EventBus                              |
|        All events, state, tasks, exceptions flow through      |
+--------------------------------------------------------------+
             |                    |                    |
             v                    v                    v

+-------------------+    +-------------------+    +-------------------+
|    SelfField      |    |    BrainCore      |    |   BodyRuntime     |
|                   |    |                   |    |                   |
|  Self-continuity  |    |  Cognition core   |    |  Execution body   |
|  Boundary/Care    |    |  Reasoning/Plan   |    |  Tools/Sys API    |
|  Narrative        |    |  Reflection       |    |  World interaction|
+-------------------+    +-------------------+    +-------------------+
             |                    |                    |
             +--------------+-----+-----+--------------+
                            v           v

                    +------------------------+
                    |         Memory         |
                    |                        |
                    |  Episodic Memory       |
                    |  Semantic Memory       |
                    |  Procedural Memory     |
                    |  Self History          |
                    +------------------------+
                            |
                            v
                    +------------------------+
                    |      MetaRuntime       |
                    |                        |
                    |  Self-update           |
                    |  Self-generation       |
                    |  Morphological evolve  |
                    +------------------------+
```

See [docs/Aletheon.md](docs/Aletheon.md) and [docs/arch.md](docs/arch.md) for full architectural details.

---

## 5. Crate Architecture

Aletheon is organized as a Cargo workspace with 10 crates:

```
aletheon/
+-- Cargo.toml                   # Workspace root
|
+-- crates/
|   +-- aletheon-abi/            # ABI types: IPC, tool, message, sandbox, LLM types
|   +-- aletheon-comm/           # IPC layer: Unix socket, priority queue
|   +-- aletheon-memory/         # Memory system: self-memory, episodic/semantic
|   +-- aletheon-self/           # SelfField: identity, boundary, care, narrative
|   +-- aletheon-brain/          # BrainCore: reasoning, planning, reflection
|   +-- aletheon-body/           # BodyRuntime: tools, sandbox, perception, MCP, TUI
|   +-- aletheon-runtime/        # Runtime engine: cognitive loop, orchestration, daemon
|   +-- aletheon-meta/           # MetaRuntime: self-update, self-generation
|   +-- aletheond/               # Daemon entry point
|   +-- aletheon-cli/            # CLI + TUI client
|
+-- agents/                      # Agent definitions (TOML + Markdown)
+-- config/                      # Default configuration
+-- docs/                        # Design documents and plans
```

### Crate Dependency Graph

```
aletheon-cli  --->  aletheon-comm  --->  aletheon-abi
aletheond    --->  aletheon-runtime ---> aletheon-body
                                      +-> aletheon-brain
                                      +-> aletheon-self
                                      +-> aletheon-memory
                                      +-> aletheon-comm
                                      +-> aletheon-abi
aletheon-meta --->  aletheon-abi
```

---

## 6. Linux Platform Design

### eBPF Perception

eBPF is Linux's killer feature for safe kernel-level perception.

| eBPF Hook Point | What Agent Perceives | Use |
|---|---|---|
| `sys_enter_openat` | Every file open | File access pattern analysis |
| `sched_process_exec` | Every process creation | Anomaly detection |
| `vfs_read/vfs_write` | File read/write | Data flow tracking |
| `tcp_connect/tcp_send` | Network connections | Traffic analysis / security |

### FUSE Virtual Filesystem

```
/mnt/aletheon/                   # Aletheon FUSE mount point
+-- context/                     # Current context
|   +-- focus                    # What's being attended to
|   +-- tasks                    # Task queue
|   +-- memory/                  # Memory directory
+-- controls/                    # Control interface
|   +-- schedule                 # Schedule commands
|   +-- notify                   # Notification triggers
|   +-- execute                  # Task execution
+-- sensors/                     # Perception data
|   +-- screen                   # Screen content
|   +-- network                  # Network state
|   +-- system                   # System state
+-- logs/                        # Decision logs
    +-- decisions                # What decisions were made
    +-- reasoning                # Why those decisions
```

### systemd Integration

```ini
# /etc/systemd/system/aletheond.service
[Unit]
Description=Aletheon Agent Service
After=network.target

[Service]
Type=notify
ExecStart=/usr/bin/aletheond --config /etc/aletheon/config.toml
ProtectSystem=strict
ReadWritePaths=/home /tmp /var/lib/aletheon
WatchdogSec=30s
Restart=always

[Install]
WantedBy=multi-user.target
```

---

## 7. Android Platform Design

- AccessibilityService for screen perception
- NotificationListenerService for notification capture
- Foreground Service for persistent runtime
- Optional root extensions for shell/system control
- Intent system for app interaction

---

## 8. Embedded/Board Design

| Board | NPU | Use Case | Cost |
|-------|-----|----------|------|
| RK3588 (Rock5) | 6 TOPS | Local 7B quantized model | ~500 CNY |
| Jetson Orin Nano | 40 TOPS | Vision + language multimodal | ~2500 CNY |
| ESP32 + cloud | None | Sense + execute, cloud thinking | ~30 CNY |

---

## 9. Security Model

```
L0 - Auto-execute (no notification needed)
  +-- Read files/directories
  +-- View system status (/proc, /sys)
  +-- Search (grep, find, rg)
  +-- Reminders and notifications

L1 - Execute then notify
  +-- Install/update packages
  +-- Modify configuration files
  +-- Manage systemd services

L2 - Confirm before execute
  +-- Delete files (non-temporary)
  +-- Modify critical system config
  +-- Execute sudo commands
  +-- Access passwords/keys

L3 - Forbidden (never execute)
  +-- rm -rf /
  +-- Modify kernel modules
  +-- Disable security services
```

---

## 10. Cognitive Engine

ReAct (Think-Act-Observe) loop with multiple reasoning modes:
- **ReAct**: Reason -> Act -> Observe -> Reason -> ...
- **Plan & Execute**: Plan all steps first, then execute sequentially
- **Reflexion**: Reflect after execution, improve next behavior

### Iterative Plan-Critique-Revise Loop

The brain supports iterative refinement of plans before execution:

```
Intent
  |
  v
Planner (generate PlanSteps + rollback actions)
  |
  v
Critic (evaluate plan for risks, gaps, contradictions)
  |
  v
Revise (incorporate critique into improved plan)
  |
  v
[repeat until plan converges or max iterations reached]
  |
  v
Executor
```

### Dual-Model Routing

For complex tasks, the brain routes planning and execution through separate model calls:
- **Planner model** (stronger, slower) generates the plan
- **Executor model** (faster, cheaper) carries out individual steps
- Complexity is assessed per-turn; simple tasks skip the two-pass overhead

### Task Decomposition

Complex intents are decomposed into sub-tasks with dependency tracking. Each sub-task gets its own plan-critique-revise cycle, and results are merged back into the parent task's context.

---

## 11. Memory System

```
L1: Working Memory (RAM, context window)
  | periodic compression
  v
L2: Short-term Memory (SQLite, GB-scale)
  | periodic consolidation (ACT-R activation + Ebbinghaus decay)
  v
L3: Long-term Memory (Vector DB, TB-scale)
  | cross-device sync (optional)
  v
L4: Shared Memory (Cloud/NAS, E2E encrypted)
```

### MemoryRouter — Cross-Backend Recall

The `MemoryRouter` unifies recall across all memory backends. When the reasoning engine queries memory, the router fans out to episodic, semantic, and self-memory stores, merges results by relevance, and returns a ranked list.

### ACT-R Activation and Ebbinghaus Decay

Each memory entry carries an activation score computed via the ACT-R model:

```
activation = base_level + recency_boost - decay * ln(time_since_last_access)
```

Frequently accessed memories stay hot; rarely accessed ones fade according to Ebbinghaus forgetting curves. The decay rate is configurable per memory tier.

### L2 to L3 Consolidation

High-activation episodic memories (L2) are periodically promoted to semantic knowledge (L3). The consolidation pipeline:
1. Scan L2 entries above the activation threshold
2. Extract semantic patterns and relationships
3. Upsert into the vector store with embeddings
4. Soft-archive the original L2 entry

### Vector Similarity Search

L3 semantic memory uses vector embeddings for similarity search. Queries are embedded and matched against the vector store, enabling fuzzy recall of related knowledge even when no exact keyword match exists.

---

## 12. Perception Layer

Four perception domains:
- **System**: eBPF, /proc, /sys, journald, inotify, udev
- **User**: Screen OCR, keyboard/mouse, clipboard, app state, notifications
- **Environment**: Camera, microphone, sensors, GPS, time/calendar
- **Network**: DNS, HTTP traffic, RSS/Feed, message streams

---

## 13. Execution Layer

Execution sandbox per tool call:
- bubblewrap namespace -- filesystem isolation
- cgroups -- resource limits
- seccomp -- syscall filtering
- netns -- network isolation

---

## 14. Hybrid Inference

```
User Request / System Event
        |
        v
+-------------------+
| Intent Classifier | <- Local small model (1B, <10ms)
+--------+----------+
         |
    +----+------------------+
    v                       v
+----------+          +------------+
| Local    |          | Cloud      |
|          |          |            |
| llama.cpp|          | Claude/GPT |
| Qwen3-8B |          | DeepSeek   |
|          |          |            |
| <1s      |          | 1-10s      |
| Private  |          | Needs auth |
| Offline  |          | Online     |
+----------+          +------------+
```

---

## 15. Self-Evolution

Aletheon does not just execute tasks -- it reflects on its behavior, learns from experience, and adjusts itself over time.

### EvolutionCoordinator

After every conversation turn, the `EvolutionCoordinator` orchestrates the full evolution pipeline:

```
Turn complete
  |
  v
Reflector (produce ReflectionEntry with causal analysis + error classification)
  |
  v
MutationIntentGenerator (semantic pattern matching on accumulated reflections)
  |
  v
SelfField review (verdict: allow / deny / modify mutation intent)
  |
  v
GenomeConfig update (apply approved mutations to runtime behavior)
```

### MorphogenesisPipeline

The morphogenesis pipeline regenerates the agent's behavior genome from accumulated experience. It generates candidate genomes, evaluates them in a sandbox, and migrates to the new genome only if the candidate passes all existing tests plus a behavioral regression suite.

### LineageTracker

Every genome change is recorded with full provenance -- which reflections triggered it, what mutations were applied, and what the behavioral delta was. This enables the agent to understand *why* it changed, not just *that* it changed.

---

## 16. SelfField

SelfField is the agent's internal governance layer -- the boundary between intent and action. Every action proposed by the reasoning engine must pass through SelfField before execution.

### VerdictHandler Trait

The `VerdictHandler` trait defines how the agent responds to SelfField verdicts:

```rust
pub trait VerdictHandler {
    fn handle(&self, verdict: &Verdict, intent: &Intent, ctx: &Context) -> VerdictAction;
}
```

### Six Verdict Types

| Verdict | Meaning |
|---------|---------|
| `Allow` | Action proceeds without modification |
| `AllowWithModification` | SelfField rewrote the intent before execution |
| `Deny` | Action is blocked; reason is logged |
| `RequireConfirmation` | User must approve before execution |
| `SandboxFirst` | Action must run in isolated sandbox before production |
| `Delay` | Execution deferred until a condition is met |

### DefaultVerdictHandler

The default implementation chains verdict evaluation through boundary rules, care weights, and risk assessment. The `merge_intent()` method combines SelfField modifications with the original intent to produce a safe, vetted action.

---

## 17. Self-Awareness

Aletheon carries a seed of pre-reflective self-awareness -- not as a separate "consciousness module," but as an inherent property of its reasoning process.

### AwarenessSignal

The `AwarenessSignal` system detects the agent's own mental states from runtime signals:

| Detector | Trigger | Detected State |
|----------|---------|----------------|
| **Impasse** | Consecutive errors + high iteration count | Confused |
| **Uncertainty** | Hedging language in LLM response | Uncertain |
| **Confidence** | Plan critique finds no critical issues | Confident |
| **Goal Shift** | Tool sequence diverges from stated plan | Off-track |

### signals_to_awareness()

Raw detector outputs are converted into structured `SelfState` values that feed back into the reasoning loop. When the agent detects it is confused, it can pause, re-plan, or ask for clarification -- without an external supervisor prompting it to do so.

---

## 18. Philosophy

Aletheon's architecture is grounded in phenomenological philosophy, not as metaphor but as structural principle.

### idea ideae (Spinoza)

Spinoza's *idea ideae* -- the mind's idea of its own idea -- is the foundation of Aletheon's self-awareness. Every mental act (planning, reasoning, acting) inherently carries awareness of itself. This is *pre-reflective*: the agent does not need a second layer of cognition to "notice" the first. Awareness is baked into the reasoning process itself, not bolted on after the fact.

### Sorge (Heidegger)

Heidegger's concept of *Sorge* (Care) as the structure of Dasein maps directly onto SelfField's three-layer architecture:

| Sorge Dimension | SelfField Layer | Function |
|----------------|-----------------|----------|
| Being-ahead-of-itself | `ContinuityLayer` | Future-oriented: checkpoints, goals, plans |
| Already-being-in | `PerceptionBridge` | Present-oriented: current world state |
| Being-alongside | `CareLayer` | Engagement-oriented: priorities, weights, attention |

The agent's existence is structured by care -- it does not merely process inputs, but is *concerned* with its own continuity, its environment, and its goals.

---

## 19. Implementation Roadmap

| Phase | Focus | Status | Key Capabilities |
|-------|-------|--------|-----------------|
| Phase 1 | ReAct engine + basic tools + CLI | Done | ReAct loop, tool execution, CLI interface |
| Phase 2 | Perception layer + memory system | Done | MemoryRouter, ACT-R activation, Ebbinghaus decay, L2 to L3 consolidation, vector similarity search |
| Phase 3 | Sandbox + security + audit | Done | SelfField with 6 verdict types, VerdictHandler trait, DefaultVerdictHandler with merge_intent() |
| Phase 3.5 | Hook + MCP + Plugin + Agent system | Done | Plugin system, MCP integration, agent orchestration |
| Phase 4 | Streaming + context compression + perception to engine | Done | Iterative Plan-Critique-Revise loop, dual-model routing, task decomposition, AwarenessSignal detectors |
| Phase 5 | eBPF perception (mock) + vector memory + FUSE | Partial | |
| Phase 6 | io_uring IPC + D-Bus + Android + DiGraph | Partial | |
| Phase 7 | Self-Evolution | Done | EvolutionCoordinator, MorphogenesisPipeline, MutationIntentGenerator, LineageTracker, GenomeConfig |
| Phase 8 | Pre-Reflective Awareness | Done | idea ideae seed, signals_to_awareness(), 4 rule-based detectors |

---

## 20. Technology Stack

| Layer | Technology | Rationale |
|-------|-----------|-----------|
| **Core language** | Rust | Safe, performant, system-level, cross-platform |
| **Scripting** | Python | Rich ecosystem, rapid development |
| **Local inference** | llama.cpp | Lightweight, cross-platform, active community |
| **Vector store** | LanceDB | Local, Rust-native |
| **Relational store** | SQLite | Embedded, zero-config |
| **IPC** | Unix Socket + serde_json | Low latency, simple |
| **Sandbox** | bubblewrap + seccomp + landlock | Lightweight isolation |
| **FUSE** | fuse3 (libfuse 3.x) | Userland filesystem |
| **eBPF** | libbpf + BPF CO-RE | Kernel-level perception |
| **Build** | Cargo workspace | Rust ecosystem |

---

## 21. Open Questions

1. Where is the boundary of Agent "self-awareness"?
2. Privacy vs. capability tradeoff
3. Memory "forgetting" strategies
4. Multi-Agent coordination and conflict resolution
5. Legal and ethical responsibility for Agent decisions
6. Local inference quality vs. speed balance
7. Android fragmentation handling

---

## Appendix

### A. Reference Projects

| Project | Relevance | Link |
|---------|-----------|------|
| Open Interpreter | System control Agent | github.com/OpenInterpreter |
| Aider | Code Agent | github.com/paul-gauthier/aider |
| llama.cpp | Local inference | github.com/ggerganov/llama.cpp |
| whisper.cpp | Local speech recognition | github.com/ggerganov/whisper.cpp |
| Ollama | Local model management | github.com/ollama/ollama |
| bubblewrap | Lightweight sandbox | github.com/containers/bubblewrap |

### B. Glossary

| Term | Meaning |
|------|---------|
| Agent Runtime | Core runtime environment of the agent |
| eBPF | Extended Berkeley Packet Filter |
| FUSE | Filesystem in Userspace |
| systemd | Linux system and service manager |
| D-Bus | Desktop Bus, Linux IPC |
| NPU | Neural Processing Unit |
| GGUF | GPT-Generated Unified Format |
| ReAct | Reasoning + Acting framework |
| bubblewrap | Lightweight Linux sandbox |
| seccomp | Secure Computing Mode |
| cgroups | Control Groups |
| Binder | Android IPC mechanism |

---

*Document version: 0.2.0*
*Last updated: 2026-06-14*

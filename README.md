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
- [15. Implementation Roadmap](#15-implementation-roadmap)
- [16. Technology Stack](#16-technology-stack)
- [17. Open Questions](#17-open-questions)

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

Aletheon is organized as a Cargo workspace with 8 crates:

| Crate | Concept | Role |
|---|---|---|
| `base` | ABI | IPC, tool/message/sandbox/LLM types, `paths` |
| `dasein` | Self | identity, boundary, care, narrative |
| `cognit` | Brain | reasoning, planning, reflection, provider routing |
| `corpus` | Body | tools, sandbox, perception, MCP, drivers |
| `runtime` | Runtime | cognitive loop, orchestration, daemon (`aletheond`, `aletheon-exec` bins) |
| `interact` | Interface | CLI + TUI client (`aletheon` bin) |
| `memory` | Memory | cognitive memory backends (episodic/semantic/procedural/self) |
| `metacog` | Meta | self-evolution scaffolding |

Real binaries:
- `aletheond` + `aletheon-exec` — `crates/runtime/Cargo.toml:8-14`
- `aletheon` — `crates/interact/Cargo.toml:8-10`

### Crate Dependency Graph

```
aletheon (bin)  --->  interact  --->  base, corpus
aletheond (bin) --->  runtime   --->  base, cognit, corpus, dasein, memory, metacog
aletheon-exec    ---/
cognit           --->  base, corpus, interact        (* see note)
```

> **Note:** `cognit` currently depends on `corpus` and `interact` (an inversion; Tier 2c on the roadmap will fix this by moving the shared contract into `base`). This diagram describes the *current* state of the repo.

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

---

## 11. Memory System

```
L1: Working Memory (RAM, context window)
  | periodic compression
  v
L2: Short-term Memory (SQLite, GB-scale)
  | periodic consolidation
  v
L3: Long-term Memory (Vector DB, TB-scale)
  | cross-device sync (optional)
  v
L4: Shared Memory (Cloud/NAS, E2E encrypted)
```

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

## 15. Implementation Roadmap

| Phase | Focus | Status |
|-------|-------|--------|
| Phase 1 | ReAct engine + basic tools + CLI | Done |
| Phase 2 | Perception layer + memory system | Done |
| Phase 3 | Sandbox + security + audit | Done |
| Phase 3.5 | Hook + MCP + Plugin + Agent system | Done |
| Phase 4 | Streaming + context compression + perception->engine | Done |
| Phase 5 | eBPF perception (mock) + vector memory + FUSE | Partial |
| Phase 6 | io_uring IPC + D-Bus + Android + DiGraph | Partial |

---

## 16. Technology Stack

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

## 17. Open Questions

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

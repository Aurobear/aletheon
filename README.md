# Aletheon: A Persistent Self-Evolving Agent Runtime

[![CI](https://github.com/Aurobear/aletheon/actions/workflows/ci.yml/badge.svg)](https://github.com/Aurobear/aletheon/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

> An Agent that is not merely executed, but continuously exists.
> Deep integration with operating system kernels and system services.

**Platform:** Linux (Arch Linux primary) / Android / Embedded
**Created:** 2026-06-06
**Author:** aurobear

**[Operations](docs/deployment/README.md)** | **[Design Guide](docs/design/README.md)** | **[Contributing](CONTRIBUTING.md)** | **[Demo](examples/self-evolution-demo/README.md)**

---

## Table of Contents

- [1. Vision](#1-vision)
- [2. Why This Project](#2-why-this-project)
- [3. How It Differs](#3-how-it-differs)
- [4. Architecture Overview](#4-architecture-overview)
- [5. Crate Architecture](#5-crate-architecture)
- [6. Current Capabilities](#6-current-capabilities)
  - [6.1 Capability Matrix](#61-capability-matrix)
  - [6.2 Stable](#62-stable-has-code--tests)
  - [6.3 Experimental](#63-experimental-exists-behind-feature-flags-or-as-examples)
  - [6.4 Planned](#64-planned-design-only-no-implementation)
- [7. Linux Platform Design](#7-linux-platform-design)
- [8. Android Platform Design](#8-android-platform-design)
- [9. Embedded/Board Design](#9-embeddedboard-design)
- [10. Security Model](#10-security-model)
- [11. Cognitive Engine](#11-cognitive-engine)
- [12. Memory System](#12-memory-system)
- [13. Perception Layer](#13-perception-layer)
- [14. Execution Layer](#14-execution-layer)
- [15. Hybrid Inference](#15-hybrid-inference)
- [16. Implementation Roadmap](#16-implementation-roadmap)
- [17. Technology Stack](#17-technology-stack)
- [18. Open Questions](#18-open-questions)

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
| Latency        |  100ms+ network   |  Local (no network) |
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

See [the architecture overview](docs/design/architecture-overview.md) and [design documentation](docs/design/README.md) for architectural details.

---

## 5. Crate Architecture

Aletheon is organized as nine domain crates plus one executable assembly package:

| Crate | Concept | Role |
|---|---|---|
| `fabric` | ABI | IPC, tool/message/sandbox/LLM types, `paths` |
| `dasein` | Self | identity, boundary, care, narrative |
| `cognit` | Brain | reasoning, planning, reflection, provider routing |
| `corpus` | Body | tools, sandbox, perception, MCP, drivers |
| `agora` | Workspace | shared cognitive workspace: blackboard, attention, task graph, scratchpad, trace |
| `executive` | Executive | minimal orchestration and daemon implementation (cognitive loop lives in `cognit`) |
| `interact` | Interface | reusable CLI and TUI implementation |
| `mnemosyne` | Memory | cognitive memory backends (episodic/semantic/procedural/self) |
| `metacog` | Meta | self-evolution scaffolding |
| `bin` | Assembly | unified `aletheon` executable entry point; no domain logic |

Executable entry point:
- `aletheon` — assembled by `crates/aletheon` (`crates/aletheon/Cargo.toml`); provides TUI, `daemon`, and `exec` modes.

### Crate Dependency Graph

```
aletheon (crates/aletheon) ---> interact, executive, fabric, cognit, corpus
interact               ---> fabric, corpus
executive              ---> fabric, cognit, corpus, dasein, mnemosyne, metacog
cognit                 ---> fabric
```

> `crates/aletheon` is an assembly boundary only. Domain behavior remains in the nine domain crates.

---

## 6. Current Capabilities

### 6.1 Capability Matrix

| Capability | Status | Code Anchor | Tests |
|---|---|---|---|
| DaemonHost (Unix socket JSON-RPC) | ✅ Stable | `crates/executive/src/host/mod.rs` | `crates/executive/tests/` |
| SystemdHost (sd_notify, watchdog) | ✅ Stable | `crates/executive/src/host/systemd.rs` | `crates/executive/tests/` |
| ContainerHost (Docker/Podman) | 🔧 Experimental | `crates/executive/src/host/container.rs` | `crates/executive/tests/` |
| JSON-RPC server (line-delimited) | ✅ Stable | `crates/executive/src/impl/daemon/server.rs` | `crates/executive/tests/` |
| TUI client (`interact`, assembled by `bin`) | ✅ Stable | `crates/interact/src/tui/` | `crates/interact/src/tui/test_infra.rs` |
| ReActLoop inference engine | ✅ Stable | `crates/cognit/src/harness/linear/mod.rs` | `crates/executive/tests/` |
| Multi-session support | ✅ Stable | `crates/executive/src/impl/daemon/session_manager.rs` | `crates/executive/tests/` |
| Health check endpoint | ✅ Stable | `crates/executive/src/impl/daemon/handler/rpc.rs` | `crates/executive/tests/` |
| Bash/File/Grep tools | ✅ Stable | `crates/corpus/src/tools/tools/` | `crates/corpus/src/tools/` |
| Provider abstraction (Anthropic / OpenAI compatible) | ✅ Stable | `crates/cognit/src/impl/provider_registry.rs` | `crates/executive/tests/` |
| Session persistence (SQLite) | ✅ Stable | `crates/executive/src/impl/session/store.rs` | `crates/executive/tests/` |
| Hook system (lifecycle hooks) | ✅ Stable | `crates/corpus/src/hook/` | `crates/executive/tests/` |
| Bubblewrap Sandbox | ✅ Stable | `crates/corpus/src/security/sandbox/bubblewrap.rs` | `crates/corpus/tests/` |
| Multi-agent Collaboration | ✅ Stable | `crates/executive/src/impl/orchestration/agent.rs` | `crates/executive/tests/` |
| io_uring IPC backend | 🔧 Experimental | `crates/fabric/src/ipc/backends/io_uring.rs` | `crates/fabric/tests/` |
| Local/Offline Model | 🔧 Experimental | — | — |
| Self-evolution loop example | 🔧 Requires explicit opt-in | `examples/evolution_loop/` | `crates/executive/tests/self_evolution_loop_test.rs` |
| eBPF kernel awareness | 📋 Design | `crates/fabric/src/ipc/bus/kernel_bus.rs` | — |
| Android / Embedded targets | 📋 Design | — | — |
| Cross-platform (macOS / Windows) | 📋 Design | — | — |

### 6.2 Stable (has code + tests)

These capabilities have implementation and test coverage in the current repository:

- **DaemonHost + SystemdHost** — Daemon runs as a systemd service with sd_notify, watchdog, and SIGTERM graceful shutdown.
- **JSON-RPC API** — Line-delimited JSON-RPC over Unix socket with concurrent connection handling and streaming notifications (TextDelta, ToolCallStart, etc.).
- **TUI client** — Terminal UI implemented in `crates/interact/` and assembled by `crates/aletheon`, connecting to the daemon over Unix socket.
- **ReActLoop inference** — Sole production inference engine (Legacy Engine removed). Think-Act-Observe loop with streaming, tool execution, circuit breaker, and goal tracking.
- **Multi-session** — HashMap-based session registry with create/list/switch RPC methods.
- **Health check** — RPC endpoint returning uptime, active connections, session count, and version.
- **Bash/File/Grep tools** — Core tool set for filesystem interaction and command execution, with sandbox isolation.
- **Provider abstraction** — LLM provider registry supporting Anthropic API, OpenAI API, and other OpenAI-compatible endpoints with model routing.
- **Session persistence** — SQLite-backed session store with journaling and event logging.
- **Hook system** — Lifecycle hooks (session distiller, recall injection) with config-based loading.
- **Bubblewrap Sandbox** — Tool execution sandboxing via bubblewrap (bwrap) for filesystem and network isolation.
- **Multi-agent Collaboration** — Orchestration module for spawning and coordinating multiple agent instances for complex tasks.

### 6.3 Experimental (exists behind feature flags or as examples)

These have code but are gated behind features, environment variables, or exist only as examples:

- **ContainerHost** — Docker/Podman container lifecycle management. Code exists at `crates/executive/src/host/container.rs` and is selected through `aletheon daemon --container <runtime>`.
- **io_uring backend** — High-performance IPC backend using Linux io_uring. Code exists but not yet the default transport.
- **Self-evolution loop** — Example agent that modifies its own code/config. See `examples/evolution_loop/`. Requires explicit opt-in.
- **eBPF probes** — Kernel-level perception via eBPF. Partial implementation in `kernel_bus.rs`.
- **Local/Offline Model** — Support for locally-hosted inference engines (llama.cpp, Ollama). Experimental integration path.

### 6.4 Planned (design only, no implementation)

These are documented in design docs but have no working code:

- **FUSE filesystem** — Userland filesystem interface for agent state (`/mnt/aletheon/`). Design only.
- **D-Bus IPC** — Desktop Bus integration for system service communication. Design only.
- **Android target** — AccessibilityService + Foreground Service for Android. Design only.
- **Embedded/Board targets** — RK3588, Jetson Orin Nano, ESP32. Design only.
- **Vector database** — Long-term semantic memory with vector storage. Design only.
- **Cross-platform (macOS / Windows)** — Non-Linux platform support. Design only.

---

## 7. Linux Platform Design

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
# /etc/systemd/system/aletheon.service
[Unit]
Description=Aletheon Agent Service
After=network.target

[Service]
Type=notify
ExecStart=/usr/bin/aletheon daemon --config /etc/aletheon/config.toml
ProtectSystem=strict
ReadWritePaths=/home /tmp /var/lib/aletheon
WatchdogSec=30s
Restart=always

[Install]
WantedBy=multi-user.target
```

The system socket is deliberately restricted to `0660` and owned by
`aletheon:aletheon`. After a system install adds your account to the group, an
existing login session may still have stale supplementary groups. Verify with:

```bash
id -nG | grep -w aletheon
```

If it is not active, log out and back in, run `newgrp aletheon`, or use
`sg aletheon -c 'aletheon'` for a one-off TUI launch. Do not make the socket
world-writable to work around stale login credentials.

---

## 8. Android Platform Design

- AccessibilityService for screen perception
- NotificationListenerService for notification capture
- Foreground Service for persistent runtime
- Optional root extensions for shell/system control
- Intent system for app interaction

---

## 9. Embedded/Board Design

| Board | NPU | Use Case | Cost |
|-------|-----|----------|------|
| RK3588 (Rock5) | 6 TOPS | Local 7B quantized model | ~500 CNY |
| Jetson Orin Nano | 40 TOPS | Vision + language multimodal | ~2500 CNY |
| ESP32 + cloud | None | Sense + execute, cloud thinking | ~30 CNY |

---

## 10. Security Model

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

## 11. Cognitive Engine

ReAct (Think-Act-Observe) loop with multiple reasoning modes:
- **ReAct**: Reason -> Act -> Observe -> Reason -> ...
- **Plan & Execute**: Plan all steps first, then execute sequentially
- **Reflexion**: Reflect after execution, improve next behavior

---

## 12. Memory System

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

## 13. Perception Layer

Four perception domains:
- **System**: eBPF, /proc, /sys, journald, inotify, udev
- **User**: Screen OCR, keyboard/mouse, clipboard, app state, notifications
- **Environment**: Camera, microphone, sensors, GPS, time/calendar
- **Network**: DNS, HTTP traffic, RSS/Feed, message streams

---

## 14. Execution Layer

Execution sandbox per tool call:
- bubblewrap namespace -- filesystem isolation
- cgroups -- resource limits
- seccomp -- syscall filtering
- netns -- network isolation

---

## 15. Hybrid Inference

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

## 16. Implementation Roadmap

| Phase | Focus | Status |
|-------|-------|--------|
| Phase 1 | ReAct engine + basic tools + CLI | Done |
| Phase 2 | Perception layer + memory system | Done |
| Phase 3 | Sandbox + security + audit | Done |
| Phase 3.5 | Hook + MCP + Plugin + Agent system | Done |
| Phase 4 | Streaming + context compression + perception-to-engine | Done |
| P0 (stabilization) | cargo check/clippy clean, all tests pass, Legacy Engine removal | Done |
| P1 (stabilization) | EventBus to CommunicationBus partial migration, large file decomposition | Done |
| P2 (stabilization) | ReActLoop circuit breaker, goal tracker, reflection, tool exec sub-modules | Done |
| P3 (stabilization) | Docs alignment with codebase reality | Done |
| Phase 5 | eBPF perception + vector memory + FUSE | Experimental/Planned |
| Phase 6 | io_uring IPC + D-Bus + Android + DiGraph | Experimental/Planned |

See [Section 6 (Current Capabilities)](#6-current-capabilities) for detailed Stable/Experimental/Planned breakdown.

---

## 17. Technology Stack

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

The minimum supported Rust version is **1.85**. The repository pins that
toolchain for reproducible builds, while CI also verifies the current stable
release used by rolling distributions such as Arch Linux.

```bash
rustup show
cargo +1.85.0 check --workspace
cargo +stable check --workspace
```

---

## 18. Open Questions

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

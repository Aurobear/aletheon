# Aletheon 设计文档

> Phase 1-4 已全部实现，Phase 5 已实质完成，Phase 6 部分实现。设计文档保留接口规格，实现代码在 `crates/`。

> 将 AI Agent 深度融入操作系统内核与系统服务的架构设计方案。
> 目标：让 Agent 成为操作系统的"第二大脑"，而不是一个 App。

**目标平台:** Linux (Arch Linux 为主) / Android / 嵌入式开发板
**创建日期:** 2026-06-06
**作者:** aurobear
**版本:** 3.0.0

---

## Project Implementation Status

| Subsystem | Implemented | Partial | Planned | Not Started |
|-----------|-------------|---------|---------|-------------|
| **Core (cognitive-engine, memory, session)** | ReAct loop, ContentBlock, CoreMemory, RecallMemory, SessionStore, EventJournal, Context compaction, Streaming, MemoryScope (Global/Session/Agent), Memory Pipeline | ArchivalMemory, Session recovery | InterruptManager, ProactiveGoal, IdleScheduler | — |
| **Execution (tool, sandbox, IPC, MCP)** | Tool trait, 9 built-in tools, OutputManager, BubblewrapBackend, ProcessBackend, NoopBackend, SplitSandbox, ContainerSandbox, UnixSocket, PriorityQueue, IpcManager, MCP stdio/StreamableHTTP/SSE transports, BM25+TF-IDF tool search, parallel execution (RwLock+PathConflictDetector) | IoUringBackend (feature gate), SharedMemBackend (stub) | Tool exposure layers | — |
| **Security** | PolicyEngine, LoopDetector, CircuitBreaker, RiskClassifier, OutputGuardrail, Audit, ToolRunnerWithGuard, RollbackEngine, WritableRoot, IntegrityMonitor, SelfProtection, ErrorHandling | — | File-level rollback, NetworkSandboxPolicy | — |
| **Orchestration** | Agent trait, Registry, DelegateTool, Selector, Handoff, DiGraph, Termination, Budget | — | — | — |
| **Inference** | IntentClassifier, InferenceRouter (代码已实现，未接入 Engine) | — | — | — |
| **Perception** | PerceptionEvent, Manager, Aggregator, ProcSource, JournaldSource, Perception→Engine feed, FUSE AgentFs (fuse3 real mount) | eBPF source (mock /proc only), InotifySource (polling), Network monitoring (passive) | — | Hardware sensors (GPU/SMART/temp/ECC) |
| **Platform** | PlatformAdapter (Linux+Android), Boot, Agent Awareness | — | Multi-Device | Kernel IPC module (agent_ipc.ko) |
| **Resilience** | Error handling, Panic recovery | — | Rate limiting | — |
| **Observability** | EventJournal, Observability modules | — | Durable/Ephemeral split, Prometheus metrics, Debug CLI | — |
| **Testing** | 614 unit tests, Mock infrastructure (MockLlm, MockSandbox, MockMemory, MockPerception) | Integration tests (inline) | CI pipeline (GitHub Actions), E2E tests, Performance benchmarks | — |
| **Automation** | Automation system | — | — | — |
| **MCP** | MCP OAuth 2.0, StreamableHTTP + SSE transports | — | — | — |

**Legend:** ✅ Implemented (works end-to-end) | 🔶 Partial (exists but incomplete) | ⬜ Planned (designed, not started) | ❌ Not Started (no code at all)

---

## 阅读指南

**首次阅读顺序：**
1. [architecture-overview.md](architecture-overview.md) — 整体愿景与三层演化路线
2. 本文档 — 架构总览、技术选型、设计原则
3. [roadmap/phases.md](roadmap/phases.md) — 6 Phase 路线图

**按 Crate 查阅（推荐）：**

| Crate | 目录 | 核心内容 |
|-------|------|----------|
| `aletheon-abi` | [abi/](abi/) | 共享类型定义、Trait 接口、ABI 契约 |
| `aletheon-comm` | [comm/](comm/) | IPC 层、Unix Socket、消息优先队列 |
| `aletheon-memory` | [memory/](memory/) | 记忆系统：episodic/semantic/procedural/self-memory |
| `aletheon-body` | [body/](body/) | 执行层：工具、沙箱、MCP、感知、平台、驱动、UI |
| `aletheon-self` | [self/](self/) | SelfField：身份、边界、关切、叙事、Hook、安全、容错 |
| `aletheon-brain` | [brain/](brain/) | 认知引擎：推理、规划、反思、学习、推理路由 |
| `aletheon-runtime` | [runtime/](runtime/) | 运行时：ReAct 循环、会话、编排、可观测、插件、自动化 |
| `aletheon-meta` | [meta/](meta/) | MetaRuntime：自我更新、形态演化、基因组 |
| `aletheond` | [daemon/](daemon/) | 守护进程入口、配置加载、Unix Socket 服务 |
| `aletheon-cli` | [cli/](cli/) | CLI/TUI 客户端（逻辑在 body crate，thin re-export） |

**按关注点查阅：**

| 关注点 | 目录 | 核心内容 |
|--------|------|----------|
| 核心循环 | [runtime/react-loop.md](runtime/react-loop.md) | ReAct 循环、ContentBlock 协议 |
| 记忆系统 | [memory/memory-system.md](memory/memory-system.md) | 三级记忆、自学习循环、上下文预算 |
| 工具与沙箱 | [body/tools.md](body/tools.md), [body/sandbox.md](body/sandbox.md) | Tool trait、沙箱执行 |
| 安全 | [body/security.md](body/security.md), [self/](self/) | 权限、策略、自我保护、循环检测 |
| 感知 | [body/perception.md](body/perception.md) | eBPF、事件聚合、背压控制 |
| 编排 | [runtime/orchestration.md](runtime/orchestration.md) | 多 Agent 编排、Selector/Handoff/DiGraph |
| 自我演化 | [meta/](meta/) | MetaRuntime、Morphogenesis、Genome |
| 测试 | [testing/](testing/) | 测试策略、Mock、CI |
| 路线图 | [roadmap/](roadmap/) | 6 Phase 路线图、开放问题 |

---

## 整体架构

```
┌─────────────────────────────────────────────────────────────┐
│                    Aletheon System                           │
│                                                             │
│  ┌───────────────────────────────────────────────────────┐  │
│  │              aletheon-runtime (Orchestrator)           │  │
│  │                                                       │  │
│  │  ┌─────────────┐ ┌──────────────┐ ┌───────────────┐  │  │
│  │  │ ReAct Loop  │ │ Memory       │ │ Orchestration │  │  │
│  │  │ Engine      │ │ L1+L2+L3     │ │ Selector      │  │  │
│  │  │             │ │ Compressor   │ │ Handoff       │  │  │
│  │  └──────┬──────┘ └──────┬───────┘ └───────┬───────┘  │  │
│  │         │               │                 │           │  │
│  │  ┌──────┴───────────────┴─────────────────┴────────┐  │  │
│  │  │              EventBus (aletheon-comm)            │  │  │
│  │  └──────┬───────────────┬─────────────────┬────────┘  │  │
│  └─────────┼───────────────┼─────────────────┼──────────┘  │
│            │               │                 │              │
│  ┌─────────┴──────┐ ┌──────┴───────┐ ┌──────┴───────────┐  │
│  │ aletheon-body  │ │ aletheon-self│ │ aletheon-brain   │  │
│  │                │ │              │ │                  │  │
│  │ Tools/Sandbox  │ │ Identity     │ │ Reasoning        │  │
│  │ MCP/Perception │ │ Boundary     │ │ Planning         │  │
│  │ Platform/UI    │ │ Care/Hook    │ │ Reflection       │  │
│  │ Driver/ACIX    │ │ Security     │ │ Learning         │  │
│  └────────────────┘ │ Resilience   │ │ Inference        │  │
│                     └──────────────┘ └──────────────────┘  │
│                                                             │
│  ┌──────────────────────┐  ┌────────────────────────────┐  │
│  │ aletheon-memory      │  │ aletheon-abi               │  │
│  │ episodic/semantic/   │  │ Shared types, traits,      │  │
│  │ procedural/self      │  │ message, tool, sandbox     │  │
│  └──────────────────────┘  └────────────────────────────┘  │
│                                                             │
│  ┌──────────────────────┐  ┌────────────────────────────┐  │
│  │ aletheon-meta        │  │ Entry Points               │  │
│  │ MetaRuntime          │  │ aletheond (daemon)         │  │
│  │ Morphogenesis        │  │ aletheon-cli (CLI/TUI)     │  │
│  └──────────────────────┘  └────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

---

## 核心设计原则

1. **模块化** — 每个能力是一个 plugin，可独立加载/卸载
2. **安全第一** — 默认最小权限，显式授权升级
3. **可观测** — 所有决策可审计可回溯
4. **离线优先** — 本地能做的不依赖云端
5. **渐进式** — 从简单到复杂，每个阶段都有价值
6. **平台抽象** — 核心逻辑与平台无关，通过 Adapter 对接不同 OS

---

## 从开源框架借鉴的核心模式

通过深入分析 9 个主流 Agent 框架的源码，提取了可借鉴的设计模式：

| 设计维度 | 选择的模式 | 来源 |
|----------|-----------|------|
| Agent 循环 | ReAct (Think→Act→Observe) + 工具循环 | Anthropic SDK |
| 状态管理 | 类型化通道 + 检查点 | LangGraph |
| 消息格式 | Content-block 协议 | Anthropic SDK |
| 记忆系统 | 三级自编辑 (Core/Recall/Archival) | Letta |
| 编排策略 | 可插拔 (Selector/Handoff/DiGraph) | AutoGen |
| 委托机制 | 委托即工具 | CrewAI |
| 沙箱执行 | bubblewrap + seccomp + cgroups | OpenHands + Codex |
| 上下文管理 | 压缩 + HeadAndTail | Anthropic SDK + AutoGen |
| 安全护栏 | 输出验证 + 权限分级 + Guardian | CrewAI + Codex |
| 会话持久化 | Rollout + SQLite + Checkpoint | Codex + Hermes |
| 工具暴露 | 分层暴露 (Direct/Deferred/Hidden) | Codex + Hermes |
| Hook 系统 | 生命周期钩子 (Pre/Post ToolUse) | Codex + Hermes |

---

## 技术选型

| 层次 | 技术 | 选型理由 |
|------|------|----------|
| **核心语言** | Rust | 安全、高性能、系统级、跨平台 |
| **脚本/插件** | Python | 生态丰富、快速开发 |
| **本地推理** | llama.cpp | 轻量、跨平台、社区活跃 |
| **语音** | whisper.cpp | 离线语音识别 |
| **向量存储** | LanceDB | 本地向量数据库，Rust 原生 |
| **关系存储** | SQLite | 嵌入式、零配置 |
| **配置** | TOML + YAML | 可读性好 |
| **日志** | tracing (Rust) | 结构化日志 |
| **IPC** | Unix Socket + serde_json | 低延迟、简单 |
| **沙箱** | bubblewrap + seccomp + landlock | 轻量级隔离 |
| **FUSE** | fuse3 (libfuse 3.x) | 用户态文件系统 |
| **eBPF** | libbpf + BPF CO-RE | 内核级感知 |
| **构建** | Cargo workspace | Rust 生态 |

---

## 项目结构

```
aletheon/
├── Cargo.toml                  # workspace 根
│
├── crates/
│   ├── aletheon-abi/           # ABI 类型: IPC, tool, message, sandbox, LLM types
│   ├── aletheon-comm/          # IPC 层: Unix socket, priority queue
│   ├── aletheon-memory/        # 记忆系统: self-memory, episodic/semantic
│   ├── aletheon-self/          # SelfField: identity, boundary, care, narrative
│   ├── aletheon-brain/         # BrainCore: reasoning, planning, reflection
│   ├── aletheon-body/          # BodyRuntime: tools, sandbox, perception, MCP, TUI
│   ├── aletheon-runtime/       # Runtime engine: cognitive loop, orchestration, daemon
│   ├── aletheon-meta/          # MetaRuntime: self-update, self-generation
│   ├── aletheond/              # Daemon 入口
│   └── aletheon-cli/           # CLI + TUI 客户端
│
├── agents/                     # Agent 定义文件 (TOML + .md)
│   ├── fs-agent.md
│   ├── code-agent.md
│   └── net-agent.md
│
├── config/
│   └── default.toml            # 默认配置
│
├── systemd/
│   └── aletheond.service       # systemd 服务文件
│
├── references/                 # 参考项目 (~3GB, gitignored)
│
└── docs/
    ├── design/                 # 按 Crate 组织的设计文档 (本目录)
    └── plans/
```

---

## 设计文档索引

### 按 Crate 组织

| Crate | 设计文档 | 核心内容 |
|-------|---------|----------|
| **aletheon-abi** | [abi/types.md](abi/types.md) | 共享类型、Trait 定义、接口规范 |
| **aletheon-comm** | [comm/ipc.md](comm/ipc.md) | Unix Socket、io_uring、优先队列、消息路由 |
| **aletheon-memory** | [memory/memory-system.md](memory/memory-system.md) | 三级记忆、上下文预算、记忆管道、向量存储 |
| **aletheon-body** | [body/tools.md](body/tools.md) | Tool trait、并行执行、分层暴露 |
| | [body/sandbox.md](body/sandbox.md) | bubblewrap、seccomp、cgroups |
| | [body/mcp.md](body/mcp.md) | MCP 集成、OAuth、工具转换 |
| | [body/perception.md](body/perception.md) | eBPF、事件聚合、背压控制 |
| | [body/fuse.md](body/fuse.md) | FUSE 虚拟文件系统接口 |
| | [body/platform.md](body/platform.md) | 平台适配、启动集成、内核 IPC、多设备 |
| | [body/security.md](body/security.md) | 策略引擎、风险分类、审计、回滚 |
| | [body/driver.md](body/driver.md) | 显示/输入/OCR/无障碍驱动 |
| | [body/ui.md](body/ui.md) | TUI 终端界面 |
| | [body/acix.md](body/acix.md) | Agent-Computer 交互体验 |
| **aletheon-self** | [self/self-field.md](self/self-field.md) | SelfField 架构：身份/边界/关切/叙事/冲突/注意力/连续性/变异 |
| | [self/hook-system.md](self/hook-system.md) | 21 事件类型，3 层配置，命令钩子 |
| | [self/loop-detector.md](self/loop-detector.md) | 循环检测、熔断器 |
| | [self/self-protection.md](self/self-protection.md) | 注入防御、资源治理、紧急停止 |
| | [self/writable-root.md](self/writable-root.md) | 可写根路径隔离 |
| | [self/resilience.md](self/resilience.md) | 错误处理、限流、panic 恢复 |
| | [self/perception-sources.md](self/perception-sources.md) | eBPF、inotify、journald、/proc |
| **aletheon-brain** | [brain/cognitive-engine.md](brain/cognitive-engine.md) | 推理、规划、批判、反思、学习 |
| | [brain/inference.md](brain/inference.md) | 推理路由、Provider 管理 |
| **aletheon-runtime** | [runtime/react-loop.md](runtime/react-loop.md) | ReAct 循环、ContentBlock 协议 |
| | [runtime/session.md](runtime/session.md) | 会话持久化、崩溃恢复 |
| | [runtime/orchestration.md](runtime/orchestration.md) | Selector、Handoff、DiGraph |
| | [runtime/observability.md](runtime/observability.md) | Metrics、Tracing、健康检查 |
| | [runtime/plugin.md](runtime/plugin.md) | 插件系统 |
| | [runtime/automation.md](runtime/automation.md) | Cron、Webhook、脚本 |
| **aletheon-meta** | [meta/meta-runtime.md](meta/meta-runtime.md) | 自我读取、修改、回滚、迁移 |
| | [meta/morphogenesis.md](meta/morphogenesis.md) | 形态演化、Genome、候选生成 |
| **aletheond** | [daemon/README.md](daemon/README.md) | 守护进程、Unix Socket 服务 |
| **aletheon-cli** | [cli/README.md](cli/README.md) | CLI/TUI、三种运行模式 |

### 跨 Crate 文档

| 文档 | 内容 |
|------|------|
| [architecture-overview.md](architecture-overview.md) | 整体架构、数据流、演化路线 |
| [testing/test-strategy.md](testing/test-strategy.md) | 测试策略 |
| [testing/mock-strategy.md](testing/mock-strategy.md) | Mock 基础设施 |
| [testing/ci-pipeline.md](testing/ci-pipeline.md) | CI 流水线 |
| [roadmap/phases.md](roadmap/phases.md) | 6 Phase 路线图 |
| [roadmap/open-questions.md](roadmap/open-questions.md) | 开放问题 |

---

## Crate 内部结构模式

所有 crate 遵循统一的三层内部结构：

```
crates/aletheon-*/
├── src/
│   ├── core/          # 抽象类型、Trait 定义、主结构体
│   ├── bridge/        # 跨 crate 集成适配器
│   ├── impl/          # 具体实现
│   └── testing/       # Mock 基础设施
```

- **core/** — 接口契约和核心数据结构
- **bridge/** — 连接 core 和 impl 的适配层，不同后端可替换
- **impl/** — 具体实现（LLM 提供者、沙箱后端、感知源、工具实现等）
- **testing/** — 测试用 Mock 和辅助工具

---

*文档版本: 3.0.0*
*最后更新: 2026-06-14*

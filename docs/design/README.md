# OS-Agent 设计文档

> Phase 1-4 已全部实现，Phase 5 已实质完成，Phase 6 部分实现。B1-B5 实现批次 + 设计改进 PR 均已合并。设计文档保留接口规格，实现代码在 `crates/`。

> 将 AI Agent 深度融入操作系统内核与系统服务的架构设计方案。
> 目标：让 Agent 成为操作系统的"第二大脑"，而不是一个 App。

**目标平台:** Linux (Arch Linux 为主) / Android / 嵌入式开发板
**创建日期:** 2026-06-06
**作者:** aurobear
**版本:** 2.0.0

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
| **Testing** | 533 unit tests, Mock infrastructure (MockLlm, MockSandbox, MockMemory, MockPerception) | Integration tests (inline) | CI pipeline (GitHub Actions), E2E tests, Performance benchmarks | — |
| **Automation** | Automation system | — | — | — |
| **MCP** | MCP OAuth 2.0, StreamableHTTP + SSE transports | — | — | — |

**Legend:** ✅ Implemented (works end-to-end) | 🔶 Partial (exists but incomplete) | ⬜ Planned (designed, not started) | ❌ Not Started (no code at all)

---

## 阅读指南

**首次阅读顺序：**
1. [architecture-overview.md](architecture-overview.md) — 整体愿景与三层演化路线
2. 本文档 — 架构总览、技术选型、设计原则
3. [roadmap/phases.md](roadmap/phases.md) — 6 Phase 路线图

**按关注点查阅：**

| 关注点 | 目录 | 核心内容 |
|--------|------|----------|
| 核心循环 | [core/](core/) | 认知引擎、记忆系统、会话管理 |
| 执行层 | [execution/](execution/) | 工具系统、沙箱、MCP、IPC |
| 感知层 | [perception/](perception/) | eBPF、FUSE、系统服务管理 |
| 安全 | [security/](security/) | 权限、策略、自我保护 |
| 编排 | [orchestration/](orchestration/) | 多 Agent 编排、推理路由 |
| 平台 | [platform/](platform/) | 内核模块、跨平台适配、启动、多设备 |
| 容错 | [resilience/](resilience/) | 错误处理、限流、panic 恢复 |
| 可观测 | [observability/](observability/) | Metrics、Tracing、健康检查 |
| 测试 | [testing/](testing/) | 测试策略、Mock、CI |

---

## 整体架构

```
┌─────────────────────────────────────────────────────────────┐
│                    OS-Agent System                           │
│                                                             │
│  ┌───────────────────────────────────────────────────────┐  │
│  │                 Agent Daemon (Rust)                    │  │
│  │                                                       │  │
│  │  ┌─────────────┐ ┌──────────────┐ ┌───────────────┐  │  │
│  │  │ 认知引擎     │ │ 记忆系统      │ │ 编排引擎       │  │  │
│  │  │ ReAct Loop  │ │ 三级记忆     │ │ Selector      │  │  │
│  │  │ 主动行为    │ │ 自学习循环    │ │ Handoff       │  │  │
│  │  └──────┬──────┘ └──────┬───────┘ └───────┬───────┘  │  │
│  │         │               │                 │           │  │
│  │  ┌──────┴───────────────┴─────────────────┴────────┐  │  │
│  │  │              通道总线 (Channel Bus)               │  │  │
│  │  └──────┬───────────────┬─────────────────┬────────┘  │  │
│  │         │               │                 │           │  │
│  │  ┌──────┴──────┐ ┌──────┴───────┐ ┌──────┴───────┐   │  │
│  │  │ 工具系统    │ │ 感知引擎      │ │ 系统管理     │   │  │
│  │  │ 沙箱执行   │ │ eBPF+FUSE    │ │ systemd      │   │  │
│  │  │ MCP 集成   │ │ /proc /sys   │ │ udev/网络    │   │  │
│  │  └─────────────┘ └──────────────┘ └──────────────┘   │  │
│  │                                                       │  │
│  │  ┌─────────────┐ ┌──────────────┐ ┌──────────────┐   │  │
│  │  │ 安全引擎     │ │ 自我保护     │ │ 推理路由     │   │  │
│  │  │ 策略/审计   │ │ 注入防御     │ │ Local/Cloud  │   │  │
│  │  │ 回滚/Guard  │ │ 资源治理     │ │ Provider     │   │  │
│  │  └─────────────┘ └──────────────┘ └──────────────┘   │  │
│  └───────────────────────────────────────────────────────┘  │
│                         │                                    │
│  ┌──────────────────────┴────────────────────────────────┐  │
│  │              内核交互层 (渐进式)                        │  │
│  │  Phase 1: eBPF + D-Bus + FUSE + /proc                 │  │
│  │  Phase 2: eBPF 扩展 (Agent 专用 map/program)           │  │
│  │  Phase 3: agent_ipc.ko (共享内存环形缓冲区)            │  │
│  │  Phase 4: sys_agent_* syscall (按需)                   │  │
│  └───────────────────────────────────────────────────────┘  │
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
| **IPC (Phase 1-4)** | Unix Socket + serde_json | 低延迟、简单 |
| **IPC (Phase 5)** | agent_ipc.ko | 零拷贝、内核级 |
| **沙箱** | bubblewrap + seccomp + landlock | 轻量级隔离 |
| **FUSE** | fuse3 (libfuse 3.x) | 用户态文件系统 |
| **eBPF** | libbpf + BPF CO-RE | 内核级感知 |
| **构建** | Cargo workspace | Rust 生态 |
| **内核模块** | C + kbuild | 标准内核开发 |

---

## 项目结构

```
argos/
├── Cargo.toml                  # workspace 根
│
├── crates/
│   ├── agent-core/             # 核心库 (~17K 行): engine, tool, memory, security,
│   │   └── src/                #   sandbox, perception, orchestration, hook, mcp,
│   │       ├── engine.rs       #   plugin, inference, session, ipc, fuse, platform,
│   │       ├── tool/           #   llm, learning
│   │       ├── memory/         #
│   │       ├── security/       #   (所有功能模块都在此 crate 内)
│   │       ├── sandbox/        #
│   │       ├── perception/     #
│   │       ├── orchestration/  #
│   │       ├── hook/           #
│   │       ├── mcp/            #
│   │       ├── plugin/         #
│   │       ├── ipc/            #
│   │       ├── fuse/           #
│   │       └── ...
│   ├── agentd/                 # Daemon 入口 (~0.4K 行)
│   └── agent-cli/              # CLI + TUI (~2K 行)
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
│   └── agentd.service          # systemd 服务文件
│
├── references/                 # 参考项目 (~3GB, gitignored)
│
└── docs/
    ├── design/                 # 模块化设计文档 (本目录)
    └── plans/
```

---

## 模块设计文档索引

| 目录 | 模块 | 核心内容 |
|------|------|----------|
| [core/](core/) | 认知引擎 | ReAct 循环、主动行为、消息协议、上下文压缩 |
| [core/](core/) | 记忆系统 | 三级记忆、自学习循环、上下文预算 |
| [core/](core/) | 会话管理 | 持久化、崩溃恢复、Hook 系统 |
| [execution/](execution/) | 工具系统 | Tool trait、并行执行、分层暴露 |
| [execution/](execution/) | 沙箱 | bubblewrap、seccomp、cgroups |
| [execution/](execution/) | MCP | MCP 集成、OAuth、工具转换 |
| [execution/](execution/) | IPC | Unix socket、消息协议 |
| [perception/](perception/) | 感知层 | eBPF、事件聚合、背压控制 |
| [perception/](perception/) | FUSE | 虚拟文件系统接口 |
| [perception/](perception/) | 系统管理 | systemd、udev、网络、包管理 |
| [security/](security/) | 安全模型 | 权限分级、策略引擎、审计、回滚 |
| [security/](security/) | 自我保护 | 注入防御、资源治理、紧急停止 |
| [orchestration/](orchestration/) | 编排引擎 | Selector、Handoff、DiGraph |
| [orchestration/](orchestration/) | 推理路由 | 本地/云端路由、Provider 配置 |
| [platform/](platform/) | 内核 IPC | agent_ipc.ko、syscall 扩展 |
| [platform/](platform/) | 平台适配 | PlatformAdapter trait |
| [platform/](platform/) | 启动集成 | 开机自举、启动监控 |
| [platform/](platform/) | 多设备 | 设备发现、记忆同步、任务委托 |
| [resilience/](resilience/) | 容错 | 错误处理、限流、panic 恢复 |
| [observability/](observability/) | 可观测 | Metrics、Tracing、健康检查 |
| [testing/](testing/) | 测试 | 测试策略、Mock、CI |

---

*文档版本: 2.1.0*
*最后更新: 2026-06-07*

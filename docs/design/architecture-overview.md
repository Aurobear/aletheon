# Aletheon 架构总览

> 永久运行的系统级 AI Agent，深度集成 Linux，面向机器人和边缘计算场景。

---

## 1. 设计理念

当前 Agent（Claude Code、Codex、Cursor）都是**应用层 Agent** —— 运行在终端/浏览器中，能操作代码和文件，但无法感知系统状态。

Aletheon 的目标是成为**系统层 Agent** —— 能感知内核事件、管理服务、诊断硬件问题，最终演化为 AI Native Operating Environment。

**核心原则：利用内核，而非修改内核。** 通过 eBPF、procfs、journald 等现有机制获取系统感知，不需要修改 Linux 内核。

---

## 2. 当前架构

```
┌─────────────────────────────────────────────────────────┐
│                  interact (TUI)                      │
│              ratatui + markdown rendering                │
└────────────────────────┬────────────────────────────────┘
                         │ Unix Socket (JSON-RPC)
┌────────────────────────┴────────────────────────────────┐
│                   aletheond (daemon)                     │
│  ┌────────────────────────────────────────────────────┐ │
│  │            executive (orchestrator)         │ │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────────────┐ │ │
│  │  │ LLM      │  │ Tool     │  │ Security Pipeline│ │ │
│  │  │ Provider │→ │ Registry │→ │ Policy+Loop+     │ │ │
│  │  │ (2 impl) │  │(12 tools)│  │ Sandbox+Audit   │ │ │
│  │  └──────────┘  └──────────┘  └──────────────────┘ │ │
│  │       ↕              ↕               ↕             │ │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────────────┐ │ │
│  │  │ Memory   │  │ Hook     │  │ Perception       │ │ │
│  │  │ System   │  │ System   │  │ Bridge           │ │ │
│  │  │ L1+L2+L3 │  │ (21 evt) │  │ (event→engine)   │ │ │
│  │  └──────────┘  └──────────┘  └──────────────────┘ │ │
│  │       ↕              ↕               ↕             │ │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────────────┐ │ │
│  │  │ Orchest- │  │ MCP      │  │ Plugin           │ │ │
│  │  │ ration   │  │ Client   │  │ Runtime          │ │ │
│  │  │ (agents) │  │ (stdio)  │  │ (cmd/native)     │ │ │
│  │  └──────────┘  └──────────┘  └──────────────────┘ │ │
│  └────────────────────────────────────────────────────┘ │
│  ┌────────────────────────────────────────────────────┐ │
│  │              Perception Sources                     │ │
│  │  /proc polling │ inotify │ journald │ eBPF(mock)  │ │
│  └────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

---

## 3. Crate 架构

```
fabric          (no deps -- pure interfaces)
    ^
    |
mnemosyne       (depends on fabric, cognit, corpus)
    ^
    |
agora           (depends on fabric)
    ^
    |
corpus         (depends on fabric)
    ^
    |
cognit        (depends on fabric)
    ^
    |
dasein         (depends on fabric, corpus, mnemosyne)
    ^
    |
executive      (depends on fabric, cognit, corpus, mnemosyne, agora, dasein, metacog)
    ^
    |
interact          (depends on fabric, corpus)

metacog         (depends on fabric -- self-evolution)

bin             (aletheon-bin; depends on executive, interact, fabric, cognit -- aletheond/aletheon binary)
```

10 crates total: `fabric`, `mnemosyne`, `agora`, `corpus`, `cognit`, `dasein`, `executive`, `metacog`, `interact`, `bin`.

> The vertical order above is illustrative; the authoritative build dependencies are the parentheticals. Note `mnemosyne` still depends on `cognit`/`corpus` (CompactorTrait + tool-output pruner) — a residual cross-layer coupling tracked as RFC-018 D4 (Phase 4 removed the `LlmProvider` half by moving it to `fabric`; `dasein` is now cognit-free).

### Crate 职责

| Crate | 角色 | 核心内容 |
|-------|------|----------|
| `fabric` | 契约层+通信层 | 零实现 Trait 定义、共享类型（message, tool, sandbox, llm_types, memory, event, genome）、EventBus、Unix Socket、io_uring、优先队列、消息路由 |
| `mnemosyne` | 记忆层 | SQLite 后端：episodic、semantic、procedural、self_memory、MemoryRouter |
| `agora` | 共享工作区层 | 会话隔离的认知工作区：blackboard、attention、task_graph、trace、scratchpad，`AgoraOps` |
| `corpus` | 执行层 | 工具、沙箱、MCP、平台适配、驱动 |
| `dasein` | 主体场 | 身份、边界、关切、叙事、冲突、注意力、连续性、变异、感知、安全、容错 |
| `cognit` | 认知层 | 推理、规划、批判、反思、学习、LLM 桥接、推理路由 |
| `executive` | 编排层 | Harness（当前 linear ReAct，经 HarnessKind/build_harness 工厂选择）、会话、编排、记忆管道、插件、自动化、守护进程、Hook 系统 |
| `metacog` | 自演化层 | MetaRuntime、Morphogenesis、Genome、候选生成、沙箱测试、迁移 |
| `interact` | 客户端 | CLI/TUI、三种模式（单消息/TUI/REPL）、ACIX |
| `bin` | 入口层 | `aletheon-bin`：aletheond/aletheon 二进制，daemon/exec/TUI 入口 |

### 内部结构模式

所有 crate 遵循统一的三层结构：

```
crates/*/
├── src/
│   ├── core/          # 抽象类型、Trait 定义、主结构体
│   ├── bridge/        # 跨 crate 集成适配器
│   ├── impl/          # 具体实现
│   └── testing/       # Mock 基础设施
```

---

## 4. 模块总览

| 模块 | 状态 | 说明 | Crate | 设计文档 |
|------|------|------|-------|---------|
| **认知引擎** | ✅ | ReAct 循环，content-block 协议；流式输出 | executive + cognit | [executive/react-loop.md](executive/react-loop.md), [cognit/cognitive-engine.md](cognit/cognitive-engine.md) |
| **LLM 提供者** | ✅ | OpenAI 兼容 + Anthropic，流式 SSE | cognit | [cognit/inference.md](cognit/inference.md) |
| **混合推理路由** | 🔶 | IntentClassifier + InferenceRouter 代码已实现，但未接入 Engine | cognit | [cognit/inference.md](cognit/inference.md) |
| **工具系统** | ✅ | 9 个内置工具，沙箱执行 | corpus | [corpus/tools.md](corpus/tools.md) |
| **安全层** | ✅ | 策略引擎，循环检测，熔断器，审计日志 | corpus + dasein | [corpus/security.md](corpus/security.md), [corpus/loop-detector.md](corpus/loop-detector.md) |
| **沙箱** | ✅ | bubblewrap / process / noop 三种后端 | corpus | [corpus/sandbox.md](corpus/sandbox.md) |
| **记忆系统** | ✅ | L1 CoreMemory + L2 Recall(SQLite) + L3 Archival(向量搜索 🔶) | mnemosyne + executive | [mnemosyne/memory-system.md](mnemosyne/memory-system.md) |
| **共享认知工作区** | 🔶 | Blackboard/Attention/TaskGraph/Trace/Scratchpad，`AgoraOps`（publish/recall/update/snapshot/clear/trace）；trace 尚未全量写入、snapshot 尚未持久化到 Mnemosyne（RFC-018 Phase 1 追踪中） | agora | [agora/README.md](agora/README.md) |
| **感知层** | ✅ | /proc + inotify(轮询) + journald + eBPF(mock) + 瓶颈检测 | corpus + dasein | [dasein/perception.md](dasein/perception.md), [dasein/perception-sources.md](dasein/perception-sources.md) |
| **多 Agent** | ✅ | Agent trait，委托，选择器/交接策略，DiGraph 工作流 | executive | [executive/orchestration.md](executive/orchestration.md) |
| **Hook 系统** | ✅ | 21 事件类型，3 层 TOML 配置，命令钩子 | executive | [executive/hook-system.md](executive/hook-system.md) |
| **MCP 客户端** | ✅ | stdio/StreamableHTTP/SSE 传输，OAuth 2.0 | corpus | [corpus/mcp.md](corpus/mcp.md) |
| **插件系统** | ✅ | 命令子进程运行时，清单加载，工具注册 | executive | [executive/plugin.md](executive/plugin.md) |
| **Agent 系统** | ✅ | TOML+Markdown 配置驱动，内置 3 个 Agent | executive | [executive/orchestration.md](executive/orchestration.md) |
| **会话管理** | ✅ | SQLite 存储，JSONL 事件日志，崩溃恢复 | executive | [executive/session.md](executive/session.md) |
| **TUI/CLI** | ✅ | ratatui 终端界面，markdown 渲染，技能系统 | corpus + interact | [interact/ui.md](interact/ui.md), [interact/README.md](interact/README.md) |
| **上下文压缩** | ✅ | LLM 摘要压缩，HeadAndTail 策略 | executive | [executive/react-loop.md](executive/react-loop.md) |
| **IPC 层** | ✅ | Unix Socket + JSON-RPC + io_uring(🔶) + 共享内存(stub) | fabric | [fabric/ipc.md](fabric/ipc.md) |
| **FUSE 文件系统** | ✅ | fuse3 真实挂载，context/controls/sensors/logs | dasein | [corpus/fuse.md](corpus/fuse.md) |
| **向量记忆** | 🔶 | L3 ArchivalMemory 语义搜索（Qdrant + LanceDB 双后端） | executive | [mnemosyne/memory-system.md](mnemosyne/memory-system.md) |
| **回滚引擎** | ✅ | 三层回滚（btrfs 快照/文件备份/审计日志） | corpus | [corpus/security.md](corpus/security.md) |
| **平台适配器** | ✅ | PlatformAdapter trait + Linux(systemd/D-Bus) + Android | corpus | [corpus/platform.md](corpus/platform.md) |
| **SelfField** | ✅ | 身份/边界/关切/叙事/冲突/注意力/连续性/变异 8 层 | dasein | [dasein/self-field.md](dasein/self-field.md) |
| **MetaRuntime** | 🔶 | 自我读取/修改/回滚/迁移，设计骨架 | metacog | [metacog/meta-runtime.md](metacog/meta-runtime.md) |
| **Morphogenesis** | 🔶 | 形态演化 pipeline，Genome 模型 | metacog | [metacog/morphogenesis.md](metacog/morphogenesis.md) |
| **自动化** | ✅ | Cron、Webhook、脚本、投递 | executive | [executive/automation.md](executive/automation.md) |

---

## 5. 数据流

### 用户请求流

```
User → interact → Unix Socket → aletheond handler
  → Engine.run_turn()
    → Hook: PreLLMCall
    → LLM.complete() / complete_stream()
    → Parse tool_use blocks
    → For each tool call:
        → Hook: PreToolUse
        → SelfField.review()              ← CENTRAL GATE (per-tool)
           ├─ Boundary: pattern match (fast gate)
           ├─ Identity: who am I?
           ├─ Care: weighted concern scoring
           ├─ Attention: focus priority & decay
           ├─ Conflict: multi-source arbitration
           ├─ Narrative: record decision reason
           ├─ Continuity: lineage check
           └─ Mutation: self-modification approval
        → Verdict (Allow/Deny/Confirm/Sandbox/Delay)
        → PolicyEngine: permission check
        → LoopDetector: cycle detection
        → SandboxExecutor: isolation
        → Tool.execute()
        → OutputGuardrail: validate output
        → AuditLogger: record
        → Hook: PostToolUse
    → After all tool calls:
        → SelfField: refresh DaseinContext      ← per-iteration
           (update attention decay, narrative ring buffer,
            continuity lineage, care weights)
    → Loop until no tool calls or max iterations
    → Hook: PostLLMCall
  → Response → interact TUI
```

### 感知事件流

```
/proc, inotify, journald → PerceptionManager (5s poll)
  → EventAggregator (dedup, rate limit, priority boost)
  → mpsc channel
  → PerceptionBridge
    → Critical/High: immediate injection as system message
    → Medium/Low: buffered, flushed every 30s
  → Engine.drain_perceptions() → injected into message history
```

### 多 Agent 委托流

```
Engine.run_turn()
  → LLM returns tool_use: delegate_task
  → DelegateTool.execute()
    → SelectorStrategy: LLM selects best agent
    → ConfigAgent.handle_task()
      → Scoped system prompt + tools
      → Own LLM loop (independent iteration budget)
      → Return AgentResponse
  → Result injected as tool_result
```

---

## 6. 配置层次

```
/etc/aletheon/config.toml     # 系统配置
~/.aletheon/config.toml        # 用户配置 (覆盖系统)
.aletheon/config.toml          # 项目配置 (覆盖用户)
```

配置内容：
- `[agent]` — 默认提供者、模型、迭代限制
- `[[providers]]` — LLM 提供者列表（名称、URL、API key、传输方式）
- `[[mcp_servers]]` — MCP 服务器配置
- `[compaction]` — 上下文压缩参数

---

## 7. 演化路线

| 阶段 | 重点 | 状态 |
|------|------|------|
| Phase 1 | ReAct 引擎 + 基本工具 + CLI | ✅ 完成 |
| Phase 2 | 感知层 + 记忆系统 | ✅ 完成 |
| Phase 3 | 沙箱 + 安全 + 审计 | ✅ 完成 |
| Phase 3.5 | Hook + MCP(stdio) + Plugin + Agent 系统 | ✅ 完成 |
| Phase 4 | 流式输出 + 上下文压缩 + 感知→引擎 | ✅ 完成 |
| Phase 5 | eBPF 感知(mock) + 向量记忆(🔶) + FUSE(✅) | 🔶 部分完成 |
| Phase 6 | io_uring IPC + D-Bus + Android + DiGraph + 插件系统 | 🔶 部分完成 |

详细路线图见 [roadmap/phases.md](roadmap/phases.md)。

---

## 8. 参考来源

| 项目 | 借鉴内容 |
|------|---------|
| Anthropic SDK | ReAct 循环，content-block 协议 |
| Letta/MemGPT | 三级记忆架构 |
| Hermes Agent | Hook 系统，技能系统，插件边界 |
| Claude Code | Agent 系统提示驱动，工具作用域 |
| Codex | 沙箱执行，模块大小限制 |
| OpenCode | 上下文管理，会话持久化 |
| LangGraph | 类型化通道，检查点 |

---

*文档版本: 3.0.0*
*最后更新: 2026-06-14*

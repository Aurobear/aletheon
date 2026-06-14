# OS-Agent 架构总览

> 永久运行的系统级 AI Agent，深度集成 Linux，面向机器人和边缘计算场景。

---

## 1. 设计理念

当前 Agent（Claude Code、Codex、Cursor）都是**应用层 Agent** —— 运行在终端/浏览器中，能操作代码和文件，但无法感知系统状态。

OS-Agent 的目标是成为**系统层 Agent** —— 能感知内核事件、管理服务、诊断硬件问题，最终演化为 AI Native Operating Environment。

**核心原则：利用内核，而非修改内核。** 通过 eBPF、procfs、journald 等现有机制获取系统感知，不需要修改 Linux 内核。

---

## 2. 当前架构

```
┌─────────────────────────────────────────────────────────┐
│                    agent-cli (TUI)                       │
│              ratatui + markdown rendering                │
└────────────────────────┬────────────────────────────────┘
                         │ Unix Socket (JSON-RPC)
┌────────────────────────┴────────────────────────────────┐
│                     agentd (daemon)                      │
│  ┌────────────────────────────────────────────────────┐ │
│  │                 Engine (ReAct Loop)                 │ │
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

## 3. 模块总览

| 模块 | 状态 | 说明 | 设计文档 |
|------|------|------|---------|
| **认知引擎** | ✅ | ReAct 循环，content-block 协议；流式输出(✅) | [core/cognitive-engine.md](core/cognitive-engine.md) |
| **LLM 提供者** | ✅ | OpenAI 兼容 + Anthropic，流式 SSE | [shared/traits.md](shared/traits.md) |
| **混合推理路由** | 🔶 | IntentClassifier + InferenceRouter 代码已实现，但未接入 Engine（Engine 直接用 ProviderRegistry） | [orchestration/hybrid-inference.md](orchestration/hybrid-inference.md) |
| **工具系统** | ✅ | 9 个内置工具（含 ebpf_compile/module_build/module_load/kernel_build），沙箱执行 | [execution/tool-system.md](execution/tool-system.md) |
| **安全层** | ✅ | 策略引擎，循环检测，熔断器，审计日志 | [security/security-model.md](security/security-model.md) |
| **沙箱** | ✅ | bubblewrap / process / noop 三种后端 | [execution/sandbox.md](execution/sandbox.md) |
| **记忆系统** | ✅ | L1 CoreMemory + L2 Recall(SQLite) + L3 Archival(向量搜索 🔶 Partial) | [core/memory-system.md](core/memory-system.md) |
| **感知层** | ✅ | /proc + inotify(轮询) + journald + eBPF(mock /proc 回退) + 瓶颈检测，事件桥接到引擎 | [perception/perception-layer.md](perception/perception-layer.md) |
| **多 Agent** | ✅ | Agent trait，委托，选择器/交接策略，DiGraph 工作流 | [orchestration/orchestration-engine.md](orchestration/orchestration-engine.md) |
| **Hook 系统** | ✅ | 21 事件类型（含 7 个内核操作事件），3 层 TOML 配置，命令钩子 | [core/hook-system.md](core/hook-system.md) |
| **MCP 客户端** | 🔶 | stdio 传输已实现（connect + 工具发现 + 调用），SSE/HTTP 待实现 | [execution/mcp-integration.md](execution/mcp-integration.md) |
| **插件系统** | ✅ | 命令子进程运行时，清单加载，工具注册 | [execution/tool-system.md](execution/tool-system.md) |
| **Agent 系统** | ✅ | TOML+Markdown 配置驱动，内置 3 个 Agent | [orchestration/orchestration-engine.md](orchestration/orchestration-engine.md) |
| **会话管理** | ✅ | SQLite 存储，JSONL 事件日志，崩溃恢复 | [core/session-lifecycle.md](core/session-lifecycle.md) |
| **TUI/CLI** | ✅ | ratatui 终端界面，markdown 渲染，技能系统 | — |
| **上下文压缩** | ✅ | LLM 摘要压缩，HeadAndTail 策略，消息数超阈值自动触发 | [core/cognitive-engine.md](core/cognitive-engine.md) |
| **IPC 层** | ✅ | Unix Socket(完整) + JSON-RPC 适配器 + io_uring(feature gate) + 共享内存(stub) | [execution/ipc.md](execution/ipc.md) |
| **eBPF 感知** | 🔶 | mock /proc 回退可用（sched/net/block），真实 eBPF ring buffer 读取未实现 | [perception/perception-layer.md](perception/perception-layer.md) |
| **FUSE 文件系统** | 🔶 | 内存虚拟 FS API（context/controls/sensors/logs），未接入 fuse3 挂载 | [perception/fuse-interface.md](perception/fuse-interface.md) |
| **向量记忆** | 🔶 | L3 ArchivalMemory 语义搜索（Qdrant + LanceDB 双后端，内存存根实现） | [core/memory-system.md](core/memory-system.md) |
| **瓶颈检测** | ✅ | CPU/内存/IO/网络瓶颈检测 + 升级建议（eBPF→模块→内核→硬件） | [perception/perception-layer.md](perception/perception-layer.md) |
| **内核模块工具** | ✅ | module_build + module_load，编译和加载 .ko 模块 | [execution/tool-system.md](execution/tool-system.md) |
| **内核编译工具** | ✅ | kernel_build（clone/config/build/install），完整内核编译 | [execution/tool-system.md](execution/tool-system.md) |
| **回滚引擎** | ✅ | 三层回滚（btrfs 快照/文件备份/审计日志），自动选择最佳后端 | [security/security-model.md](security/security-model.md) |
| **平台适配器** | ✅ | PlatformAdapter trait + Linux(systemd/D-Bus) + Android(getprop/dumpsys) | [platform/platform-adapter.md](platform/platform-adapter.md) |
| **D-Bus 集成** | ✅ | Linux 平台适配器，systemd 服务管理，polkit 权限提升 | [platform/platform-adapter.md](platform/platform-adapter.md) |
| **Android** | ✅ | Android 平台适配器，服务管理(getprop/dumpsys)，su/adb root，无实际测试环境 | [platform/platform-adapter.md](platform/platform-adapter.md) |

---

## 4. Workspace 结构

```
aletheon/
├── Cargo.toml                    # Workspace root
├── config/default.toml           # 默认配置
├── agents/                       # Agent 定义 (TOML + Markdown)
│   ├── fs-agent.toml + .md
│   ├── code-agent.toml + .md
│   └── net-agent.toml + .md
├── crates/
│   ├── aletheon-abi/             # ABI 类型: IPC, tool, message, sandbox, LLM
│   ├── aletheon-comm/            # IPC 层: Unix socket, priority queue
│   ├── aletheon-memory/          # 记忆系统: self-memory, episodic/semantic
│   ├── aletheon-self/            # SelfField: identity, boundary, care, narrative
│   ├── aletheon-brain/           # BrainCore: reasoning, planning, reflection
│   ├── aletheon-body/            # BodyRuntime: tools, sandbox, perception, MCP, TUI
│   ├── aletheon-runtime/         # Runtime engine: cognitive loop, orchestration
│   ├── aletheon-meta/            # MetaRuntime: self-update, self-generation
│   ├── aletheond/                # Daemon 入口
│   └── aletheon-cli/             # CLI + TUI 客户端
├── docs/
│   ├── design/                   # 设计文档 (36 个)
│   └── plans/                    # 设计计划
└── references/                   # 参考项目 (Hermes, Codex, Claude Code, OpenCode)
```

---

## 5. 数据流

### 用户请求流

```
User → agent-cli → Unix Socket → agentd handler
  → Engine.run_turn()
    → Hook: PreLLMCall
    → LLM.complete() / complete_stream()
    → Parse tool_use blocks
    → For each tool call:
        → Hook: PreToolUse
        → PolicyEngine: permission check
        → LoopDetector: cycle detection
        → SandboxExecutor: isolation
        → Tool.execute()
        → OutputGuardrail: validate output
        → AuditLogger: record
        → Hook: PostToolUse
    → Loop until no tool calls or max iterations
    → Hook: PostLLMCall
  → Response → agent-cli TUI
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
| Phase 5 | eBPF 感知(mock) + 向量记忆(🔶) + FUSE(🔶内存API) | 🔶 部分完成 |
| Phase 6 | io_uring IPC + D-Bus + Android + DiGraph + 插件系统 | 🔶 部分完成 (3/8) |

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

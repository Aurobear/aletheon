# Aletheon 项目架构文档

> 基于实际代码分析，版本 0.1.0，分析日期 2026-07-10

---

## 目录

1. [项目概览](#1-项目概览)
2. [架构哲学](#2-架构哲学)
3. [Crate 结构与依赖关系](#3-crate-结构与依赖关系)
4. [base — ABI 接口层](#4-base--abi-接口层)
5. [cognit — 认知计算引擎](#5-cognit--认知计算引擎)
6. [corpus — 工具执行体](#6-corpus--工具执行体)
7. [dasein — 自我策略引擎](#7-dasein--自我策略引擎)
8. [runtime — 运行时与编排](#8-runtime--运行时与编排)
9. [interact — 用户交互层](#9-interact--用户交互层)
10. [memory — 记忆系统](#10-memory--记忆系统)
11. [metacog — 元运行时](#11-metacog--元运行时)
12. [aletheon — 统一入口](#12-aletheon--统一入口)
13. [aletheon-monitor — 监控工具](#13-aletheon-monitor--监控工具)
14. [数据流与执行路径](#14-数据流与执行路径)
15. [附录：文件清单](#15-附录文件清单)

---

## 1. 项目概览

Aletheon 是一个用 Rust 编写的自进化 AI Agent 运行时。核心理念是 **Agent = Runtime + Subject + Evolution**，而非传统的 Model + Tools + Prompt。

### 1.1 技术栈

| 组件 | 技术选型 |
|------|----------|
| 语言 | Rust 2021 edition, MSRV 1.85 |
| 异步运行时 | Tokio (full features) |
| 序列化 | serde + serde_json + bincode + toml |
| 数据库 | SQLite (rusqlite, bundled) |
| 日志 | tracing + tracing-subscriber |
| HTTP | reqwest 0.12 |
| IPC | Unix domain socket, nix |
| 沙箱 | bubblewrap, seccomp (nix) |
| TUI | ratatui 0.29 + crossterm 0.28 |
| 语法解析 | tree-sitter + tree-sitter-rust |
| 并发 | dashmap, parking_lot, tokio-util |

### 1.2 目录树总览

```
aletheon/
├── crates/           # 9 个 Rust crate（核心代码）
├── examples/         # 示例项目 (basic-agent, self-evolution-loop)
├── agents/           # 运行时 agent 角色定义 (code-agent, fs-agent, net-agent)
├── config/           # systemd 服务单元 + 默认配置
├── docs/             # 设计文档、架构文档、路线图
├── scripts/          # 测试脚本 (5 个)
├── tests/            # 顶层测试
├── tools/            # 辅助工具
└── .github/          # CI/CD workflows
```

---

## 2. 架构哲学

### 2.1 三层架构（Nous Triune）

Aletheon 采用类似 Linux 内核的模块化分层设计，核心概念是 **Nous Triune** 三层架构：

```
┌──────────────────────────────────────────────────┐
│                   SelfField                       │
│         "我应该做什么？" (Should I?)              │
│     策略引擎 — 类似 LSM / SELinux                 │
├──────────────────────────────────────────────────┤
│                  BrainCore                        │
│        "我如何做？" (How do I?)                   │
│   认知计算 — 类似 CFS 调度器                      │
├──────────────────────────────────────────────────┤
│                 BodyRuntime                       │
│    "执行它" (Execute it)                          │
│   设备 HAL — 类似 device_ops / file_operations    │
└──────────────────────────────────────────────────┘
         ↕                ↕                ↕
    ┌──────────────────────────────────────────┐
    │              EventBus / IPC              │
    │         跨子系统通信（中断控制器）          │
    └──────────────────────────────────────────┘
    ┌──────────────────────────────────────────┐
    │              Memory System               │
    │   情景/语义/程序/自我 记忆（类似 VFS）     │
    └──────────────────────────────────────────┘
    ┌──────────────────────────────────────────┐
    │            MetaRuntime                    │
    │    自我修改引擎（类似 module 子系统）      │
    └──────────────────────────────────────────┘
```

### 2.2 Linux 内核设计类比

每个子系统都借鉴了 Linux 内核的设计模式：

| Aletheon 概念 | Linux 内核类比 | 职责 |
|---------------|---------------|------|
| Subsystem trait | `module_init` / `module_exit` | 统一切换生命周期 |
| SelfField | LSM / SELinux | 安全策略决策 |
| BrainCore | CFS Scheduler | CPU 调度（认知资源） |
| BodyRuntime | device_ops / HAL | 硬件抽象 |
| EventBus / CommunicationBus | 中断控制器 / netlink | 消息路由 |
| Memory (VFS-like) | VFS (ext4, tmpfs, procfs) | 多后端存储抽象 |
| MetaRuntime | module 子系统 (modprobe) | 热加载/升级 |
| Context | task_struct | 请求生命周期上下文 |
| Version | modinfo version | ABI 兼容性检查 |

### 2.3 第一性原则

从 `dasein/src/lib.rs:8-11`：

> **Everything is interpreted by the Self.** SelfField 不是一个模块 — 它是每个事件、意图、记忆和动作都必须经过的场。就像 Linux 围绕着进程原语组织，Unix 围绕着文件原语组织一样，Aletheon 围绕着 Self 原语组织。

---

## 3. Crate 结构与依赖关系

### 3.1 工作区成员（11 个）

```toml
# Cargo.toml:1-15
[workspace]
resolver = "2"
members = [
    "crates/aletheon",    # 统一 CLI 入口
    "crates/base",        # ABI 接口层（零实现，仅 trait + 类型）
    "crates/cognit",      # 认知计算引擎（推理/规划/反思/学习）
    "crates/corpus",      # 工具执行体（沙箱/驱动/MCP/技能）
    "crates/dasein",      # 自我策略引擎（身份/边界/关怀/叙事）
    "crates/interact",    # 用户交互（TUI + CLI + ACIX）
    "crates/memory",      # 记忆系统（SQLite 后端）
    "crates/metacog",     # 元运行时（自我修改/基因组/形态发生）
    "crates/runtime",     # 运行时编排（ReAct 循环/守护进程/会话）
    "examples/basic-agent",
    "examples/self-evolution-loop",
]
```

### 3.2 依赖关系图

```
                    ┌──────────┐
                    │ aletheon │  CLI 入口 (bin)
                    └────┬─────┘
                         │
              ┌──────────┼──────────┐
              │          │          │
         ┌────▼───┐ ┌───▼────┐ ┌───▼─────┐
         │runtime │ │interact│ │ (direct)│
         │ (集成)  │ │ (TUI)  │ │ deps    │
         └──┬──┬──┘ └───┬────┘ └───┬─────┘
            │  │        │          │
     ┌──────┤  ├────────┤          │
     │      │  │        │          │
┌────▼─┐ ┌─▼──▼──┐ ┌───▼───┐ ┌───▼────┐
│cognit│ │dasein │ │corpus │ │metacog │
│(大脑) │ │(自我) │ │(工具) │ │(元运行)│
└──┬───┘ └──┬───┘ └──┬───┘ └───┬────┘
   │        │        │         │
   └────────┼────────┼─────────┘
            │        │
       ┌────▼──┐ ┌──▼─────┐
       │memory │ │  base  │ ← 零实现 ABI 层
       │(记忆) │ │ (接口) │   所有 crate 都依赖它
       └───────┘ └────────┘
```

**关键依赖事实**（来自各 `Cargo.toml`）：
- `base` — 无内部依赖，纯接口层
- `memory` — 仅依赖 `base`
- `metacog` — 仅依赖 `base`
- `cognit` — 依赖 `base`
- `corpus` — 依赖 `base`
- `dasein` — 依赖 `base` + `corpus` + `cognit` + `memory`
- `runtime` — **集成层**，依赖所有其他 crate
- `interact` — 依赖 `base` + `corpus`
- `aletheon` — 依赖 `runtime` + `interact` + `base` + `cognit` + `corpus`

---

## 4. base — ABI 接口层

**路径**: `crates/base/src/`
**Cargo.toml**: `crates/base/Cargo.toml` — 描述为 "Aletheon ABI layer - public API interfaces for agent runtime"

### 4.1 设计原则

`base` crate **只包含接口定义，不含任何业务逻辑实现**（`lib.rs:4` — "This crate contains **zero implementations** — only interfaces"）。

模块布局仿 Linux 内核头文件组织（`lib.rs:11-18`）：

```
base/src/
├── include/     # 子系统 trait 契约（类似 kernel include/）
├── types/       # 共享数据类型
├── events/      # 事件系统（类型 + 基础设施）
├── ipc/         # 进程间通信（类似 kernel net/）
├── kernel/      # 核心基础设施（可观测性、注册表、调试、错误）
├── policy/      # 执行策略引擎
└── dasein/      # 现象学模块
```

### 4.2 include/ — 子系统 Trait 契约

每个文件定义一个子系统 trait，是 Aletheon 的"内核头文件"：

| 文件 | Trait | 类比 | 核心方法 |
|------|-------|------|----------|
| `subsystem.rs` | `Subsystem` | module_init/exit | `init()`, `health()`, `shutdown()`, `version()` |
| `self_field.rs` | `SelfFieldOps` | LSM/SELinux | `review()`, `identity()`, `cares()`, `narrate()`, `resolve_conflict()`, `review_mutation()` |
| `brain.rs` | `BrainCoreOps` | CFS Scheduler | `think()`, `reflect()`, `critique()`, `learn()`, `update_world()` |
| `body.rs` | `BodyRuntime` | device_ops / HAL | `execute()`, `capabilities()`, `check()` |
| `memory.rs` | `MemoryBackend` | VFS | `store()`, `recall()`, `list()`, `forget()`, `compact()`, `stats()` |
| `runtime.rs` | `RuntimeOps` | init/systemd | `orchestrate()`, `agents()`, `schedule()`, `health_all()`, `step()` |
| `meta.rs` | `MetaRuntimeOps` | module 子系统 | `read_genome()`, `generate_candidate()`, `sandbox_test()`, `evaluate()`, `migrate()`, `rollback()` |
| `event_bus.rs` | `EventBus` | 中断控制器 | `publish()`, `subscribe()`, `request()`, `unsubscribe()` |
| `plugin.rs` | `Plugin` | 可加载模块 | `name()`, `init()`, `hooks()`, `shutdown()` |

#### 4.2.1 Subsystem 生命周期（`include/subsystem.rs:16-108`）

```rust
pub trait Subsystem: Send + Sync {
    fn name(&self) -> &str;
    async fn init(&mut self, ctx: &SubsystemContext) -> Result<()>;
    async fn health(&self) -> SubsystemHealth;
    async fn shutdown(&mut self) -> Result<()>;
    fn version(&self) -> Version;
    fn init_phase(&self) -> InitPhase { InitPhase::Subsystem }
}
```

启动阶段顺序：`Core(0) → Subsystem(1) → Service(2) → Late(3)`

`SubsystemContext` 包含共享基础设施引用：`CommunicationBus`（统一通信总线）、工作目录、配置。

#### 4.2.2 SelfFieldOps — 策略引擎（`include/self_field.rs:292-317`）

`review()` 方法是核心操作——每个意图都必须经过此方法。Veridict 枚举定义了六种结果：

```rust
pub enum Verdict {
    Allow,                                    // 直接允许
    AllowWithModification { modification },   // 允许但修改意图
    Deny { reason },                          // 拒绝
    RequireConfirmation { reason, risk_level }, // 需要用户确认
    SandboxFirst { reason },                  // 先沙箱测试
    Delay { reason, until },                  // 延迟执行
}
```

#### 4.2.3 BrainCoreOps — 认知计算（`include/brain.rs:275-290`）

产出 `Plan` → `PlanStep`（含回滚动作）→ `ExecutionResult` → `Reflection` → `LearnedRule`

支持两阶段推理（`include/brain.rs:211-258`）：`BehaviorAdjustment` 和 `EvolutionLogEntry` 支持从经验中学习并调整行为。

#### 4.2.4 BodyRuntime — 执行 HAL（`include/body.rs:73-90`）

```rust
pub trait BodyRuntime: Subsystem {
    async fn execute(&self, action: Action, ctx: &Context) -> Result<ActionResult>;
    fn capabilities(&self) -> &[Capability];
    async fn check(&self, action: &Action, ctx: &Context) -> Result<()>;
}
```

`Action` 结构体类似于系统调用（`body.rs:19-28`），包含名称、JSON 参数、沙箱要求、超时。

#### 4.2.5 MemoryBackend — 存储 VFS（`include/memory.rs:141-159`）

四种记忆类型（`MemoryType` 枚举，`memory.rs:17-25`）：
- **Episodic** — 情景：发生了什么，何时，结果
- **Semantic** — 语义：知识、概念、事实、文档
- **Procedural** — 程序：可复用技能、工作流、模式
- **SelfMemory** — 自我：身份变更、边界决策、关怀演化、变异历史

`MemoryEntry` 结构体（`memory.rs:29-49`）包含：UUID、内容（`Vec<u8>`）、标签、访问计数、重要性分数（0.0-1.0）、衰减率、关联记忆 ID 列表。

`MemoryQuery`（`memory.rs:58-75`）支持：文本搜索、语义向量搜索、时间范围过滤、标签过滤、重要性阈值。

### 4.3 types/ — 共享数据类型

| 文件 | 关键类型 | 用途 |
|------|---------|------|
| `message.rs` | `Message`, `ContentBlock`, `Role` | 统一消息协议（对齐 Anthropic SDK 格式） |
| `context.rs` | `Context`, `TraceState` | 请求生命周期上下文（类似 task_struct） |
| `llm_types.rs` | `ToolDefinition` | LLM 工具定义 |
| `tool.rs` | `Tool`, `ToolContext`, `ToolResult` | 工具执行接口 |
| `genome.rs` | `Genome` | 自我进化的基因组数据结构 |
| `objective.rs` | `Objective`, `ObjectiveStatus` | 目标任务跟踪 |
| `sandbox.rs` | `SandboxConfig`, `IsolationLevel`, `SandboxResult` | 沙箱配置与结果 |
| `capability.rs` | `Capability`, `CapabilitySet`, `PermissionLevel` | 能力与权限 |
| `permission.rs` | `PermissionMode`, `PermissionRule` | 权限规则引擎 |
| `hook.rs` / `hook_ext.rs` | `HookPoint`, `HookConfig`, `HookResult` | 钩子系统 |
| `vision.rs` | — | 图像/视觉类型 |
| `paths.rs` | — | 标准路径（配置、数据、日志） |
| `resource.rs` | `ManagedResource`, `ResourceState` | 资源管理 |
| `grounding.rs` | — | 物理接地类型 |
| `agent.rs` | `Pid` | Agent 进程标识符 |

#### ContentBlock 消息协议（`types/message.rs:9-36`）

对齐 Anthropic SDK 格式的统一消息块：
```rust
pub enum ContentBlock {
    Text { text: String },
    Thinking { text: String, signature: Option<String> },
    ToolUse { id: String, name: String, input: Value },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
    Image { source: ImageSource },
    System { text: String, priority: Priority },
}
```

#### Context — 请求上下文（`types/context.rs:18-37`）

类似 Linux `task_struct`，携带整个请求生命周期的所有状态：
- 唯一请求 ID（UUID v4）
- 会话 ID（跨请求持久化）
- 分布式追踪状态（trace_id, span_id, parent_span_id）
- 能力集（权限控制）
- 工作目录
- 可扩展元数据（HashMap）

### 4.4 events/ — 事件系统

| 文件 | 职责 |
|------|------|
| `event.rs` | 事件类型定义、优先级、Event trait、ConcreteEvent、EventHandler |
| `event_bridge.rs` | 将 Event/EventBus 适配到 Envelope 系统（迁移路径） |
| `event_log.rs` | 事件日志记录 |
| `evolution.rs` | 自进化事件类型 |
| `routing_policy.rs` | 事件路由策略 |
| `subscription.rs` | 订阅管理 |
| `ui_event.rs` | TUI 事件类型（ClientEvent, PlanUpdate, SubAgentHandle 等） |

事件类型枚举（`event.rs:15-74`）涵盖 30+ 种事件，分为 User-space、Environment、BodyRuntime、Memory、SelfField、BrainCore、MetaRuntime、Lifecycle、Runtime、Self-evolution、Energy 等类别。

### 4.5 ipc/ — 进程间通信

```
ipc/
├── bus/
│   ├── communication_bus.rs  # CommunicationBus — 统一通信入口
│   ├── in_process.rs         # InProcessTransport
│   ├── kernel_bus.rs         # KernelEventBus
│   ├── pubsub.rs             # PubSubProtocol
│   └── request_response.rs   # RequestResponseProtocol
├── backends/
│   ├── json_rpc.rs           # JSON-RPC 后端
│   ├── json_rpc_transport.rs # JSON-RPC 传输层
│   ├── io_uring.rs           # io_uring 后端 (feature-gated)
│   ├── io_uring_transport.rs
│   └── shared_mem.rs         # 共享内存后端
├── transport/                # 传输适配层
├── envelope.rs               # 消息信封 (Endpoint, Target, Payload, Pattern)
├── ipc_msg.rs                # IPC 消息 (IpcMessage, ForkDirective, Signal)
├── ipc_types.rs              # IPC 类型 (AgentId, IpcBackend, MessageType)
└── protocol.rs               # 协议抽象
```

**CommunicationBus** (`bus/communication_bus.rs:49-65`) 是外部接口，提供：
- 请求-响应（通过 `RequestResponseProtocol`）
- 发布-订阅（通过 `PubSubProtocol`）
- 模块邮箱
- 可选调试钩子（`DebugBusHook`）

### 4.6 kernel/ — 核心基础设施

| 文件 | 职责 |
|------|------|
| `debug.rs` | 调试事件、跟踪点（DebugEvent, DebugLevel, Tracepoint） |
| `debug_bus.rs` | 调试总线钩子、事件过滤器、性能计数器 |
| `error.rs` | AgentError、退避策略（指数退避）、降级链、错误分类 |
| `observable.rs` | 可观测子系统状态 |
| `registry.rs` | 通用注册表（RegistrationId, Registry） |

### 4.7 policy/ — 执行策略引擎

| 文件 | 职责 |
|------|------|
| `execpolicy.rs` | 策略引擎 — 网络规则、前缀规则、模式匹配、默认启发式 |
| `permission_authority.rs` | 权限机构 |
| `verifier.rs` | 结果验证器 |

---

## 5. cognit — 认知计算引擎

**路径**: `crates/cognit/src/`
**Cargo.toml**: 描述为 "Aletheon Brain core - reasoning, planning, and reflection"
**依赖**: `base`, tokio, parking_lot, serde, reqwest, rusqlite, dirs

### 5.1 模块结构

```
cognit/src/
├── lib.rs             # 公开 API 重导出
├── config/
│   └── mod.rs         # AppConfig（分层配置加载）
├── core/              # 核心认知组件
│   ├── brain_core_ops.rs    # BrainCoreOps trait 实现
│   ├── brain_core_subsystem.rs # Subsystem trait 实现
│   ├── reasoner.rs          # 推理器（Direct, ChainOfThought 策略）
│   ├── planner.rs           # 规划器（意图 → Plan）
│   ├── reflector.rs         # 反思器
│   ├── critic.rs            # 批判器
│   ├── learner.rs           # 学习器
│   ├── world_model.rs       # 世界模型
│   ├── awareness.rs         # 自我意识
│   ├── awareness_signal.rs  # 意识信号
│   ├── skill_extractor.rs   # 技能提取
│   ├── experience_summarizer.rs # 经验总结器
│   └── evolution_trigger.rs # 进化触发器
├── bridge/            # 桥接层
│   ├── dual_model.rs  # DualModelBridge — 双模型推理（规划器 + 执行器）
│   ├── inference.rs   # InferenceBridge — 推理路由
│   ├── learning.rs    # LearningBridge
│   └── llm.rs         # LlmBridge — LLM 调用封装
└── impl/
    ├── llm/
    │   ├── mod.rs          # LlmProvider trait + StopReason
    │   ├── anthropic.rs    # Anthropic Claude 适配器
    │   ├── openai_provider.rs # OpenAI 适配器
    │   ├── ollama.rs       # Ollama 本地模型适配器
    │   ├── provider.rs     # Provider 基础设施
    │   ├── provider_factory.rs # Provider 工厂
    │   ├── scheduler.rs    # 认知脉冲调度器
    │   └── pulse.rs        # LlmPulse — 定期认知脉冲
    ├── inference/
    │   ├── classifier.rs   # 任务分类器
    │   ├── router.rs       # 推断路由器
    │   └── provider_config.rs
    ├── learning/
    │   ├── outcome.rs      # 结果学习
    │   ├── pattern.rs      # 模式学习
    │   └── rule.rs         # 规则学习
    ├── event_handlers/
    │   └── tool_observer.rs # 工具观察器
    ├── grounding/
    │   └── vision.rs       # 视觉接地
    └── provider_registry.rs # Provider 注册表（从配置解析创建）
```

### 5.2 核心组件

#### BrainCore（`core/brain_core_ops.rs`）

BrainCore 结构体（在 `core/mod.rs` 中定义）实现 `BrainCoreOps` + `Subsystem`：

**think() 推理流程**（`brain_core_ops.rs:25-102`）：
1. 如果配置了双模型桥且任务复杂度为 Complex → **两阶段推理**：
   - Pass 1: 规划器模型分析任务
   - Pass 2: 执行器模型生成最终 Plan（受规划器分析引导）
   - 验证执行器的 Plan 覆盖了规划器的分析
2. 否则如果 LLM 桥可用 → 单模型推理
3. 否则 → 基于模板的 Reasoner 兜底

#### DualModelBridge（`bridge/dual_model.rs`）

支持分离的规划器/执行器模型对：
- `TaskComplexity` 枚举：Simple / Complex
- `DualModelConfig` 配置：两个独立的 `LlmBridge` 实例
- 仅在复杂任务时启动两阶段流程

#### LlmProvider（`impl/llm/mod.rs`）

统一的 LLM 适配器 trait，支持：
- Anthropic Claude（原生 API）
- OpenAI（兼容 API）
- Ollama（本地部署）
- Mock（测试用）

#### ProviderRegistry（`impl/provider_registry.rs`）

从 `AppConfig` 中解析 Provider 配置，按需创建 LLM 适配器实例。

### 5.3 配置系统（`config/mod.rs`）

`AppConfig` 支持分层加载：
1. 内置默认值
2. `~/.aletheon/config.toml`（用户级）
3. `/etc/agentd/config.toml`（系统级）
4. 命令行显式指定路径

---

## 6. corpus — 工具执行体

**路径**: `crates/corpus/src/`
**Cargo.toml**: 描述为 "Core execution body — the minimal runtime for tool execution"
**依赖**: `base`, nix (optional), x11rb (optional), atspi (optional), tesseract (optional), zbus (optional), reqwest, tree-sitter

### 6.1 模块结构

```
corpus/src/
├── lib.rs              # 重导出 AletheonBodyRuntime
├── core/
│   ├── mod.rs
│   └── conversions.rs  # 类型转换
├── drivers/            # 平台驱动层
│   ├── driver/
│   │   ├── factory.rs   # DriverFactory
│   │   ├── types.rs     # Driver trait
│   │   ├── input/       # uinput 输入驱动
│   │   ├── display/     # X11 显示驱动 (clipboard, window, drm)
│   │   ├── a11y/        # AT-SPI 无障碍驱动
│   │   ├── ocr/         # Tesseract OCR 驱动
│   │   ├── io/          # I/O 驱动
│   │   ├── proc/        # 进程驱动
│   │   └── sandbox_driver/
│   └── platform/
│       ├── linux.rs     # Linux 平台适配
│       ├── android.rs   # Android 平台适配
│       ├── adapter.rs   # 平台适配器
│       ├── boot.rs      # 启动逻辑
│       └── awareness/   # 跨平台感知
├── security/           # 安全子系统
│   ├── sandbox/
│   │   ├── bubblewrap.rs    # bubblewrap 沙箱实现
│   │   ├── bwrap_builder.rs # bubblewrap 参数构建器
│   │   ├── container.rs     # 容器沙箱
│   │   ├── executor.rs      # SandboxPreference + 执行器
│   │   ├── backend.rs       # 沙箱后端 trait
│   │   ├── policy.rs        # 沙箱策略
│   │   ├── profile.rs       # 安全配置
│   │   ├── process.rs       # 进程管理
│   │   ├── noop.rs          # 无操作沙箱
│   │   ├── env.rs           # 环境变量处理
│   │   └── glob_scanner.rs  # Glob 文件扫描
│   └── security/
│       ├── runner.rs        # ToolRunnerWithGuard — 工具执行守卫
│       ├── approval.rs      # ApprovalGate + TerminalApprovalGate
│       ├── audit.rs         # AuditLogger (JSONL 审计日志)
│       ├── circuit_breaker.rs # 断路器
│       ├── loop_detector.rs # 循环检测
│       ├── exec_policy.rs   # 执行策略
│       ├── output_guardrail.rs # 输出护栏
│       ├── permission_rules.rs # 权限规则
│       ├── policy.rs        # 安全策略
│       ├── risk_classifier.rs  # 风险分类器
│       └── socket_approval.rs  # Socket 审批
├── tools/              # 工具系统
│   ├── tools/
│   │   ├── mod.rs          # 工具注册表 + 24 个内置工具
│   │   ├── bash_exec.rs    # Shell 命令执行
│   │   ├── file_read.rs    # 文件读取
│   │   ├── file_write.rs   # 文件写入
│   │   ├── file_search.rs  # 文件搜索
│   │   ├── grep.rs         # 内容搜索
│   │   ├── glob.rs         # Glob 模式匹配
│   │   ├── web_fetch.rs    # HTTP 请求
│   │   ├── web_search.rs   # 网络搜索
│   │   ├── agent_tool.rs   # 子 Agent 调用
│   │   ├── apply_patch.rs  # 补丁应用
│   │   ├── code_graph.rs   # 代码图分析
│   │   ├── script_tool.rs  # 脚本执行
│   │   ├── task_tools.rs   # 任务管理工具
│   │   ├── process_list.rs # 进程列表
│   │   ├── system_status.rs # 系统状态
│   │   ├── ebpf_compile.rs # eBPF 编译
│   │   ├── kernel_build.rs # 内核构建
│   │   ├── module_build.rs # 模块构建
│   │   ├── module_load.rs  # 模块加载
│   │   ├── registry.rs     # ToolRegistry
│   │   ├── toolset.rs      # ToolsetRegistry
│   │   ├── executor.rs     # 工具执行器
│   │   ├── exposure.rs     # 工具暴露控制
│   │   └── output/         # 输出捕获/截断/持久化/修剪
│   ├── mcp/
│   │   ├── manager.rs      # MCP 服务器管理器
│   │   ├── client.rs       # MCP 客户端
│   │   ├── config.rs       # MCP 配置
│   │   ├── transport.rs    # MCP 传输层
│   │   ├── wrapper.rs      # MCP 工具封装
│   │   ├── auth.rs         # MCP 认证
│   │   └── mod.rs
│   ├── skills/
│   │   ├── loader.rs       # 技能加载器
│   │   └── markdown_skill.rs # Markdown 技能定义
│   └── hooks/
│       ├── types.rs        # 钩子类型
│       ├── registry.rs     # 钩子注册表
│       └── runner.rs       # 钩子执行器
└── testing/
    └── mock_sandbox.rs     # Mock 沙箱（测试用）
```

### 6.2 内置工具清单（24 个）

| 工具名 | 文件 | 功能 |
|--------|------|------|
| bash_exec | `bash_exec.rs` | Shell 命令执行 |
| file_read | `file_read.rs` | 文件读取 |
| file_write | `file_write.rs` | 文件写入 |
| file_search | `file_search.rs` | 文件名搜索 |
| grep | `grep.rs` | 文件内容搜索 |
| glob | `glob.rs` | Glob 模式匹配 |
| web_fetch | `web_fetch.rs` | HTTP 请求 |
| web_search | `web_search.rs` | 网络搜索 |
| agent_tool | `agent_tool.rs` | 子 Agent 调用 |
| apply_patch | `apply_patch.rs` | 补丁应用 |
| code_graph | `code_graph.rs` | 代码语法树分析 |
| script_tool | `script_tool.rs` | 脚本执行 |
| task_tools | `task_tools.rs` | 任务创建/更新/查询 |
| process_list | `process_list.rs` | 系统进程列表 |
| system_status | `system_status.rs` | 系统资源状态 |
| ebpf_compile | `ebpf_compile.rs` | eBPF 程序编译 |
| kernel_build | `kernel_build.rs` | Linux 内核构建 |
| module_build | `module_build.rs` | 内核模块构建 |
| module_load | `module_load.rs` | 内核模块加载 |
| search | `search/` | 工具搜索 + 子 Agent 搜索 |

### 6.3 安全模型

#### 沙箱系统（`security/sandbox/`）

- **bubblewrap**：Linux bubblewrap 容器隔离（`bubblewrap.rs` + `bwrap_builder.rs`）
- **Container**：Docker/Podman 容器沙箱（`container.rs`）
- **Noop**：无沙箱直接执行（`noop.rs`）
- **SandboxPreference**（`executor.rs`）：auto / require / forbid

#### ToolRunnerWithGuard（`security/security/runner.rs`）

工具执行守卫链：
1. ApprovalGate（审批门）→ 需要人工确认
2. 沙箱检查 → 隔离执行
3. Hook 前置/后置处理
4. AuditLogger → JSONL 审计日志
5. CircuitBreaker → 异常熔断

### 6.4 平台驱动特性门控

```toml
# corpus/Cargo.toml features
dbus = ["zbus"]          # D-Bus 系统总线
input = ["nix"]           # uinput 输入注入
display = ["nix", "x11rb"] # X11 显示 + 剪贴板
a11y = ["atspi"]          # 无障碍 (AT-SPI)
ocr-tesseract = ["ocr", "tesseract"] # OCR
sandbox-primitives = ["nix"] # seccomp/landlock
```

---

## 7. dasein — 自我策略引擎

**路径**: `crates/dasein/src/`
**Cargo.toml**: 描述为 "Aletheon Self-evolution - reflection, behavior evolution, and genome generation"

### 7.1 哲学基础

dasein crate 实现了 Aletheon 的"此在"（Dasein）层，借鉴海德格尔现象学概念。核心原则（`lib.rs:8-11`）：

> **Everything is interpreted by the Self.** SelfField 不是一个模块——它是每个事件、意图、记忆和动作都必须经过的场。

### 7.2 模块结构

```
dasein/src/
├── lib.rs             # SelfField struct + SelfFieldConfig
├── core/              # 核心策略层
│   ├── boundary.rs    # 边界规则引擎（模式匹配，快速门控）
│   ├── identity.rs    # 身份模型 + 变更历史
│   ├── care.rs        # 关怀权重计算
│   ├── narrative.rs   # 决策叙事日志（环形缓冲区）
│   ├── conflict.rs    # 多源冲突仲裁
│   ├── attention.rs   # 注意力追踪（优先级 + 衰减）
│   ├── continuity.rs  # 身份连续性（谱系记录）
│   ├── mutation.rs    # 变异请求追踪与审批
│   ├── awareness_growth.rs    # 意识增长分析
│   ├── evolution_validator.rs # 进化验证器
│   └── store.rs       # 持久化存储
├── dasein/            # 现象学模块
│   ├── self_model.rs  # 自我模型
│   ├── care_structure.rs # 关怀结构
│   ├── temporality.rs # 时间性（过去-现在-未来）
│   ├── bewandtnis.rs  # 因缘（工具关联网络）
│   ├── sorge.rs       # 操劳（Sorge）
│   ├── negativity.rs  # 否定性（错误/失败分析）
│   ├── context_injection.rs # 上下文注入
│   ├── event_bridge.rs     # 事件桥接
│   ├── persistence.rs      # 持久化
│   └── types.rs            # Dasein 类型
├── bridge/            # 策略桥接
│   ├── policy.rs      # PolicyBridge (PolicyEngine 适配)
│   ├── hook.rs        # HookBridge (pre/post-tool hooks)
│   └── loop_detector.rs # 循环检测桥接
└── impl/
    ├── perception/         # 感知子系统
    │   ├── manager.rs      # PerceptionManager
    │   ├── aggregator.rs   # 感知聚合器
    │   ├── event.rs        # 感知事件
    │   ├── bridge.rs       # 感知桥接
    │   ├── sources/
    │   │   ├── proc_source.rs       # /proc 文件系统监控
    │   │   ├── journald_source.rs   # systemd journal 监控
    │   │   ├── inotify_source.rs    # inotify 文件监控
    │   │   ├── ebpf_source.rs       # eBPF 内核探测
    │   │   └── bottleneck_detector.rs # 瓶颈检测
    │   └── fuse/            # FUSE 文件系统代理
    ├── mutation/
    │   ├── approver.rs      # 变异审批（安全检查）
    │   └── mod.rs
    ├── security/
    │   ├── audit.rs         # 安全审计
    │   ├── circuit_breaker.rs  # 断路器
    │   ├── loop_detector.rs # 循环检测
    │   ├── output_guardrail.rs  # 输出护栏
    │   ├── policy.rs        # 安全策略
    │   ├── risk_classifier.rs   # 风险分类
    │   ├── runner.rs        # 安全执行器
    │   ├── rate_limiting/   # 速率限制
    │   │   ├── backpressure.rs  # 背压控制
    │   │   ├── flood_protector.rs # 洪水防护
    │   │   ├── token_limiter.rs   # Token 限制器
    │   │   └── tool_limiter.rs    # 工具调用限制器
    │   ├── sandbox/
    │   │   └── writable_root.rs   # 可写根文件系统沙箱
    │   ├── rollback/        # 回滚机制
    │   │   └── types.rs
    │   └── self_protection/ # 自我保护
    │       ├── emergency_killswitch.rs # 紧急停止开关
    │       ├── input_sanitizer.rs      # 输入清理
    │       ├── integrity_monitor.rs    # 完整性监控
    │       └── resource_governor.rs    # 资源调控
    ├── resilience/
    │   ├── guardian.rs      # 守护者（异常恢复）
    │   ├── safe_mode.rs     # 安全模式
    │   └── watchdog.rs      # 看门狗
    ├── hook/
    │   ├── dispatcher.rs    # 钩子分发器
    │   ├── config.rs        # 钩子配置
    │   └── types.rs         # 钩子类型
    └── llm_bridge.rs        # LLM 桥接
```

### 7.3 review() 管道（`lib.rs:40-57`）

SelfField 的 `review()` 方法按以下流程处理每个 Intent：

```
Intent 到达
  → HookBridge.fire_pre_tool()   [pre-tool hooks 可以阻止/修改]
     → Block? return Verdict::Deny
  → PolicyBridge.check()         [PolicyEngine 策略检查]
     → Deny? return Verdict::Deny
     → RequireApproval? return Verdict::RequireConfirmation
  → BoundaryLayer.check()        [模式匹配, 类似 SELinux]
     → Deny? return Verdict::Deny
     → Sandbox? return Verdict::SandboxFirst
     → Confirm? return Verdict::RequireConfirmation
  → CareLayer.score_action()     [加权关怀评分]
  → Permission check             [Context.permissions vs 所需等级]
  → NarrativeLayer.record()      [始终记录以保证连续性]
  → return Verdict::Allow
```

### 7.4 8 层内部架构

1. **BoundaryLayer** — 模式匹配规则引擎（快速门控）
2. **IdentityLayer** — 当前自我模型 + 变异历史
3. **CareLayer** — 加权关怀评分，影响行为选择
4. **NarrativeLayer** — 环形缓冲区决策日志
5. **ConflictLayer** — 多源仲裁（User/Brain/Body/Memory/Self 冲突）
6. **AttentionLayer** — 焦点追踪（优先级 + 衰减）
7. **ContinuityLayer** — 身份连续性谱系记录
8. **MutationLayer** — 变异请求追踪与审批

### 7.5 感知子系统

感知子系统收集环境信息：

| 数据源 | 实现 | 用途 |
|--------|------|------|
| /proc 文件系统 | `proc_source.rs` | CPU/内存/进程指标 |
| systemd journal | `journald_source.rs` | 系统日志监控 |
| inotify | `inotify_source.rs` | 文件系统变更 |
| eBPF | `ebpf_source.rs` | 内核事件探测 |
| FUSE | `fuse/` | 文件系统代理（feature-gated） |

---

## 8. runtime — 运行时与编排

**路径**: `crates/runtime/src/`
**Cargo.toml**: 描述为 "Aletheon Runtime - core agent runtime and orchestration"

runtime 是**集成层**，依赖所有其他 crate，包含 ~100 个源文件。

### 8.1 模块结构

```
runtime/src/
├── lib.rs                # 公开 API 重导出
├── core/                 # 核心编排逻辑
│   ├── orchestrator.rs       # AletheonRuntime — 顶层运行时
│   ├── react_loop/           # ReAct (Reason + Act) 迭代循环
│   │   ├── mod.rs            # ReActLoop 结构体
│   │   ├── step.rs           # 单个推理步骤
│   │   ├── tool_exec.rs      # 工具执行
│   │   ├── tool_budget.rs    # 工具调用预算
│   │   ├── circuit_breaker.rs # 断路器
│   │   ├── goal_tracker.rs   # 目标追踪器
│   │   ├── reflection.rs     # 定期反思引擎
│   │   ├── awareness.rs      # 自我意识
│   │   ├── batching.rs       # 工具调用分批
│   │   ├── metrics.rs        # 轮次指标
│   │   └── message_compose.rs # 消息组合
│   ├── runtime_core.rs       # RuntimeCore — 守护进程启动器
│   ├── controller.rs         # 控制器
│   ├── session.rs            # 会话管理
│   │   ├── gateway.rs        # SessionGateway
│   │   ├── session_state.rs  # 会话状态
│   │   ├── approval_flow.rs  # 审批流
│   │   ├── param_registry.rs # 参数注册表
│   │   ├── snapshot.rs       # 快照
│   │   ├── subsystem_query.rs # 子系统查询
│   │   └── turn_context.rs   # 轮次上下文
│   ├── event_sink.rs         # 事件汇聚
│   ├── mode_router.rs        # 模式路由器
│   ├── interrupt.rs          # 中断标志
│   ├── permission_manager.rs # 权限管理器
│   ├── verdict_handler.rs    # Verdict 处理器（默认）
│   ├── behavior_paths.rs     # 行为路径路由器
│   ├── evolution_coordinator.rs # 进化协调器
│   ├── sub_agent.rs          # 子 Agent 生成器
│   ├── storm_breaker.rs      # StormBreaker（紧急停止）
│   └── config/               # 运行时配置
│       ├── mod.rs            # AppConfig + DaemonConfig + ...
│       ├── agent.rs          # AgentConfig
│       ├── genome.rs         # GenomeConfig
│       ├── infra.rs          # 基础设施配置
│       └── provider.rs       # ProviderConfig
├── host/                 # 部署形态抽象
│   ├── mod.rs                # RuntimeHost trait + DaemonHost
│   ├── systemd.rs            # SystemdHost
│   └── container.rs          # ContainerHost
├── impl/                 # 具体实现
│   ├── daemon/
│   │   ├── server.rs         # UnixServer — Unix socket 事件循环
│   │   ├── mod.rs             # DaemonConfig
│   │   ├── session_manager.rs # SessionManager
│   │   ├── model_router.rs    # ModelRouter
│   │   ├── prefix_builder.rs  # 前缀构建器
│   │   ├── mcp_embedded.rs    # MCP 嵌入式服务器
│   │   ├── cache_shape.rs     # 缓存形状
│   │   ├── debug_handler.rs   # 调试处理器
│   │   └── handler/
│   │       ├── chat.rs        # 聊天处理
│   │       ├── connection.rs  # 连接管理
│   │       ├── format.rs      # 格式化
│   │       ├── init.rs        # 初始化
│   │       ├── session_routing.rs # 会话路由
│   │       ├── turn_handler.rs # 轮次处理器
│   │       └── rpc/
│   │           ├── rpc_admin.rs     # 管理 RPC
│   │           ├── rpc_goal.rs      # 目标 RPC
│   │           ├── rpc_health.rs    # 健康检查 RPC
│   │           ├── rpc_memory.rs    # 记忆 RPC
│   │           ├── rpc_reflection.rs # 反思 RPC
│   │           ├── rpc_session.rs   # 会话 RPC
│   │           └── rpc_workflow.rs  # 工作流 RPC
│   ├── agent/
│   │   ├── process.rs    # Agent 进程管理
│   │   ├── harness.rs    # Agent 测试 harness
│   │   ├── budget.rs     # 预算管理
│   │   └── fork.rs       # Agent fork 逻辑
│   ├── engine/
│   │   ├── config.rs
│   │   ├── modules/
│   │   │   ├── self_field_module.rs  # SelfField 模块
│   │   │   ├── body_module.rs        # Body 模块
│   │   │   ├── memory_module.rs      # Memory 模块
│   │   │   └── perception_module.rs  # Perception 模块
│   ├── orchestration/
│   │   ├── agent.rs       # 多 Agent 编排
│   │   ├── delegate.rs    # 委托
│   │   ├── selector.rs    # Agent 选择器
│   │   ├── registry.rs    # AgentRegistry
│   │   ├── handoff.rs     # 交接
│   │   ├── termination.rs # 终止策略
│   │   ├── budget.rs      # 预算
│   │   ├── store.rs       # 持久化存储
│   │   ├── config_agent.rs # 配置 Agent
│   │   ├── builtin/       # 内置 Agent
│   │   │   ├── code_agent.rs  # 代码 Agent
│   │   │   ├── fs_agent.rs    # 文件系统 Agent
│   │   │   └── net_agent.rs   # 网络 Agent
│   │   └── digraph/       # 有向图（任务依赖 DAG）
│   ├── memory/
│   │   ├── core_memory.rs      # CoreMemory
│   │   ├── recall_memory.rs    # RecallMemory
│   │   ├── archival_memory.rs  # 归档记忆
│   │   ├── auto_memory.rs      # 自动记忆
│   │   ├── scope.rs            # 记忆作用域
│   │   ├── budget.rs           # 记忆预算
│   │   ├── compaction.rs       # 记忆压缩
│   │   ├── vector_store.rs     # 向量存储
│   │   ├── core_memory_store.rs # CoreMemory 持久化
│   │   ├── memory_pipeline.rs   # 记忆管道
│   │   ├── tools.rs             # 记忆工具
│   │   ├── fact_store/          # 事实存储（索引 + 查询）
│   │   ├── compressor/          # 上下文压缩器（尾压缩 + 模板压缩）
│   │   └── pipeline/            # 记忆管道阶段
│   ├── hooks/
│   │   ├── registry.rs      # HookRegistry
│   │   ├── loader.rs        # Hook 加载器
│   │   ├── lifecycle/
│   │   │   ├── session_distiller.rs # 会话蒸馏器
│   │   │   └── recall_inject.rs     # 记忆注入
│   │   └── builtin/
│   │       └── audit_hook.rs # 审计钩子
│   ├── skills/
│   │   ├── loader.rs        # 技能加载器
│   │   ├── manifest.rs      # 技能清单
│   │   ├── inject.rs        # 技能注入
│   │   ├── keyword_matcher.rs # 关键词匹配器
│   │   └── plugin.rs        # 技能插件
│   ├── kernel/
│   │   ├── kernel.rs        # 内核
│   │   ├── supervisor.rs    # 监督者
│   │   ├── ipc.rs           # IPC 内核
│   │   └── global_pool.rs   # 全局线程池
│   ├── session/
│   │   ├── store.rs         # SQLite 会话存储
│   │   ├── journal.rs       # 会话日志
│   │   └── observability/   # 可观测性
│   │       ├── metrics.rs         # 指标
│   │       ├── fragment.rs        # 片段
│   │       ├── publisher.rs       # 发布者
│   │       ├── reasoning_logger.rs # 推理日志
│   │       └── tool_tracker.rs     # 工具追踪
│   ├── goal/
│   │   ├── mod.rs
│   │   └── store.rs         # ObjectiveStore (目标持久化)
│   └── coordinator.rs       # 协调器
├── tools/
│   ├── mod.rs
│   └── self_observe.rs      # 自我观察工具
└── bridge/
    └── mod.rs
```

### 8.2 AletheonRuntime（`core/orchestrator.rs:28-37`）

顶层编排器结构体，将 Engine god-object 分解为 6 层：

```rust
pub struct AletheonRuntime {
    config: RuntimeConfig,
    react_loop: ReActLoop,         // ReAct 迭代循环
    evolution: Option<EvolutionCoordinator>, // 可选的进化协调器
    genome_config: GenomeConfig,   // 基因组配置
    verdict_handler: Arc<dyn VerdictHandler>, // Verdict 处理器
    mode_router: ModeRouter,       // 模式路由器
    interrupt_flag: InterruptFlag, // 中断标志
    sub_agent_spawner: SubAgentSpawner, // 子 Agent 生成器
}
```

关键方法：
- `with_evolution(config)` — 附加进化协调器
- `post_evolution(...)` — 每轮后执行进化分析
- `with_genome_config(config)` — 设置基因组配置

### 8.3 ReActLoop（`core/react_loop/mod.rs:36-71`）

Reason + Act 核心认知循环：

```rust
pub struct ReActLoop {
    config: RuntimeConfig,
    iteration: usize,                   // 当前迭代数
    messages: Vec<Message>,             // 对话消息
    compressor: AdvancedCompressor,     // 上下文压缩器
    system_prompt: String,              // 不可变系统提示
    plan_mode: bool,                    // 计划模式标志
    pending_memory: Vec<String>,        // 待注入记忆
    signals: Vec<AwarenessSignal>,      // 本轮收集的意识信号
    recent_tools: Vec<String>,          // 最近使用的工具（目标转移检测）
    consecutive_errors: usize,          // 连续错误数（僵局检测）
    interrupt_flag: Option<InterruptFlag>, // 外部中断
    tool_budget: ToolBudget,            // 工具调用预算
    circuit_breaker: CircuitBreaker,    // 循环断路器
    goal_tracker: GoalTracker,          // 目标与子目标追踪
    reflection_engine: ReflectionEngine, // 定期反思引擎
    verifier: Option<Arc<dyn Verifier>>, // 可选结果验证器
    dasein_ctx_provider: Option<...>,    // 可选 Dasein 上下文提供者
}
```

**上下文压缩策略**（`mod.rs:74-100`）：
- Tail token 预算按上下文窗口比例缩放（默认 ~12.5%）
- 达到阈值时使用 `AdvancedCompressor` 压缩早期消息
- 保留头部（系统提示）和尾部（最近交互）

### 8.4 RuntimeHost（`host/mod.rs:49-59`）

部署形态抽象 trait：

```rust
pub trait RuntimeHost {
    async fn init(&mut self) -> Result<()>;    // 准备资源
    async fn serve(self: Box<Self>) -> Result<()>; // 运行事件循环
    async fn shutdown(&mut self) -> Result<()>; // 释放资源
}
```

三种宿主实现：
- **DaemonHost** — 前台 Unix socket 守护进程
- **SystemdHost** — systemd 服务集成（NOTIFY_SOCKET 检测）
- **ContainerHost** — Docker/Podman 容器集成

### 8.5 守护进程服务器（`impl/daemon/server.rs`）

Unix socket 守护进程启动流程（`host/mod.rs:98-230`）：

1. PID 文件写入 `/tmp/aletheon/aletheond.pid`
2. 加载 `.env`（优先级：`~/.aletheon/.env` → `/etc/aletheon/.env` → `./.env`）
3. `RuntimeCore::bootstrap()` — 初始化配置、Provider、EventBus、LlmPulse、感知、RequestHandler
4. 创建数据目录
5. 启动 MCP 嵌入式服务器（独立的 Unix socket）
6. 启动主 Unix socket 监听循环
7. Ctrl+C 优雅关闭 → 取消当前 turn → 停止 LlmPulse → 清理 PID 文件

Socket 安全（`server.rs:43-48`）：
- 模式 `0o660`（仅 owner + group 可读写）
- 连接时验证 peer UID/GID 凭证

### 8.6 会话管理（`impl/daemon/session_manager.rs` + `impl/daemon/handler/`）

RequestHandler 是守护进程的 JSON-RPC 调度器，管理：
- 聊天处理（`handler/chat.rs`）
- 会话路由（`handler/session_routing.rs`）
- 轮次处理（`handler/turn_handler.rs`）
- RPC 端点（7 个 RPC 处理器：admin, goal, health, memory, reflection, session, workflow）
- 模型路由（`model_router.rs`）

### 8.7 多 Agent 编排（`impl/orchestration/`）

| 组件 | 文件 | 职责 |
|------|------|------|
| AgentRegistry | `registry.rs` | Agent 注册与查找 |
| AgentSelector | `selector.rs` | 基于任务特征的 Agent 选择 |
| DelegateOrchestrator | `delegate.rs` | 委托编排 |
| HandoffProtocol | `handoff.rs` | Agent 间交接协议 |
| TerminationPolicy | `termination.rs` | 终止策略 |
| TaskDigraph | `digraph/` | 任务有向无环图（DAG） |

内置 Agent 类型（`builtin/`）：
- `code_agent` — 代码生成/修改
- `fs_agent` — 文件系统操作
- `net_agent` — 网络请求

### 8.8 守护进程 RequestHandler 详解（`impl/daemon/handler/mod.rs:81-179`）

RequestHandler 是守护进程的 JSON-RPC 调度器，包含 90+ 字段，按职责分组：

**LLM & 路由**：
- `llm: Arc<dyn LlmProvider>` — 主 LLM 实例
- `model_router: Arc<ModelRouter>` — 按 TaskType 动态选择模型

**会话管理**：
- `sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>` — 多会话注册表
- `default_session_id` — 默认会话
- `session_created_at` — 会话时间戳

**记忆注入**：
- `recall_memory: Arc<Mutex<RecallMemory>>` — 持久化消息存储
- `core_memory: Arc<Mutex<CoreMemory>>` — 身份/人格记忆
- `memory_queue: Arc<Mutex<Vec<String>>>` — 中期记忆注入（保持前缀缓存稳定）
- `fact_store: Arc<Mutex<FactStore>>` — SQLite FTS5 事实数据库
- `auto_memory: Arc<Mutex<AutoMemory>>` — 每轮自动事实提取（使用廉价 LLM）

**安全/沙箱**：
- `tool_runner: Arc<Mutex<ToolRunnerWithGuard>>` — 策略/审批/沙箱/审计管道
- `approval_rx`, `pending_approvals`, `notify_tx` — socket 审批门
- `session_approvals: Arc<Mutex<HashMap<String, bool>>>` — 每会话"始终批准"缓存

**认知/自我**：
- `self_field: Arc<Mutex<SelfField>>` — 策略引擎
- `reflector: Reflector` — 对话反思器
- `episodic_memory: Arc<Mutex<EpisodicMemory>>` — 反思/进化日志
- `pipeline: Arc<MorphogenesisPipeline<DefaultMetaRuntime>>` — 轮后进化

**可观测性**：
- `storm_breaker: Arc<Mutex<StormBreaker>>` — 循环检测器
- `debug_handler: Arc<DebugHandler>` — 17 个 debug.* JSON-RPC 方法
- `debug_perf: Arc<PerfCounter>` — token/工具调用计数器

### 8.9 RPC 方法调度（`impl/daemon/handler/rpc.rs:26-87`）

| 方法前缀 | 处理器文件 | 功能 |
|----------|-----------|------|
| `chat` | `chat.rs` | 主聊天轮次管道 |
| `clear`, `sessions`, `resume`, `compact`, `new_session`, `load_recent` | `rpc_session.rs` | 会话管理 |
| `status`, `health` | `rpc_health.rs` | 健康检查 |
| `daemon.shutdown`, `reload_skills`, `approval_response`, `interrupt`, `mode_switch`, `model_list`, `model_switch`, `tools/list`, `hooks_list`, `sub_agents` | `rpc_admin.rs` | 管理操作 |
| `reflect`, `reflect_now`, `genome`, `evolution` | `rpc_reflection.rs` | 反思与进化 |
| `memory.add/list/search/show/forget/pin/unpin` | `rpc_memory.rs` | 事实存储 CRUD |
| `workflow.save/load/list/delete/run` | `rpc_workflow.rs` | 工作流持久化 |
| `goal.set/show/status/resume` | `rpc_goal.rs` | 目标存储 CRUD |
| `debug.*` | `debug_handler.rs` | 跟踪、性能、bags、拓扑 |

### 8.10 前缀缓存系统

守护进程实现了精细的 LLM 提示缓存优化：

**PrefixBuilder** (`prefix_builder.rs`)：
- 系统提示前缀在启动时构建一次，会话期间不变
- 结构：`基础提示 → 技能索引 → 核心记忆`
- `build_with_dasein()` 方法在末尾追加存在主义状态
- `diff_reason()` 比较两个前缀并解释缓存未命中原因

**CacheShape** (`cache_shape.rs`)：
- 跟踪 4 个哈希值：`system_hash`, `tools_hash`, `prefix_hash`, `rewrite_version`
- `compare()` 返回 `Hit` 或 `Miss { reasons }`（SystemChanged, ToolsChanged, Compacted）
- `CacheStats` 跟踪命中/未命中 token 计数

**记忆注入策略** (`handler/mod.rs:288-298`)：
- 记忆变更不修改系统提示前缀
- 而是通过用户消息中的 `<memory-update>` XML 块注入
- 这最大化了 DeepSeek/Mimo 等提供商的 LLM 提示缓存命中率

### 8.11 连接管理与安全

**Unix socket 安全** (`server.rs:132-160`)：
- Socket 权限 `0o660`（仅 owner + group）
- 连接时验证 peer UID/GID：
  - root 用户允许
  - daemon 所有者 UID 允许
  - aletheon group 成员允许
- 每个连接独立通知通道（`per-connection notify_tx`）
- 后台任务处理长时间运行的处理器
- 多路复用响应（审批请求、流式事件、调试事件）

### 8.12 内存实现（`impl/memory/`）

| 组件 | 文件 | 职责 |
|------|------|------|
| CoreMemory | `core_memory.rs` | 核心工作内存（会话级，身份/人格） |
| RecallMemory | `recall_memory.rs` | 持久化消息存储（SQLite journal） |
| ArchivalMemory | `archival_memory.rs` | 长期归档 |
| AutoMemory | `auto_memory.rs` | 每轮自动事实提取（廉价 LLM） |
| CompactionEngine | `compaction.rs` | 记忆压缩/去重 |
| FactStore | `fact_store/` | FTS5 全文搜索事实存储 |
| MemoryPipeline | `memory_pipeline.rs` + `pipeline/` | 多阶段记忆处理（Phase1 → Phase2 → StateDB） |
| AdvancedCompressor | `compressor/` | 上下文压缩（TailCompressor + TemplateCompressor） |
| VectorStore | `vector_store.rs` | 向量嵌入存储 |
| ScopeManager | `scope.rs` | 记忆作用域管理（Session/Global 级别） |

### 8.13 子 Agent 系统（`impl/orchestration/builtin/`）

内置子 Agent 类型定义在 `agents/` 目录（`.md` + `.toml` 配对）：

| Agent | 配置 | 能力 |
|-------|------|------|
| `code-agent` | `agents/code-agent.toml` | 代码分析、生成、修改、搜索 |
| `fs-agent` | `agents/fs-agent.toml` | 文件系统操作（读/写/搜索/glob） |
| `net-agent` | `agents/net-agent.toml` | 网络请求（HTTP/API） |

`AgentLoader` (`impl/agent_loader/`) 负责从文件系统加载 Agent 定义并注入工具能力。

---

## 9. interact — 用户交互层

**路径**: `crates/interact/src/`
**Cargo.toml**: 描述为 "User interaction: TUI, CLI, and ACIX"
**依赖**: `base`, `corpus` (features: input, display, a11y, ocr), ratatui, crossterm, pulldown-cmark, syntect, clap

### 9.1 模块结构

```
interact/src/
├── lib.rs
├── tui/                    # 终端 UI
│   ├── mod.rs
│   ├── cli.rs              # CLI 入口（TUI 启动 + 单消息模式）
│   ├── app/
│   │   ├── mod.rs          # App 主结构
│   │   ├── lifecycle.rs    # 生命周期管理
│   │   ├── key_handler.rs  # 键盘事件处理
│   │   └── submit.rs       # 提交处理
│   ├── state.rs            # UI 状态管理
│   ├── chat.rs             # 聊天视图
│   ├── input.rs            # 输入处理
│   ├── response.rs         # 响应渲染
│   ├── streaming.rs        # 流式输出
│   ├── markdown.rs         # Markdown 渲染（pulldown-cmark + syntect 语法高亮）
│   ├── completion.rs       # 自动补全
│   ├── command.rs          # 命令解析
│   ├── status.rs           # 状态栏
│   ├── debug.rs            # 调试视图
│   ├── goal.rs             # 目标视图
│   ├── skill.rs            # 技能视图
│   ├── workflow.rs         # 工作流视图
│   ├── plan_view.rs        # 计划视图
│   ├── subagent_view.rs    # 子 Agent 视图
│   ├── awareness.rs        # 意识状态视图
│   ├── computer.rs         # 计算机使用视图
│   ├── pager.rs            # 分页器
│   ├── approval_dialog.rs  # 审批对话框
│   ├── help_overlay.rs     # 帮助覆盖层
│   ├── history_search.rs   # 历史搜索
│   ├── rpc_client.rs       # RPC 客户端（与守护进程通信）
│   ├── term_compat.rs      # 终端兼容层
│   ├── render/             # 渲染引擎
│   │   ├── mod.rs
│   │   ├── draw.rs         # 绘制逻辑
│   │   ├── header.rs       # 头部渲染
│   │   ├── input_line.rs   # 输入行渲染
│   │   └── renderable.rs   # Renderable trait
│   └── test_infra.rs       # TUI 测试基础设施（tmux 捕获）
└── acix/                   # ACIX 协议（Agent-Computer Interaction eXchange）
    ├── mod.rs
    ├── aci.rs              # ACI 核心
    ├── experience.rs       # 经验记录
    ├── grounding.rs        # 接地
    ├── task.rs             # 任务定义
    └── tools.rs            # 工具集成
```

### 9.2 TUI 组件

基于 `ratatui` + `crossterm` 的终端 UI，包含：
- 聊天视图（`chat.rs`）— 对话消息渲染
- Markdown 渲染（`markdown.rs`）— 支持语法高亮的 Markdown
- 流式输出（`streaming.rs`）— LLM 响应实时显示
- 审批对话框（`approval_dialog.rs`）— 危险操作确认
- 自动补全（`completion.rs`）— 命令/技能/文件补全
- 子 Agent 视图（`subagent_view.rs`）— 多 Agent 状态监控
- 计划视图（`plan_view.rs`）— BrainCore Plan 可视化

### 9.3 消息发送模式

CLI 支持两种模式（`tui/cli.rs`）：
1. **TUI 模式**（无参数）— 启动交互式终端 UI，自动启动守护进程
2. **单消息模式**（`-m "msg"`）— 发送单条消息到守护进程并获取响应

---

## 10. memory — 记忆系统

**路径**: `crates/memory/src/`
**Cargo.toml**: 描述为 "Aletheon Memory system - episodic, semantic, and procedural memory"

### 10.1 模块结构

```
memory/src/
├── lib.rs
├── backends/             # SQLite 后端实现
│   ├── episodic/         # 情景记忆（始终可用）
│   │   ├── mod.rs        # EpisodicMemory 结构体
│   │   ├── schema.rs     # SQLite schema
│   │   ├── storage.rs    # CRUD 操作
│   │   └── query.rs      # 查询逻辑
│   ├── semantic/         # 语义记忆（cognitive-memory feature）
│   │   ├── mod.rs
│   │   ├── schema.rs
│   │   ├── storage.rs
│   │   └── query.rs
│   ├── procedural.rs     # 程序记忆（cognitive-memory feature）
│   └── self_memory.rs    # 自我记忆（cognitive-memory feature）
├── ops/                  # 记忆操作
│   ├── mod.rs
│   ├── activation.rs     # 激活计算
│   ├── decay.rs          # 衰减算法
│   ├── consolidation.rs  # 巩固（cognitive-memory feature）
│   ├── router.rs         # MemoryRouter（cognitive-memory feature）
│   └── schema.rs         # 数据库 Schema 管理
└── testing/
    └── mock_memory.rs    # Mock 实现（测试用）
```

### 10.2 Feature 门控设计（M-H Option A）

EpisodicMemory **始终可用**（守护进程用它存储反思）。Cognitive 后端（MemoryRouter + semantic/procedural/self）默认关闭，通过 `cognitive-memory` feature 启用。

### 10.3 记忆生命周期

```
存储 → 激活计算 → 访问 → 衰减 → (巩固|压缩|删除)
                        ↓
                     检索查询（文本 + 语义向量 + 标签 + 时间范围）
```

核心算法：
- **Activation**（`ops/activation.rs`）：`compute_activation(entry) → f64`
- **Decay**（`ops/decay.rs`）：`should_forget(entry) → bool`、`compute_strength(entry) → f64`
- **Consolidation**（`ops/consolidation.rs`）：将情景记忆转化为语义/程序记忆

---

## 11. metacog — 元运行时

**路径**: `crates/metacog/src/`
**Cargo.toml**: 描述为 "Aletheon Meta runtime - self-update and morphological evolution"

### 11.1 模块结构

```
metacog/src/
├── lib.rs
├── core/
│   ├── meta_cognition.rs  # MetaCognition 结构体
│   ├── traits.rs          # DefaultMetaRuntime
│   └── types.rs           # GenomeSpec, EvolutionStep 等核心类型
├── bridge/
│   ├── genome_bridge.rs   # Genome 桥接
│   └── candidate_bridge.rs # Candidate 桥接
└── impl/
    ├── genome/
    │   ├── loader.rs      # GenomeLoader — 从 YAML 文件加载基因组
    │   └── mod.rs
    ├── meta_runtime/
    │   ├── mod.rs
    │   ├── evaluator.rs       # 候选评估器
    │   ├── migration.rs       # 迁移逻辑
    │   ├── rollback.rs        # 回滚逻辑
    │   ├── sandbox_runner.rs  # 沙箱测试运行器
    │   ├── runtime_builder.rs # 运行时构建器
    │   ├── self_reader.rs     # 自我读取器
    │   ├── spec_editor.rs     # 规范编辑器
    │   └── lineage.rs         # 谱系追踪
    ├── morphogenesis/
    │   ├── pipeline.rs        # MorphogenesisPipeline — 形态发生管道
    │   ├── candidate.rs       # 候选生成
    │   └── mutation_intent.rs # 变异意图处理
    └── event_handlers/
        └── mutation_executor.rs # 变异执行器
```

### 11.2 自我进化管道

MorphogenesisPipeline 实现自我修改的完整流程：

```
MutationIntent 产生
  → generate_candidate()   [基因组变异 → RuntimeCandidate]
  → sandbox_test()         [沙箱中测试候选]
  → evaluate()             [评分 + 推荐]
  → SelfField.review_mutation() [自我审批]
  → migrate()              [执行迁移]
  → rollback()             [失败时回滚]
```

`GenomeLoader` 从 YAML 文件加载基因组配置（参考 `examples/self-evolution-loop/genome.yaml`）。

---

## 12. aletheon — 统一入口

**路径**: `crates/aletheon/src/main.rs`
**Cargo.toml**: 描述为 "Aletheon — unified AI agent CLI (daemon, exec, TUI)"

### 12.1 CLI 子命令（`main.rs:20-84`）

```rust
enum Commands {
    Daemon {            // 启动守护进程
        config, env, socket, container, image, enable_evolution
    },
    Exec {              // 非交互执行
        prompt, model, max_turns, sandbox, working_dir, config, output
    },
    Version,            // 打印版本
}
```

**三种运行模式**：

| 模式 | 触发条件 | 处理逻辑 |
|------|---------|---------|
| Daemon | `aletheon daemon [opts]` | 自动检测 systemd/容器/前台模式，启动守护进程 |
| Exec | `aletheon exec -p "..."` | 独立 Agent 循环（Provider → Tools → 审计），输出 text/json |
| TUI | 无子命令，无 `-m` | 启动交互式 TUI（自动启动守护进程） |
| Single Message | `aletheon -m "hello"` | 发送单条消息到已有守护进程 |

### 12.2 守护进程模式检测（`main.rs:187-201`）

```
NOTIFY_SOCKET 环境变量存在 → Systemd 模式
CONTAINER 环境变量 或 /.dockerenv 存在 → Container 模式
否则 → Foreground 模式
```

### 12.3 Exec 模式执行流程（`main.rs:274-461`）

非交互执行路径展示了完整的 Agent 循环：

1. 加载 `~/.aletheon/.env`（Provider API keys）
2. 加载配置 → 构建 `ProviderRegistry` → 创建 `LlmProvider`
3. 创建 `ToolRegistry` + `ToolRunnerWithGuard`（含沙箱 + 审批门）
4. Agent 循环：
   - LLM 推理 → 检查 `StopReason`
   - `EndTurn` / `MaxTokens` → 提取最终文本响应
   - `ToolUse` → 执行工具（runner.run）→ 收集结果 → 追加到对话历史
5. 输出 text 或 JSON 格式结果

---

## 13. aletheon-monitor — 监控工具

**路径**: `tools/aletheon-monitor/`
**语言**: Python
**类型**: MCP Server（Model Context Protocol）

### 14.1 模块结构

```
tools/aletheon-monitor/
├── src/
│   ├── __init__.py, __main__.py
│   ├── server.py           # MCP 服务器入口
│   ├── client.py           # AletheonClient（Unix socket JSON-RPC）
│   ├── frame.py            # TUI 帧解析
│   ├── tui_checks.py       # TUI 渲染健康检查
│   ├── tui_session.py      # TUI 会话管理
│   ├── anomaly.py          # 异常检测
│   └── tools/
│       ├── analyze.py      # 综合分析
│       ├── ask.py          # 向 daemon 发送消息
│       ├── diagnose.py     # 诊断工具（15 个子命令）
│       ├── health.py       # 健康检查
│       ├── journal.py      # 日志查看
│       ├── logs.py         # 日志分析
│       ├── memory.py       # 记忆查询
│       ├── sessions.py     # 会话查看
│       ├── snapshot.py     # 快照
│       ├── tui.py          # TUI 操作（启动/发送/捕获/停止）
│       └── watch.py        # 实时监控
└── tests/
    ├── conftest.py
    ├── test_frame.py, test_tui_checks.py, test_tui_session_smoke.py
    ├── test_server_dispatch.py, test_tui_wrappers.py, test_diagnose.py
    └── fixtures/real_session_dup.txt
```

### 14.2 MCP 工具清单（15 个）

| 工具名 | 功能 |
|--------|------|
| `aletheon_health` | daemon 健康状态 |
| `aletheon_snapshot` | daemon 状态快照 |
| `aletheon_analyze` | 综合分析（健康 + 日志 + 记忆） |
| `aletheon_journal` | 事件日志查看 |
| `aletheon_logs` | daemon 日志分析 |
| `aletheon_memory` | 记忆查询 |
| `aletheon_sessions` | 会话列表与详情 |
| `aletheon_ask` | 向 daemon 发送消息 |
| `aletheon_watch` | 实时监控 |
| `aletheon_tui_start` | 启动 TUI 会话（tmux） |
| `aletheon_tui_send` | 向 TUI 发送输入 |
| `aletheon_tui_capture` | 捕获 TUI 渲染帧 |
| `aletheon_tui_stop` | 停止 TUI 会话 |
| `aletheon_diagnose` | 运行诊断套件 |

---

## 14. 数据流与执行路径

### 14.1 守护进程模式完整请求路径

```
Client (TUI/CLI -m)
  │  Unix socket connect
  ▼
┌─────────────────────────────────────────────────┐
│ UnixServer (server.rs)                          │
│   → verify peer credentials (UID/GID)           │
│   → per-connection notify channel               │
└──────────────────┬──────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────┐
│ RequestHandler (handler/mod.rs)                 │
│   → JSON-RPC dispatch                           │
│   → chat / rpc_* routing                        │
│   → SessionState → AletheonRuntime              │
└──────────────────┬──────────────────────────────┘
                   │
    ┌──────────────┼──────────────┐
    ▼              ▼              ▼
┌────────┐    ┌──────────┐   ┌──────────┐
│SelfField│   │BrainCore │   │BodyRuntime│
│ review │    │  think   │   │ execute   │
│ (策略) │    │ (推理)   │   │ (工具)    │
└───┬────┘    └────┬─────┘   └────┬─────┘
    │              │              │
    ▼              ▼              ▼
┌─────────────────────────────────────────────────┐
│ CommunicationBus (事件总线)                      │
│   → PubSub / RequestResponse / Mailbox           │
└─────────────────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────┐
│ Memory System                                    │
│   → CoreMemory (工作) → EpisodicMemory (持久)    │
│   → AutoMemory → ArchivalMemory                  │
└─────────────────────────────────────────────────┘
```

### 14.2 ReAct 循环单步流程

```
┌─────────────────────────────────────────┐
│ ReActLoop.step()                         │
│                                          │
│  1. Compose messages                     │
│     ├─ System prompt (immutable)         │
│     ├─ Pending memory injections         │
│     ├─ Plan mode marker (if active)      │
│     └─ Conversation history              │
│                                          │
│  2. Check circuit_breaker               │
│     ├─ Repeated tool calls? → stop       │
│     └─ Consecutive errors? → impasse     │
│                                          │
│  3. LLM completion                       │
│     ├─ Text response → return to user    │
│     ├─ Tool use → execute tools          │
│     │   ├─ partition_tool_calls (batch)  │
│     │   ├─ ToolRunnerWithGuard.run()     │
│     │   │   ├─ HookCheck → ApprovalGate  │
│     │   │   ├─ SandboxPreference         │
│     │   │   └─ AuditLogger               │
│     │   └─ Collect results               │
│     └─ Continue → next iteration         │
│                                          │
│  4. Post-turn processing                 │
│     ├─ Update goal_tracker               │
│     ├─ Check reflection_engine           │
│     ├─ Update metrics                    │
│     └─ AutoMemory().store()              │
│                                          │
│  5. Context compaction (if needed)       │
│     └─ AdvancedCompressor.compress()     │
└─────────────────────────────────────────┘
```

### 14.3 自我进化管道

```
┌──────────────────────────────────────────┐
│ EvolutionCoordinator (per-turn trigger)   │
│                                           │
│  1. Collect turn metrics                  │
│     ├─ success/failure                    │
│     ├─ tool_calls, tool_errors            │
│     ├─ elapsed_ms, iterations             │
│     └─ AwarenessSignals (from ReActLoop)  │
│                                           │
│  2. ExperienceSummarizer                  │
│     ├─ Analyze patterns                   │
│     ├─ Detect behavior clusters           │
│     └─ Propose adjustments                │
│                                           │
│  3. MorphogenesisPipeline                 │
│     ├─ GenomeLoader → current genome      │
│     ├─ generate_candidate (mutation)      │
│     ├─ sandbox_test                       │
│     ├─ evaluate                           │
│     └─ SelfField.review_mutation()        │
│         ├─ Approve → migrate()            │
│         └─ Reject → record + skip         │
│                                           │
│  4. Persist evolution log                 │
│     └─ EpisodicMemory.store(              │
│         EvolutionLogEntry)                │
└──────────────────────────────────────────┘
```

---

## 15. 附录：文件清单

### 15.1 各 Crate 源文件统计

| Crate | 源文件数 | 主要职责 |
|-------|---------|---------|
| base | ~55 | ABI 接口（零实现） |
| runtime | ~100 | 运行时编排与集成 |
| corpus | ~70 | 工具执行体 |
| dasein | ~70 | 自我策略引擎 |
| cognit | ~45 | 认知计算引擎 |
| interact | ~35 | 用户交互层 |
| memory | ~20 | 记忆存储 |
| metacog | ~25 | 元运行时 |
| aletheon | 1 (+ 4 test) | CLI 入口 |
| **总计** | **~420** | |

### 15.2 集成测试文件

| 路径 | 测试范围 |
|------|---------|
| `crates/aletheon/tests/integration/daemon_lifecycle.rs` | 守护进程生命周期 |
| `crates/aletheon/tests/integration/socket_auth.rs` | Socket 认证 |
| `crates/aletheon/tests/integration/api_stress.rs` | API 压力测试 |
| `crates/aletheon/tests/integration/main.rs` | 集成测试主入口 |
| `crates/base/tests/protocol_e2e.rs` | 协议端到端 |
| `crates/base/tests/execpolicy_tests.rs` | 执行策略测试 |
| `crates/base/tests/mock_subsystems.rs` | Mock 子系统 |
| `crates/runtime/tests/` (10+ files) | 运行时全覆盖测试 |

### 15.3 CI/CD 工作流（`.github/workflows/`）

| 文件 | 用途 |
|------|------|
| `ci.yml` | CI：fmt, clippy, build, test, MCP 测试 |
| `release.yml` | 发布构建（多目标） |
| `enforce-dev-only-pr.yml` | PR 目标分支限制（仅 dev） |
| `auto-merge-dev-to-main.yml` | dev → main 自动合并 |

### 15.4 配置文件

| 路径 | 用途 |
|------|------|
| `config/default.toml` | 编译默认值（provider、sandbox、memory） |
| `config/aletheon.user.service` | systemd 用户服务单元 |
| `~/.aletheon/config.toml` | 用户级运行时配置 |
| `~/.aletheon/.env` | API keys（Provider） |
| `/etc/agentd/config.toml` | 系统级配置 |

---

> 本文档基于 `aletheon` 仓库 commit `50c6bd7` (dev branch) 的实际代码编写。
> 所有文件路径引用对应代码库中的实际位置。

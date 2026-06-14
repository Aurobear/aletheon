# 多 Agent 编排引擎

> 可插拔的多 Agent 协作编排系统，支持 Selector/Handoff/DiGraph 策略，委托统一为 Tool 调用。

**模块编号:** 06
**关联模块:** [认知引擎](../core/cognitive-engine.md), [IPC 与内核](../platform/kernel-ipc.md), [安全模型](../security/security-model.md), [混合推理](hybrid-inference.md)
**最后更新:** 2026-06-06

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| Agent trait | ✅ Implemented | `orchestration/agent.rs` | Core agent abstraction |
| AgentRegistry | ✅ Implemented | `orchestration/registry.rs` | Agent registration and lookup |
| DelegateTool | ✅ Implemented | `orchestration/delegate.rs` | Delegation as tool call |
| SelectorStrategy | ✅ Implemented | `orchestration/selector.rs` | Agent selection strategy |
| HandoffStrategy | ✅ Implemented | `orchestration/handoff.rs` | Agent handoff pattern |
| TerminationConditions | ✅ Implemented | `orchestration/termination.rs` | Stop conditions for orchestration |
| IterationBudget | ✅ Implemented | `orchestration/budget.rs` | Token/iteration budget control |
| DiGraph | ✅ Implemented | `orchestration/digraph/` | DAG-based orchestration graph (edge, node, state) |
| Built-in agents | ✅ Implemented | `orchestration/builtin/` | fs_agent, net_agent, code_agent |
| ConfigAgent | ✅ Implemented | `orchestration/config_agent.rs` | Configuration-driven agent |

---

## 目录

1. [概述](#1-概述)
2. [当前设计](#2-当前设计)
3. [已识别缺陷](#3-已识别缺陷)
4. [改进设计](#4-改进设计)
5. [实现要点](#5-实现要点)
6. [参考来源](#6-参考来源)

---

## 1. 概述

编排引擎负责协调多个专业 Agent 协作完成复杂任务。核心创新来自 AutoGen 的可插拔编排策略与 CrewAI 的"委托即工具"模式，将 Agent 委托统一为普通 Tool 调用，简化多 Agent 交互接口。

编排引擎按 Phase 渐进交付：

| 阶段 | 策略 | 适用场景 |
|------|------|----------|
| Phase 1 | SingleAgent | 单 Agent + 工具调用，最简启动 |
| Phase 4a | Selector | LLM 路由选择 Agent，适合文件/网络/进程分发 |
| Phase 4b | Handoff (Swarm) | 显式委托，适合复杂任务分解 |
| Phase 6 | DiGraph | DAG 工作流 + 条件边 + 并行扇出 |

---

## 2. 当前设计

### 2.1 编排策略 (Process Strategy)

**ProcessStrategy** — 四种编排策略：
- **SingleAgent** — 一个主 Agent 运行 ReAct 循环（Phase 1）
- **Selector** — LLM 路由选择 Agent（Phase 4a），适合文件/网络/进程分发
- **Handoff (Swarm)** — 显式委托，委托 = 一个 Tool 调用（Phase 4b），适合复杂任务分解
- **DiGraph** — DAG 工作流 + 条件边 + 并行扇出（Phase 6），适合编译->测试->部署流水线

### 2.2 Agent 注册表

**Agent trait** — 核心 agent 抽象，包含 id, capabilities, tools, on_messages。
- 代码位置: `orchestration/agent.rs`

**AgentRegistry** — Agent 注册表，支持注册、查找、生命周期管理。
- 代码位置: `orchestration/registry.rs`

```
┌──────────────┬────────────────┬─────────────────┐
│ Agent ID     │ 能力声明        │ 可用工具         │
├──────────────┼────────────────┼─────────────────┤
│ coordinator  │ 任务分解/路由   │ delegate, plan  │
│ fs_agent     │ 文件系统操作    │ read,write,grep │
│ net_agent    │ 网络操作        │ curl,ssh,dns    │
│ proc_agent   │ 进程管理        │ ps,kill,systemd │
│ code_agent   │ 代码执行        │ bash,python     │
│ ui_agent     │ UI 自动化       │ click,type,snap │
└──────────────┴────────────────┴─────────────────┘
```

### 2.3 委托即工具 (DelegateTool)

**DelegateTool** — 借鉴 CrewAI 的核心创新，Agent 委托统一为 Tool 调用。
- 代码位置: `orchestration/delegate.rs`

交互示例：
```
coordinator: "帮我查下 nginx 的配置"
  → delegate(fs_agent, "读取 /etc/nginx/nginx.conf")
  → fs_agent 读取文件，返回内容
  → coordinator 拿到结果，继续推理
```

### 2.4 终止条件

**TerminationCondition** — 借鉴 AutoGen 的可组合终止条件，支持 And/Or 组合。
- 代码位置: `orchestration/termination.rs`
- 类型：MaxIterations, MaxTokens, Timeout, AndCondition, OrCondition

### 2.5 安全护栏 (Guardrail)

借鉴 CrewAI 的 Guardrail 模式，每个 Agent 的输出经过验证：

1. 命令白名单/黑名单检查
2. 权限级别验证 (L0-L3)
3. 副作用预估 (文件修改/网络请求/进程操作)
4. 失败 -> 重试或升级到人工确认

---

## 3. 已识别缺陷

### P2: 子 Agent 独立预算

**问题:** 当前设计中，delegated Agent 共享父 Agent 的全部上下文窗口和 token 预算。一个深层委托链或并行扇出可能导致预算耗尽，无 graceful 降级机制。

**影响:**
- 子 Agent 消耗父 Agent 全部预算后，父 Agent 和所有兄弟子 Agent 同时中断
- 无法追踪单个子 Agent 的资源消耗
- 无法对异常子 Agent 单独终止

### 3.1 P0: 子 Agent 权限继承与安全模型脱节

**问题:** 安全模型 (`security-model.md`) 和编排引擎各自定义了权限控制机制，但两者之间缺乏集成。具体表现为：权限等级体系未连通、硬编码的工具屏蔽列表、子 Agent 权限继承未定义、LoopDetector 作用域不明。

**影响:**
- 子 Agent 可以执行权限逃逸操作
- 子 Agent 通过派生孙 Agent 实现间接权限升级
- 权限决策分散在 `PolicyEngine` 和硬编码列表两处，审计困难

### 3.2 P1: 子 Agent 共享状态无隔离

**问题:** 子 Agent 直接继承父 Agent 的全部内存上下文，无作用域隔离——Token 消耗无独立上限、内存完全共享、DELEGATE_BLOCKED_TOOLS 过于粗暴。

**影响:**
- 一个行为异常的子 Agent 可耗尽父 Agent 全部 token 预算
- 多个子 Agent 同时写入共享内存导致语义冲突和知识泄露
- 子代理无法拥有私有工作记忆

### 3.3 P2: 子 Agent 可观测性缺失

**问题:** 活跃代理注册表仅为概念、中断传播无协议、资源压力下的暂停/恢复未设计、父 Agent 无法观测子 Agent 中间推理。

**影响:**
- 子 Agent 挂起不可检测（无心跳机制）
- 父 Agent 被取消后，孤儿子 Agent 继续运行消耗资源
- 多 Agent 流程调试完全依赖日志，无结构化事件流

### 3.4 P2: DiGraph 设计不足

**问题:** DiGraph 作为 Phase 6 核心功能，设计仅停留在概念层面——无节点定义格式、无边条件语法、无节点间状态传递机制、无错误处理策略、无检查点与恢复、无并行扇出-汇合设计。

---

## 4. 改进设计

### 4.1 IterationBudget — 独立迭代预算

> **设计变更**: 原设计将 token 预算与迭代预算耦合，并采用"从父预算切分"模式。
> 经 Hermes 实际验证，迭代预算和 token 追踪是两个独立关注点。每个子 Agent 应获得**完全独立**的迭代预算，而非从父 Agent 份额中切分。

核心结构: `IterationBudget` — 轻量级 consume/refund 计数器，线程安全 (AtomicUsize)。每个子 Agent 实例持有独立预算。

预算操作: `consume()` 尝试消耗一次迭代，`refund()` 退还不应计费的迭代（失败重试、execute_code 沙箱轮次、0-API-call 超时），`remaining()` / `used()` 只读查询。

```
新设计（独立模式）:
  父 Agent:  IterationBudget(90)     ← 配置 parent.max_iterations
  ├── fs_agent:   IterationBudget(50) ← 配置 delegation.max_iterations
  ├── net_agent:  IterationBudget(50)
  └── code_agent: IterationBudget(50)
  → 总迭代可达 90+50+50+50=240，但每个子 Agent 最多 50
  → 一个子 Agent 耗尽不影响其他子 Agent 和父 Agent
```

Token 使用量在 session 级别追踪（见认知引擎），不在 IterationBudget 中耦合。父 Agent 通过 `cost_aggregation` 汇总所有子 Agent 的 token 消耗用于成本报告。

### 4.2 集成到 DelegateTool

关键设计参数:
- `DELEGATE_BLOCKED_TOOLS` — 子 Agent 被禁止使用的工具集 (delegate_task, clarify, memory, send_message, execute_code)
- `MAX_DELEGATE_DEPTH` — 最大委托深度，默认 1（不允许孙 Agent）
- `DelegationConfig` — max_iterations (50), max_concurrent_children (3), max_depth (1), provider_override

委托流程: 深度检查 → Agent 查找 → 创建独立 IterationBudget → 剥离被禁止的工具 → 构造 Task → 执行。子 Agent 拥有独立上下文（无父 Agent 历史），父 Agent 只看到委托调用和最终摘要。

预算耗尽时返回 `ToolResult::partial(summary)` — 包含工具调用链、文件操作、输出尾部。

### 4.3 并行扇出的预算分配

每个子 Agent 获得独立预算，不存在"均分"逻辑。并行执行使用 `JoinSet` + `Semaphore(max_concurrent_children)` 控制并发上限。结果按 `task_index` 排序保序。中断传播: 父 Agent 中断 → `abort_all` → 子 Agent 停止。

子 Agent 超时处理: 0 次 API 调用超时 → 退还 IterationBudget → 写入诊断日志 → 返回 `ToolResult::timeout()`。

### 4.4 子 Agent 权限继承与安全集成

移除硬编码的 `DELEGATE_BLOCKED_TOOLS`，改为通过 `PolicyEngine` 动态派生子 Agent 权限。

默认降级规则:
- 父 L3 -> 子 L2（禁止危险操作）
- 父 L2 -> 子 L1（禁止系统目录写入）
- 父 L1 -> 子 L0（只读）

`LoopDetector` 改为按 Agent ID 独立追踪循环计数，同时支持可选的跨 Agent 聚合检测。

### 4.5 Per-Sub-Agent Token 预算与内存作用域

**Per-Sub-Agent Token 预算:** 在 `SessionTokenTracker` 中添加子 Agent 级别的 token 限额（默认为父级总预算的 20%，可配置）。

**三层内存作用域 (MemoryScope):**

```rust
enum MemoryScope {
    Global,    // 共享，子 Agent 默认只读
    Session,   // 父 Agent + 所有子 Agent 可见
    Agent,     // 仅当前 Agent 私有
}
```

写入规则: 子 Agent 默认写入 `AgentScope`（私有）；写入 `SessionScope` 需父 Agent 审批；`GlobalScope` 仅父 Agent 可写。

### 4.6 AgentRegistry 与生命周期事件协议

**AgentHandle 核心接口:** agent_id, status (Arc<Mutex<AgentStatus>>), cancel_token, event_receiver。

**AgentStatus 状态机:** Spawning -> Running -> Blocked/Completed/Failed/Cancelled。

**心跳检测:** 子 Agent 每 30 秒心跳，连续 90 秒未收到心跳按配置执行 TimeoutAction (Log/Interrupt/Cancel)。

**级联取消协议:** 父 Agent 取消时传播 `CancelToken`，子 Agent 等待当前工具调用完成（最长 10 秒）后写入 Checkpoint 并终止。

**结构化事件流:** Progress/ToolCallSummary/Checkpoint/Error，仅摘要信息。

### 4.7 DiGraph 完整执行规范

**节点类型:** Agent, Branch (条件分支), HumanApproval, SubGraph。

**边与条件:** JSONPath + 比较运算的条件表达式，支持 Always/When(expr)/Default 三种边类型。

**状态传递:** `GraphState` 作为所有节点共享的 typed dict，上游输出以 `NodeId` 为 key 存入 context。

**错误处理:** 每个节点配置 `RetryPolicy` (max_retries + BackoffStrategy)，重试耗尽后根据 OnExhausted 策略处理 (FailGraph/SkipNode/Escalate)。

**检查点与恢复:** 每个节点完成后自动检查点到 `~/.agent/checkpoints/`。恢复时已完成节点不重新执行。

**并行扇出-汇合:** `FanOutNode` 生成 N 个并行节点，`JoinStrategy` 支持 All/Any/FirstN(n)/TimeoutAll。

---

## 5. 实现要点

- **Phase 4a (Selector)**: 无需 IterationBudget，单 Agent 路由即可。
- **Phase 4b (Handoff)**: 引入 IterationBudget，每次委托创建独立预算（默认 50 次迭代）。
  - 子 Agent 禁用 `DELEGATE_BLOCKED_TOOLS`，剥离工具集在构造时完成。
  - 子 Agent 拥有独立上下文，父 Agent 只看委托调用 + 最终摘要。
  - 单任务委托直接执行，无额外开销。
- **Phase 6 (DiGraph)**: 并行扇出使用 `JoinSet` + `Semaphore` 控制并发。
  - 每个子 Agent 独立预算，无需预分配或回收。
  - 结果按 `task_index` 排序保序。
  - 中断传播：父 Agent 中断 → `abort_all` → 子 Agent 停止。
- **预算耗尽**: 子 Agent 迭代用尽 → 停止执行 → 返回已产出的摘要，通过 `ToolResult::partial()` 返回给父 Agent。
- **退费机制**: 对"免费"操作退还未消耗迭代：失败重试、execute_code 沙箱轮次、0-API-call 超时。
- **成本汇总**: 子 Agent 的 `session_estimated_cost_usd` 累加到父 Agent 总成本。
- **审计日志**: 每次 IterationBudget 的 consume/refund 事件写入审计日志。
- **子 Agent 可观测性** (Phase 6): 活跃子 Agent 注册表、中断传播链、全局 spawn 暂停/恢复开关、超时诊断。
- **子 Agent 凭证隔离** (Phase 6+): 支持 `delegation.provider` 配置将子 Agent 路由到不同 provider:model。

---

## 6. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| **AutoGen** (`autogen_agentchat/teams/`) | Selector/Swarm/DiGraph 编排策略、可组合终止条件、HeadAndTail 上下文管理 |
| **CrewAI** (`crewai/crew.py:159`) | 委托即工具 (DelegateTool)、Process 策略、Guardrail 护栏 |
| **Hermes IterationBudget** (`hermes-agent/agent/iteration_budget.py`) | 独立预算（默认 50 次迭代）、consume/refund 线程安全计数器、execute_code/失败重试退费 |
| **Hermes DelegateTool** (`hermes-agent/tools/delegate_tool.py`) | 批量并行委托（ThreadPoolExecutor + max_concurrent_children=3）、DELEGATE_BLOCKED_TOOLS、MAX_DEPTH=1、子 Agent 独立上下文 |
| **Hermes 子 Agent 可观测性** (`hermes-agent/agent/conversation_loop.py`) | 活跃子 Agent 注册表、中断传播、spawn 暂停开关、心跳/停滞检测、超时诊断转储 |
| **LangGraph** (`langgraph/pregel/`) | 检查点恢复、Per-node 策略 (RetryPolicy/CachePolicy/TimeoutPolicy) |
| **AutoGen Society of Mind** | 团队包装为单个 Agent 的层级组合模式 |

---

## Implementation Summary

**Code location:** `crates/agent-core/src/orchestration/`

**Key types/traits implemented:**
- `Agent` trait (`agent.rs`) — core agent abstraction with id, capabilities, tools, on_messages
- `AgentConfig`, `Capability`, `AgentResponse`, `AgentResponseStatus` (`agent.rs`)
- `AgentRegistry` (`registry.rs`) — thread-safe agent registration, lookup by id/capability/tool pattern, config loading
- `DelegateTool` (`delegate.rs`) — delegation as tool call with depth check, budget creation, tool filtering, parallel batch execution
- `DelegationConfig` (`delegate.rs`) — max_iterations, max_concurrent_children, max_depth, provider_override
- `SelectorStrategy` (`selector.rs`) — LLM-based agent selection with routing
- `HandoffStrategy` (`handoff.rs`) — explicit agent handoff pattern
- `TerminationCondition` trait (`termination.rs`) — composable conditions: MaxIterations, MaxTokens, Timeout, AndCondition, OrCondition
- `IterationBudget` (`budget.rs`) — thread-safe consume/refund counter with AtomicUsize
- `DiGraph` (`digraph/`) — DAG orchestration with edge, node, state submodules
- `ConfigAgent` (`config_agent.rs`) — configuration-driven agent definition
- Built-in agents (`builtin/`) — fs_agent, net_agent, code_agent

**Test coverage:** Unit tests exist for IterationBudget (2 tests), DiGraph edges (4 tests), DiGraph state (2 tests), ConfigAgent (2 tests), DelegateTool (1 test). No integration tests for multi-agent orchestration flows.

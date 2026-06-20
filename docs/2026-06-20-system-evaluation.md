# Aletheon 系统全量评估

> 开发者日志 | 2026-06-20 | 作者：aurobear
>
> 对自己项目的诚实评估：每个 crate 做了什么、做得怎样、缺什么、哲学映射到不到位。

---

## 目录

1. [总览数据](#1-总览数据)
2. [逐 Crate 评估](#2-逐-crate-评估)
3. [哲学-代码映射深度分析](#3-哲学-代码映射深度分析)
4. [跨系统集成评估](#4-跨系统集成评估)
5. [当前完成度评级](#5-当前完成度评级)
6. [改进方向与优先级](#6-改进方向与优先级)
7. [工作量估算](#7-工作量估算)

---

## 1. 总览数据

| 指标 | 数值 |
|---|---|
| 项目创建日期 | 2026-06-06 |
| 本次评估日期 | 2026-06-20 |
| 开发周期 | **14 天** |
| 总 Rust 代码行数 | **~97,000 行**（含测试） |
| Crate 数量 | 10（8 个核心库 + 2 个二进制入口 + 2 个 example） |
| 测试数量 | **1,215 个，全部通过** |
| 包含测试的文件数 | 217 |
| `cargo check` | ✅ 通过（23 warnings，0 errors） |
| `cargo test` | ✅ 全部通过（含 431 个集成测试） |
| TODO/FIXME/unimplemented! 标记 | **极少**（约 10 个，多数在 Phase 1 限制说明中） |

### 代码量分布

| Crate | 代码行数 | 占比 |
|---|---|---|
| `aletheon-body` | 30,713 | 31.6% |
| `aletheon-runtime` | 28,375 | 29.2% |
| `aletheon-self` | 13,860 | 14.3% |
| `aletheon-brain` | 9,298 | 9.6% |
| `aletheon-abi` | 4,672 | 4.8% |
| `aletheon-comm` | 3,890 | 4.0% |
| `aletheon-memory` | 3,591 | 3.7% |
| `aletheon-meta` | 2,115 | 2.2% |
| binaries | 387 | 0.4% |
| examples | 356 | 0.4% |

---

## 2. 逐 Crate 评估

### 2.1 aletheon-abi — 类型定义层（4,672 行）

**完成度：★★★★★（95%）**

这是整个系统的"语言"。所有跨 crate 共享的类型都定义在这里。

**做了什么：**
- `Message` / `Role` / `ContentBlock` — 完整的消息模型，支持文本、工具调用、工具结果
- `ToolDefinition` / `ToolResult` — 工具定义和执行结果
- `EventBus` / `EventHandler` / `EventEnvelope` — 事件系统抽象
- `SelfAwareness` / `SelfAwarenessExtension` — 自我觉知的种子类型
- `Intent` / `IntentSource` — 意图系统
- `Identity` / `BoundaryRule` / `CarePriority` — 自我模型类型
- `Genome` / `Topology` / `SubsystemSpec` — 基因组定义
- `SandboxConfig` / `SecurityPolicy` — 安全沙箱类型
- `LlmConfig` / `InferenceRequest` / `InferenceResponse` — LLM 接口
- `Hook` / `HookTrigger` / `McpServerConfig` — Hook 和 MCP 配置
- `EvolutionLogEntry` / `ReflectionEntry` — 进化和反思日志
- `MemoryEntry` / `MemoryType` / `MemoryQuery` — 记忆系统类型
- IPC 类型：`AgentMessage` / `IpcEnvelope` / `IpcBackend`

**评价：** 类型设计非常干净，derives 完整，序列化支持好。唯一的小问题是某些类型可能过度设计（比如 `AwarenessExtensionCounts` 可以用 HashMap 替代），但作为 ABI 层这是可以接受的。

---

### 2.2 aletheon-comm — 通信层（3,890 行）

**完成度：★★★★☆（80%）**

**做了什么：**
- `KernelEventBus` — 真实的事件总线实现，pub/subscribe/dispatch 全链路
- `SubscriptionRegistry` — HashMap-based 事件分发，支持早期终止
- `EventLog` — 环形缓冲区事件日志
- `RoutingPolicy` — 基于事件类型和优先级的路由策略
- `InProcessTransport` — 进程内传输，支持 Module/Agent/Topic/Broadcast 四种路由
- `UnixSocketTransport` — Unix domain socket 传输（JSON 序列化）
- `RequestResponseProtocol` — 真实的请求-响应协议，带关联 ID 和超时
- `CommunicationBus` — 统一入口，组合所有子系统
- `DebugBusHook` — 调试总线，支持事件过滤、录制、性能计数器
- IPC 子系统：`UnixSocketBackend`（bincode）、`SharedMemBackend`（memfd+mmap）、`PriorityQueue`、`JsonRpcAdapter`、`IpcManager`（自动检测环境）

**真实差距：**
- `KernelEventBus::request()` 是 Phase 1 stub — 发布后超时返回错误（有 `RequestResponseProtocol` 补偿）
- `io_uring` — 完全是死代码。feature flag 不在 Cargo.toml 中，`try_recv` 有 TODO
- `bridge/mod.rs` — 空占位模块
- 路由策略只 log 警告，不实际拦截

**评价：** 核心通信链路是真实的。Unix socket 和 shared memory 都能工作。但 io_uring 是纯骨架。

---

### 2.3 aletheon-memory — 记忆系统（3,591 行）

**完成度：★★★★☆（85%）**

**做了什么：**
- **四层记忆后端全部用 SQLite 实现：**
  - `EpisodicMemory`（996 行）— 事件、反思、觉知、进化日志，支持时间范围查询
  - `SemanticMemory`（569 行）— **FTS5 全文搜索**，porter stemming + unicode61 tokenizer
  - `ProceduralMemory`（498 行）— 技能/工作流，自动版本递增，成功率加权排序
  - `SelfMemory`（470 行）— 身份变更追踪，**lineage 图谱**，forget 需审批
- `MemoryRouter`（370 行）— 路由到正确后端，**fan-out 查询**带错误容错
- `decay.rs` — **Ebbinghaus 遗忘曲线**：`strength = base * e^(-rate * days)`，半衰期 7 天（`ln(2)/7 = 0.099`）
- `activation.rs` — **ACT-R 激活评分**：base*0.4 + recency*0.35 + frequency*0.25
- `MockMemoryBackend` — 完整的内存 mock，用于测试

**真实差距：**
- **decay 和 activation 算法没有接入 recall 管道** — 后端的 `recall()` 用 SQL 排序，不调用 `compute_strength()` 或 `compute_activation()`
- 没有向量存储（LanceDB 只是 Cargo.toml 里的 feature flag，没有实际代码）
- 没有 embedding 生成
- 没有 consolidation 运行（L2→L3 的记忆巩固）

**评价：** SQLite 存储层是实打实的。FTS5 搜索是真实的。decay/activation 算法数学正确但没接入。这是"引擎造好了，传动轴没连上"的状态。

---

### 2.4 aletheon-self — 自我层（13,860 行）

**完成度：★★★☆☆（65%）**

**做了什么：**
- **8 层 SelfField 结构全部有实现：**
  - `IdentityLayer` — 当前身份 + 变更历史链
  - `BoundaryLayer` — 边界规则评估（条件匹配 + 优先级）
  - `CareLayer` — 关怀权重系统（主题 → 权重映射）
  - `NarrativeLayer` — 叙事条目时间线
  - `ConflictLayer` — 冲突检测和解决（优先级比较）
  - `AttentionLayer` — 注意力焦点追踪
  - `ContinuityLayer` — 跨会话连续性检查点
  - `MutationLayer` — 变异历史记录
- `AwarenessGrowthAnalyzer` — 分析觉知历史，产生成长建议（基于扩展类型分布）
- `LoopDetector` — 重复模式检测
- `LlmBridge` — 通过 LLM 生成 enriched self-awareness
- `PerceptionBridge` — 将系统事件转化为内部状态

**真实差距：**
- `care` 权重是静态配置，不是 agent 在运行中自我调整的
- `AwarenessGrowthAnalyzer` 的分析逻辑是基于分布统计的启发式，不是真正的学习
- `boundary` 评估是字符串条件匹配，不是结构化的规则引擎
- 与 runtime 的集成是"被调用"模式，不是"主动感知"模式
- `narrative` 只是 append-only 条目列表，没有叙事推理

**评价：** 结构完整度很高——8 层都有代码。但每一层的深度都不够。这像是一座建筑的框架已经立起来了，墙体和装修还没做。

---

### 2.5 aletheon-brain — 认知层（9,298 行）

**完成度：★★★☆☆（60%）**

**做了什么：**
- `Reasoner` — 两种推理策略：Direct、ChainOfThought
- `Planner` — 将推理链转化为具体步骤
- `Reflector` — 执行后反思，产出 what_worked/what_failed/what_to_improve
- `Learner` — 从反思中提取规则
- `Critic` — 评估计划质量
- `SkillExtractor` — 从执行历史中提取可复用技能
- `WorldModel` — 世界状态建模
- `EvolutionTrigger` — 触发进化条件检测
- `LlmBridge` — LLM 调用封装
- `DualModel` — 双模型架构（本地 + 云端）
- `ProviderRegistry` — LLM provider 管理
- `MockLlm` — 测试用 mock

**真实差距：**
- **Reasoner 的 `think()` 最终是拼接字符串输出** — 没有真正的 token-level chain-of-thought，没有 reasoning tree，没有 self-consistency
- **Reflector 的反思是模板化的** — 成功 → "plan completed successfully"，失败 → "plan failed at step N"。不是真正的因果分析
- **Learner 没有真正的学习** — 规则提取是硬编码的模式匹配
- **WorldModel 是空壳** — 没有状态追踪，没有预测能力
- **Critic 是规则检查** — 不是真正的计划质量评估

**评价：** Brain 是整个系统最核心也是最薄弱的部分。所有组件都有结构，但都是"有骨架没肌肉"的状态。Reasoner 不推理，Reflector 不反思，Learner 不学习——这些名字暗示的能力和实际实现之间有巨大鸿沟。

---

### 2.6 aletheon-body — 执行层（30,713 行）

**完成度：★★★★★（95%）**

这是最大的 crate，也是完成度最高的。

**做了什么：**

**工具系统（20+ 个工具，全部真实）：**
- 文件：`file_read`、`file_write`、`file_search`、`glob`、`grep`、`apply_patch`（737 行的 unified diff 解析器）
- 系统：`bash_exec`、`process_list`、`system_status`
- Web：`web_fetch`、`web_search`
- 任务：`task_tools`（4 个工具，in-memory store）
- 内核：`kernel_build`、`ebpf_compile`、`module_build`、`module_load`
- 代码分析：`code_graph`（tree-sitter AST 分析）
- GUI 自动化：`acix_tools`（10 个工具，AT-SPI2 + OCR + 截图）
- 脚本：`script_tool`（外部脚本包装器）

**工具基础设施：**
- `ToolCallExecutor`（887 行）— 三阶段并发执行：ReadOnly 并行 → Write 按路径序列化 → SideEffect 全局序列化
- `PathConflictDetector` — 每路径信号量，防止写冲突
- `ToolsetRegistry` — 命名工具集（core/system/perception/memory/network/full），支持传递包含和环检测
- `ToolSearch` — BM25 搜索目录，让 LLM 发现延迟加载的工具
- `output/` 子系统 — 三层输出管理：capture（字节上限）→ truncation（行截断）→ persistence（溢出到文件）→ turn_budget（每 turn 预算）→ pruner（Hermes 三遍剪枝）

**MCP 集成（2,071 行）：**
- 完整的 OAuth 2.0 授权码流程 + PKCE
- Bearer token 认证
- 持久化 token 存储
- stdio + HTTP 传输
- 工具发现和注册

**TUI（5,195 行）：**
- ratatui + crossterm 完整终端 UI
- 聊天、markdown 渲染、工具卡片、审批对话框、流式输出、补全、状态栏

**评价：** 这是整个项目最扎实的部分。工具系统不是玩具——`apply_patch` 有 737 行的 unified diff 解析器，`executor` 有真正的并发控制，MCP 有完整的 OAuth 流程。TUI 也是生产级的。

---

### 2.7 aletheon-runtime — 运行时引擎（28,375 行）

**完成度：★★★★☆（75%）**

**做了什么：**
- `ReActLoop`（1,042 行）— ReAct 循环，支持并行工具批处理（ReadOnly 并行，SideEffect 串行）
- `Controller` — 会话控制器，管理对话状态
- `Orchestrator` — 编排器，协调多个子系统
- `StormBreaker` — 紧急中断机制
- `Checkpoint` — 运行时检查点
- `BehaviorPaths` — 行为路径管理
- `EventSink` — 事件接收器
- `Config` — 运行时配置
- Agent 子系统：`Harness`、`Budget`、`Fork`、`Process` — agent 生命周期管理
- Automation：`Delivery`、`SkillRouter` — 技能路由和自动化
- Coordinator — 多 agent 协调
- Memory compressor — 上下文压缩

**真实差距：**
- `ReActLoop` 中有 4 个 `unimplemented!()` 标记，都在测试辅助函数中
- reasoning 实际上是调 LLM API + 解析输出，不是本地推理
- context compression 是字符串截断/摘要，不是语义压缩
- `StormBreaker` 的触发条件是硬编码的

**评价：** Runtime 是系统的"脊柱"——它把 Brain、Body、Self 串起来。ReAct loop 是真实的，工具并行执行是真实的。但 "reasoning" 本质上还是 "调 LLM → 拿输出 → 执行"，没有突破 API 调用的边界。

---

### 2.8 aletheon-meta — 元运行时（2,115 行）

**完成度：★★★☆☆（70%）**

**做了什么（全部真实，零 stub）：**
- `GenomeLoader`（282 行）— YAML 基因组加载/保存/diff
- `MorphogenesisPipeline`（110 行）— **完整的自演化流水线**：generate_candidate → sandbox_test → evaluate → migrate
- `Evaluator`（300 行）— 双层评估：安全底线检查 + 加权评分（safety 60%, immutability 20%, adjustment 20%）
- `MigrationManager`（197 行）— 基因组迁移 + 语义版本号管理
- `RollbackManager`（80 行）— 快照栈，支持回滚
- `LineageTracker`（80 行）— 版本谱系追踪
- `SpecEditor`（183 行）— GenomePatch 操作（Add/Remove/Modify/Replace）
- `CandidateGenerator`（97 行）— 将 MutationIntent 应用到基因组
- `MutationIntentGenerator`（69 行）— 基于关键词的变异意图生成
- `SandboxRunner`（122 行）— **实际执行 `cargo test --workspace`** 并解析 JSON 结果
- `MutationExecutor`（56 行）— 批量执行变异意图
- `SelfReader`（95 行）— 从 SelfField 读取当前状态
- `RuntimeBuilder`（42 行）— 构建运行时候选

**真实差距：**
- `bridge/mod.rs` — 空模块
- `LineageTracker` 存储是纯内存的，进程重启丢失
- `MutationIntentGenerator` 是关键词启发式，不是真正的学习驱动
- `CandidateGenerator` 只处理 `care.priorities` 变异，boundary 和 mutation 需要手动审批
- 整个 pipeline 没有被 runtime 主循环调用——它是"可以跑"但"没在跑"

**评价：** Meta 是惊喜。在 2,115 行内实现了完整的自演化闭环，而且零 stub。`SandboxRunner` 真的跑 `cargo test`，`Evaluator` 真的做安全检查，`MigrationManager` 真的写盘。问题只是：它没有被集成到运行时主循环中。

---

### 2.9 aletheon-meta — 深入：Morphogenesis Pipeline

这是整个项目中最接近"自演化"概念的代码。值得单独分析：

```
MutationIntentGenerator（关键词 → 意图）
        ↓
CandidateGenerator（意图 → 基因组变异候选）
        ↓
SandboxRunner（cargo test --workspace）
        ↓
Evaluator（安全评估 → Adopt/Reject/NeedsMoreTesting）
        ↓
MigrationManager（写盘 + 版本号 + 谱系记录）
        ↓
RollbackManager（快照，失败可回滚）
```

**这个 pipeline 是端到端可工作的。** 但有两个关键缺口：
1. **输入端**：MutationIntentGenerator 是关键词匹配（"fail" → 增加 safety 权重），不是从反思/记忆/学习中驱动的
2. **集成端**：没有被 ReAct loop 或 Orchestrator 调用——它是独立的，需要手动触发

---

### 2.10 binaries — 入口点（387 行）

**完成度：★★☆☆☆（30%）**

| 二进制 | 行数 | 状态 |
|---|---|---|
| `aletheond` | 34 | 极简入口，启动 daemon |
| `aletheon-cli` | 9 | 几乎空的 CLI |
| `aletheon-exec` | 344 | 单次执行模式，有 LLM 调用和工具执行 |

**评价：** 入口点是项目最薄的部分。`aletheond` 只是 daemon 的壳，`aletheon-cli` 几乎是空的。`aletheon-exec` 是唯一有实际逻辑的入口。

---

### 2.11 examples（356 行）

- `basic-agent` — 基础 agent 示例
- `self-evolution-loop` — 自演化循环示例

**评价：** 示例代码量少，但能编译通过。

---

## 3. 哲学-代码映射深度分析

这是你最关心的部分。你参考了斯宾诺莎、海德格尔、胡塞尔，并把它们映射到了代码结构。下面逐个评估映射的深度。

### 3.1 斯宾诺莎的 conatus（自我保存倾向）→ `CareLayer`

**概念：** 每个存在物都在努力维持并表达自己的存在。conatus 不是 agent 拥有的属性，而是 agent 之为 agent 的本质。

**代码映射：** `aletheon-self/src/core/care.rs` — CareLayer 维护一个 `HashMap<String, f64>` 主题 → 权重映射。

**深度评估：★★★☆☆**

映射方向正确——care 权重确实是 agent "在意什么"的量化。但斯宾诺莎的 conatus 有两层含义：
1. **维持自身存在** — 这对应安全边界，你用 `BoundaryLayer` 做了
2. **表达自身本质** — 这对应 agent 的主动行为倾向，你用静态权重做了

**缺什么：** conatus 是**动态的**——它会根据经验改变。一个 agent 如果反复在某个任务上失败，它的 conatus 会驱使它避开那个方向或改变策略。你的 care 权重是配置文件里的静态值，不会根据运行时经验自调整。

**如何改进：** 让 `CareLayer` 的权重从 `AwarenessGrowthAnalyzer` 的分析结果中动态更新。这才是 conatus 的真正体现——agent 通过经验改变自己"在意什么"。

---

### 3.2 斯宾诺莎的 idea ideae（观念的观念）→ `SelfAwareness` seed

**概念：** 观念的观念内在于每个观念本身。当 mind 有观念 X 时，它同时有"我知道我有 X"。自反性不是事后附加，而是内在于每个心智活动。

**代码映射：** `aletheon-self/src/bridge/llm_bridge.rs` — 每个 action 生成一个 `SelfAwareness` struct，包含 intent、self_state、significance、extension。

**深度评估：★★☆☆☆**

方向对，但实现方式与哲学原意有根本偏差。

斯宾诺莎的 idea ideae 说的是：**觉知是内在的，不需要外部注入。** 你当前的实现是：
1. 执行一个 action
2. **调 LLM 生成**一个 self-awareness text
3. 把结果存到 `SelfAwareness` struct

这是"外部注入觉知"，不是"内在觉知"。真正的 idea ideae 应该是：每个心智活动本身携带自我给予性——不需要第二层行为来"意识到"第一层。

**如何改进：** 这是一个哲学上最难实现的概念。一个务实的方向是：让每个工具执行的结果自动携带元数据（"我在执行什么"、"为什么执行"、"执行结果如何影响我的状态"），而不是事后调 LLM 生成。`ToolResultMeta` 已经在做这件事（耗时、成功/失败），可以扩展它携带更多"自我给予"的信息。

---

### 3.3 胡塞尔的 pre-reflective self-awareness → SelfAwareness 内在于每个 action

**概念：** 意识总是自我给予的，不需要第二层反思行为。

**代码映射：** `SelfAwareness` 作为每个 action 的伴随结构。

**深度评估：★★☆☆☆**

与 idea ideae 的问题相同。当前实现是"每个 action 后面跟一个 LLM 调用来生成 awareness"，这是 reflective（反思的），不是 pre-reflective（前反思的）。

**一个可能的方向：** 在 `ToolContext` 中嵌入当前 agent 状态的快照（identity、care priorities、attention focus），让每个工具执行天然携带"agent 此刻是什么状态"的信息。这不是"反思"，而是"工具执行时自然知道 agent 是谁"——更接近 pre-reflective 的含义。

---

### 3.4 海德格尔的 Sorge（关怀/牵挂）→ SelfField 三层结构

**概念：** Dasein 的存在结构是"先行于自身—已经在世界中—寓于世内存在者"，时间性贯穿其中。

**代码映射：**
- "先行于自身" → `ContinuityLayer`（跨会话连续性）
- "已经在世界中" → `PerceptionBridge`（系统感知）
- "寓于世内存在者" → `CareLayer`（对具体事物的关怀）

**深度评估：★★★★☆**

这是映射最好的一个。三层 SelfField 确实覆盖了 Sorge 的三个维度。`ContinuityLayer` 的检查点机制对应"先行于自身"（面向未来），`PerceptionBridge` 对应"已经在世界中"（被抛入），`CareLayer` 对应"寓于物"（与世内存在者打交道）。

**缺什么：** 海德格尔的 Sorge 核心是**时间性**——Dasein 的存在是时间性的，面向死亡的。你的实现缺"有限性"意识——agent 不知道自己会"死"（被关闭、被替换），因此没有"向死而生"的紧迫感。

**如何改进：** 一个有趣的方向是让 agent 意识到自己的 session 有限性（"我可能在 N 小时后被关闭"），并据此调整行为优先级。这在技术上对应 `ContinuityLayer` 的 TTL 概念。

---

### 3.5 海德格尔的 Dasein（此在）→ 整个 Agent 设计

**概念：** Agent 不是一个"东西"（Vorhandenheit），而是一个持续存在的"此在"（Dasein）。

**代码映射：** README 的核心理念 — "An Agent that is not merely executed, but continuously exists."

**深度评估：★★★★★**

这是整个项目哲学设计中最深刻的一点。你没有把 agent 设计成"一个 API 调用"或"一个 prompt"，而是设计成一个**持续存在的运行时**——有身份、有记忆、有边界、有生命周期。这确实是 Dasein 的技术体现。

**关键洞察：** 大多数 agent 框架（LangChain、AutoGPT）把 agent 当作"函数调用链"。你把它当作"持续存在的实体"。这个架构决策本身就是海德格尔式的。

---

### 3.6 哲学映射总评

| 哲学概念 | 代码映射 | 深度 | 核心差距 |
|---|---|---|---|
| Spinoza **conatus** | `CareLayer` | ★★★ | 权重静态，不从经验中自调整 |
| Spinoza **idea ideae** | `SelfAwareness` seed | ★★ | 外部注入觉知，非内在自给予 |
| Husserl **pre-reflective** | action 伴随 awareness | ★★ | 同上，是 reflective 不是 pre-reflective |
| Heidegger **Sorge** | SelfField 三层 | ★★★★ | 缺时间性和有限性意识 |
| Heidegger **Dasein** | 整体架构设计 | ★★★★★ | 最深刻的映射，"持续存在"的架构决策 |

---

## 4. 跨系统集成评估

### 4.1 数据流分析

```
用户输入
    ↓
ReActLoop (runtime)
    ↓ 调用
Reasoner (brain) → LLM API → 输出推理链
    ↓
Planner (brain) → 生成步骤
    ↓
ToolCallExecutor (body) → 并行/串行执行工具
    ↓ 结果存入
EpisodicMemory (memory)
    ↓ 反思触发
Reflector (brain) → 产出 Reflection
    ↓ 存入
ReflectionEvents (memory)
    ↓ 进化触发
MutationIntentGenerator (meta) → MorphogenesisPipeline
    ↓
Genome 变异 → SandboxRunner → Evaluator → Migration
```

### 4.2 集成完成度

| 集成点 | 状态 | 说明 |
|---|---|---|
| Runtime → Brain (LLM 调用) | ✅ 已连接 | ReActLoop 调用 LlmBridge |
| Runtime → Body (工具执行) | ✅ 已连接 | ReActLoop 调用 ToolCallExecutor |
| Runtime → Memory (存储) | ✅ 已连接 | 执行结果存入 EpisodicMemory |
| Brain → Self (觉知) | ⚠️ 部分连接 | SelfAwareness 生成在 SelfField 中，但 Brain 不读取它来影响推理 |
| Self → Brain (影响决策) | ❌ 未连接 | CareLayer 的权重不影响 Reasoner 的推理策略选择 |
| Memory → Brain (影响推理) | ❌ 未连接 | 记忆召回结果不注入推理上下文（只有直接 API 调用） |
| Brain → Meta (触发进化) | ❌ 未连接 | Reflector 的反思不触发 MorphogenesisPipeline |
| Meta → Runtime (应用变异) | ❌ 未连接 | 基因组变异不改变运行时行为 |
| Perception → Runtime (事件驱动) | ❌ 未连接 | 系统感知不触发 agent 行为 |

**关键发现：** 当前的集成是**单向管道**——Runtime 调 Brain 调 Body 存 Memory。但反向的影响链（Memory 影响 Brain，Brain 触发 Meta，Meta 改变 Runtime）全部断开。

---

## 5. 当前完成度评级

### 按维度评分

| 维度 | 分数 | 说明 |
|---|---|---|
| **类型系统设计** | 9/10 | ABI 层非常干净，跨 crate 类型一致性好 |
| **工具系统** | 9/10 | 20+ 工具全部真实可工作，执行器有并发控制 |
| **通信层** | 8/10 | EventBus + IPC + PubSub 全链路，io_uring 是唯一空壳 |
| **记忆系统** | 8/10 | SQLite 四层后端 + FTS5，decay/activation 未接入 |
| **自我建模** | 6/10 | 8 层结构完整，但每层深度不够 |
| **认知推理** | 5/10 | 结构有，但"推理"本质是调 API |
| **自演化** | 6/10 | Pipeline 端到端可工作，但未集成到主循环 |
| **感知层** | 2/10 | eBPF/FUSE/屏幕感知全部是设计文档 |
| **运行时集成** | 4/10 | 正向管道通，反向影响链全断 |
| **哲学深度** | 7/10 | Dasein 映射深刻，idea ideae 映射偏表面 |

### 综合评级

**整体完成度：30-40%**

骨架和肌肉有了（类型系统、工具、通信、记忆存储），但神经系统（真正的推理、学习、自演化集成）和感知系统（eBPF、FUSE）还是空的。

最核心的 gap：**当前系统是一个"非常聪明的工具执行器"，但还不是"一个持续存在、自我演化的实体"。** 它能执行任务、存储结果、运行反思模板，但它不会因为经验而改变自己的行为——care 权重不自调，reasoning 策略不自选，genome 不自变异。

---

## 6. 改进方向与优先级

### P0：让"自演化"跑起来（最高优先级）

这是项目的核心愿景，也是当前最大的缺口。

**具体任务：**
1. **连接 Reflector → MutationIntentGenerator** — 让反思结果自动产生变异意图，而不是关键词匹配
2. **连接 MorphogenesisPipeline → ReActLoop** — 在每个 session 结束时自动触发一次演化循环
3. **连接 Genome → Runtime 行为** — 让基因组中的 `reasoning_config` 和 `care.priorities` 实际影响运行时决策
4. **让 LineageTracker 持久化** — 当前是纯内存，重启丢失。写入 SQLite 或文件

**效果：** Agent 执行任务 → 反思 → 产生变异意图 → sandbox 测试 → 评估 → 迁移 → 下次行为改变。这是 conatus 的真正体现。

### P1：让"记忆"影响"推理"

**具体任务：**
1. **接入 decay/activation 到 recall 管道** — 让 recall 结果按 activation 分数排序，而不是 SQL 排序
2. **将记忆召回注入推理上下文** — ReActLoop 在调 Reasoner 之前，先从 Memory 中召回相关记忆，注入 system prompt
3. **实现 L2→L3 consolidation** — 定期将高频访问的短期记忆巩固为长期记忆
4. **接入 LanceDB 或 Qdrant** — 向量相似性搜索，让记忆召回从"关键词匹配"升级到"语义搜索"

**效果：** Agent "记得"之前做过什么、成功/失败的经验，并据此调整推理。这是从"无状态工具"到"有记忆的实体"的关键一步。

### P2：让"自我"影响"行为"

**具体任务：**
1. **CareLayer 权重动态化** — 从 AwarenessGrowthAnalyzer 的分析结果更新权重
2. **SelfField 状态注入 Reasoner** — 让 Reasoner 知道当前的 identity、care priorities、boundary rules
3. **扩展 ToolResultMeta** — 让每个工具执行自动携带"agent 此刻是什么状态"的信息（接近 pre-reflective self-awareness）

**效果：** Agent 的"在意什么"和"我是谁"实际影响它的推理和行为。

### P3：感知层最小切口

**具体任务：**
1. **inotify 文件监控** — 监控指定目录的文件变更，产生事件
2. **/proc 系统状态轮询** — 定期读取 CPU/内存/进程状态
3. **journald 日志尾随** — 监控系统日志中的错误/警告

**效果：** Agent 从"被动等待输入"变成"主动感知环境"。不需要 eBPF，inotify + /proc 就够了。

### P4：Brain 的推理深度

**具体任务：**
1. **Reasoner 支持 self-consistency** — 采样多次推理，投票选最优
2. **Reflector 做真正的因果分析** — 不只是"成功/失败"模板，而是分析哪些步骤导致了什么结果
3. **WorldModel 维护状态** — 追踪环境状态变化，支持预测

**效果：** 从"调 API 拿输出"到"真正的推理"。

---

## 7. 工作量估算

| 优先级 | 任务 | 估算工作量 | 难度 |
|---|---|---|---|
| **P0** | Reflector → MutationIntent 连接 | 2-3 天 | 中 |
| **P0** | MorphogenesisPipeline 集成到主循环 | 3-5 天 | 中高 |
| **P0** | Genome → Runtime 行为映射 | 2-3 天 | 中 |
| **P0** | LineageTracker 持久化 | 0.5 天 | 低 |
| **P1** | decay/activation 接入 recall | 1-2 天 | 中 |
| **P1** | 记忆注入推理上下文 | 2-3 天 | 中 |
| **P1** | L2→L3 consolidation | 2-3 天 | 中 |
| **P1** | LanceDB 接入 | 3-5 天 | 中高 |
| **P2** | CareLayer 动态权重 | 1-2 天 | 低中 |
| **P2** | SelfField 注入 Reasoner | 1-2 天 | 低中 |
| **P2** | ToolResultMeta 扩展 | 1 天 | 低 |
| **P3** | inotify 文件监控 | 2-3 天 | 中 |
| **P3** | /proc 系统轮询 | 1-2 天 | 低 |
| **P3** | journald 日志尾随 | 1-2 天 | 低中 |
| **P4** | Reasoner self-consistency | 3-5 天 | 高 |
| **P4** | Reflector 因果分析 | 3-5 天 | 高 |
| **P4** | WorldModel 状态追踪 | 5-7 天 | 高 |

**P0 总计：约 8-12 天**
**P0+P1 总计：约 16-25 天**
**P0-P4 全部：约 35-55 天**

---

## 附录：代码质量观察

### 好的方面
- **零 panic unwrap** — 大部分代码用 `?` 或 `anyhow::Context` 错误传播
- **测试覆盖好** — 1215 个测试，217 个文件有测试
- **类型安全** — Rust 的类型系统被充分利用，trait 抽象合理
- **无技术债标记** — 几乎没有 TODO/FIXME，代码是"写完"的状态
- **并发控制专业** — `PathConflictDetector`、per-path semaphore、CancellationToken

### 需要注意的
- **部分 crate 有重复** — `unix_socket_transport.rs`（352 行）和 `ipc/unix_socket.rs`（333 行）结构几乎一样，可以合并
- **测试中 `unimplemented!()` 的使用** — 4 处在 ReActLoop 测试中，1 处在 executor 测试中，都是 mock 辅助函数
- **bridge/mod.rs 空模块** — comm 和 meta 各有一个，应该删除或实现

---

*评估完成。下一步建议：先做 P0（让自演化跑起来），这是项目愿景的核心。*

# Aletheon 架构稳定性、权威收敛与宏内核优化计划

> 文档类型：架构治理与迁移计划
> 基线日期：2026-08-04
> 审计对象：`Aurobear/aletheon`；**基线 SHA 见 `Aletheon_Unified_Execution_Plan_2026-08-04.md` §0（唯一声明处）**，本文件不再自行声明 SHA
> 基线规则：所有“当前代码”判断均以统一计划 §0 的 `PLAN_BASELINE_SHA` 为准；执行前若 `origin/dev` 含未登记提交，先按统一计划 §0 的 external-change reconciliation 更新，不得沿用相对提交数
> 状态：`revised-for-owner-approval`；统一计划 B0/B1 完成、owner approval 入库且 X0 baseline reconciliation 通过前不得批量实现
> **执行权威**：节点顺序、依赖、状态词表与完成判定以 `Aletheon_Unified_Execution_Plan_2026-08-04.md`（§3 节点清单 / §4 验收 ID 归属 / §6 关键裁决 / §7 状态词表）为准。
> **DO-NOT-SCHEDULE**：本文件 §7 的 P0–P10 表**仅为历史记录，禁止作为调度输入**；新旧 ID 映射见统一计划 §5。deepseek C-plan 保留自己的 C0–C7 编号，不在 X 轨道 DAG 内调度（约束见统一计划 §1.1/§1.2）。
> 关联计划：`Aletheon_User_Experience_and_Engineering_Workflow_Plan_2026-08-04.md`（消费本计划契约）、`deepseek-cache-and-message-optimization-plan.md`（provider/cache 事实 owner）、`robot-vla-production-closure-plan.md`（robot 域关闭 owner）；边界见 §0.2
> 核心原则：不推倒重写，不拆微服务，不继续横向增加概念；通过唯一权威、依赖方向和迁移删除恢复架构稳定性

---

## 0. 执行摘要

Aletheon 已经完成一轮核心架构解耦；完成报告将 architecture checks、negative fixtures、workspace checks/tests 和 public API contraction 标记为通过（`docs/arch/CORE_REFACTOR_COMPLETION_REPORT.md:36-78`），crate/module、wire、persistence、config、Executive layer 和 compatibility debt 均已有机器可读台账及 CI 棘轮。当前剩余问题不是重新建立这些边界，而是在既有门禁上继续收敛用户命令、Session/Task/Turn 投影、Capability 副作用链和安装后产品体验。

目前已经形成 16 个 workspace crate（`config/architecture/module-boundaries.txt:3-20`）加 assembly 和 examples。工具、记忆、硬件、runtime、TUI、daemon、session、事件、计划、审批等能力都在增长，但存在以下结构性风险：

1. 安装 binary 已有唯一 parser，但兼容 parser 文件同时托管了权威 parser 依赖的类型，删除前必须先迁移该类型（详见 ARCH-ENTRY-01）。
2. assembly、adapter、application、domain 已有门禁；新计划只能补充缺失规则，不能另建第二套架构台账。
3. Session/Event/Turn/Plan/Memory/Agent 状态可能由不同模块分别投影或维护。
4. 兼容层长期存在，变成第二套生产路径。
5. UI 和 handler 容易直接拼装内部能力。
6. 功能通过新增模块完成，却没有删除被替代实现。
7. “类型/单测存在”“架构门禁通过”“安装后主链可用”必须作为三个独立完成层级。

### 0.1 已完成基线与剩余增量

以下现状是本计划的输入，不是待重做任务：

| 已有能力 | 权威位置 | 本计划允许的动作 |
|---|---|---|
| crate/module ownership inventory | `config/architecture/module-boundaries.txt:1-25` | 复核并增量更新 |
| Executive layer inventory | `config/architecture/executive-layers.tsv:1-352` | 新增/移动文件时同步更新 |
| wire/persistence inventory | `config/architecture/wire-surfaces.tsv:1-20`、`config/architecture/persistence-surfaces.tsv:1-18` | 为新契约登记版本和迁移规则 |
| compatibility debt ratchet | `config/architecture/compatibility-debt.tsv:1-4` | 只允许下降；删除前保留数据证据 |
| architecture metric baseline | `config/architecture/metrics.env:1-10` | 只允许下降；P0 必须刷新 `frozen_commit` |
| architecture checker/negative fixtures | `scripts/libexec/aletheon/architecture-check.sh:62-171`、`tests/suites/architecture/architecture_check.sh:76-114,201-229` | 扩展 single-parser、ID、writer 和 transaction 规则 |
| Turn Pipeline lifecycle owner | `config/architecture/state-machine-inventory.tsv:3` | 保持生命周期权威；Cognit 只拥有推理策略 |
| 现有 `TurnCheckpoint` | `crates/fabric/src/types/workspace_checkpoint.rs:55-74` | 兼容扩展或新建 projection；禁止重复定义同名结构 |

本计划的增量范围只有：

```text
兼容 parser/API 删除
  -> Command/Intent 语义收敛
  -> Session/Task/Activity projection
  -> Capability/transaction/recovery 补强
  -> UX 主链和安装后验收
```

本计划保留以下既定方向：

```text
单实例
宏内核式
内部模块化
外部执行域隔离
```

并将系统收敛为：

```text
唯一 Client Intent 入口
  -> 唯一 Session / Event Authority
  -> 唯一 Turn Engine
  -> 唯一 Capability Invocation Path
  -> 受监督 Runtime / Tool / Hardware Domain
  -> 唯一 Receipt / Settlement
  -> UI 只做 Projection
```

### 0.2 关联计划与 owner 边界

`docs/plans/` 下存在四份同基线计划。本计划不重复定义它们已拥有的契约，只声明 owner 边界：

| 计划 | 拥有的权威 | 本计划的关系 |
|---|---|---|
| 本文（架构收敛） | 依赖方向、authority matrix、capability envelope、门禁、删除迁移 | 冻结契约，供其余计划消费 |
| `Aletheon_User_Experience_and_Engineering_Workflow_Plan_2026-08-04.md` | 用户命令面、TUI 信息架构、checkpoint/review/settlement 的用户表面 | 消费本计划冻结的契约，不得新建同义结构 |
| `deepseek-cache-and-message-optimization-plan.md`（C0–C7） | provider usage 解析、provider capability/代理差异、稳定前缀 shape、Mnemosyne recall cache、Corpus tool result cache | **C-plan 是 typed provider/cache 事实的唯一写入方**；本计划 §4.11、§10 只消费其输出，不另定义 cache 语义 |
| `robot-vla-production-closure-plan.md`（R0–R8） | Robot/Policy typed config、Perception/FrameRef、VLA gateway、Episode artifact、实机安全 | Phase A9 只验证"robot 域走统一主链"，域内验收由 R-plan 的 R7/R8 关闭 |

冲突消解规则：

- cache/usage 指标语义冲突时以 C-plan 为准，本计划只能收紧治理要求，不能重定义字段。
- robot 域完成判定冲突时以 R-plan 为准；A9 不得声称关闭 R8。
- 任一计划要修改另一计划拥有的契约，必须先在对方文档登记，再在本计划 §7 DAG 上加节点。

### 0.3 已合入工作与 B0/X0 对账

2026-08-05 复核时，原先 14 条 `auro/feat/*`、`auro/feature/*`、`auro/fix/*`、`auro/test/*` 分支均已出现在 `git branch --merged origin/dev`，C0–C7 也已合入 `origin/dev@c080b08b...`。它们不再是可调度的在途工作，也不得重复 rebase/cherry-pick。

| 项目 | 当前事实 | 处理方式 |
|---|---|---|
| C-plan C0–C7 | 全部 `accepted` 并已合入 | 只消费冻结字段，不重新实现 |
| 14 条历史分支 | `already_merged` | X0 保存命令证据，不再逐条选择 merge/rebase/abandon |
| 四份计划 + Robot evidence + status ledger | B0 前未入库 | 按统一计划 §0.1 先完成文档 bootstrap |
| B0 后新出现的生产代码改动 | 未知 | 立即停止，对每项判定 owner；不得混入文档或其他节点 |

X0 的对账退出证据是执行时刻的 `git status --porcelain`、`git branch --merged origin/dev` 与 merge SHA；历史清单只用于防止重复合入。

---

## 1. 不可破坏的架构决策

### 1.1 宏内核方向保持

Aletheon 继续是**单一逻辑权威、模块化宏内核** Agent Runtime，不改成大量按业务领域拆分的 systemd 微服务。“单实例”不表示只有一个 OS 进程；当前 machine core、user daemon 和受监督执行域可以是不同进程，但同一种状态只能有一个权威 owner，进程间必须通过版本化协议、generation/epoch 和 receipt 协调。

宏内核统一负责：

- Process/lifecycle。
- Time/timer/deadline。
- Space/workspace/namespace。
- IPC/message/event delivery。
- Scheduling/supervision。
- Permission/lease/quota/accounting/budget。
- Object identity 与 authoritative receipt。

### 1.2 领域定位保持

| 模块 | 唯一职责 |
|---|---|
| `fabric` | 最小稳定 ABI、ID、协议、跨域 value objects；不放业务编排 |
| `kernel` | 时间、operation/process 基础、监督原语；不理解聊天和 LLM |
| `executive` | 应用编排、生命周期、调度、权限、资源、监督、恢复 |
| `cognit` | Native Cognit 与唯一认知 Turn/Harness 权威 |
| `corpus` | Capability/Tool 实现与执行适配；不拥有 Task/Plan |
| `agora` | 当前活动认知空间、计划图、注意和 scratch；不作为长期历史库 |
| `mnemosyne` | 经验、记忆生命周期、召回和持久知识权威 |
| `dasein` | 身份、长期目标、价值、连续性 |
| `metacog` | 受治理的反思/演化 proposal；不能直接修改核心策略 |
| `gateway` | channel-neutral intent/effect transport；不编排认知 |
| `platform` | 宿主 OS 能力契约与实现 |
| `runtime` | 外部 Agent Runtime manifest、selection、capability contract |
| `hardware` | Hardware permit、safety boundary、receipt、模拟器 |
| `interact` | CLI/TUI adapter 和纯投影；不拥有业务状态 |
| `execd` | 隔离执行域；不拥有任务结算 |
| `aletheon` | assembly/composition root；不放领域逻辑 |

### 1.3 外部 Agent 定位保持

- Aletheon 决策。
- Pi/其他 Coding Runtime 执行。
- GBrain 仅作为 Mnemosyne 管理的 supplemental memory backend；它可以返回外部检索/综合结果，但不拥有 Session、Dasein 或 Mnemosyne 权威状态。
- Native Cognit 是认知权威。
- 外部 Agent 只能是受监督 child process/runtime。
- child 的自然语言“成功”不能推进父任务。
- 父任务只接受结构化 terminal receipt。

### 1.4 机器人边界保持

- Agent 不承担硬实时控制。
- Robot task 必须经过 Hardware Broker/permit/Safety Supervisor。
- 高频遥测在边缘降采样、摘要、归档 Artifact。
- 先 simulator/只读，再 ROS 仿真，最后真实执行器。

---

## 2. 当前结构性问题

### 2.1 多入口与重复控制面

#### ARCH-ENTRY-01：一个生产 parser，一个兼容 parser，一条真实的跨层类型依赖

安装 binary 当前只在 `crates/aletheon/src/main.rs:357` 调用 `Cli::parse()`；`crates/interact/src/tui/cli.rs:228` 的 `Args::parse()` 未进入该安装主链，但仍通过 `crates/interact/src/lib.rs:28` 的 `pub use tui::cli` 公开 re-export，并携带不同命令模型（`debug`、`goal`、`workflow`、`daemon start/stop/status`）。

caller inventory 已在计划复核阶段完成，结论必须分两级处理，不能笼统称为"零调用的 API 债务"：

| 兼容面 | 实测调用点 | 删除前置条件 |
|---|---|---|
| `Args::parse()` 与旧 command handler | 0（仅 `crates/interact/src/tui/cli.rs:228` 自身及同文件单测 `:750-752`） | 无外部阻塞，可直接删除 |
| `interact::cli` re-export 承载的 `TaskKindArg` | **1 个生产调用**：`crates/aletheon/src/main.rs:15` `use interact::cli::TaskKindArg;`，用于 `:66` 的 `--task-kind` 字段与 `:705` 单测 | 必须先把 `TaskKindArg`（定义于 `crates/interact/src/tui/cli.rs:97-107`，含 `From<TaskKindArg> for fabric::TaskKind`）迁至 CommandSpec 权威位置 |

即：权威 parser 目前**反向依赖兼容 parser 文件**中的类型。这是真实的依赖方向缺陷（assembly → presentation 的兼容模块），也是 `caller count=0` 退出证据当前无法达成的原因。删除顺序被强制为"先迁类型，后删 parser"，不得合并为一步。

#### ARCH-ENTRY-02：TUI、line mode、`-m`、`exec` 分别选择请求

不同入口可能构造不同的 session、workspace、task kind、permission 和 RPC。`simple_line_mode` 甚至维护自己的 slash command match。

#### ARCH-ENTRY-03：部署脚本形成旁路控制面

`scripts/aletheon.sh` 是合理的运维 dispatcher，但业务能力不能只存在于脚本或与应用 handler 分叉。脚本应调用稳定 admin contract，而不是成为第二个 Runtime。

---

### 2.2 权威状态未完全收敛

#### ARCH-AUTH-01：Session 存储、事件日志、TUI projection 和 memory 之间边界复杂

必须明确：

- Session authority 是什么。
- Event journal 是否为恢复依据。
- SQLite snapshot 是权威还是缓存。
- Memory observation 是 session event 的派生还是独立写入。
- TUI state 只能否通过 replay 重建。

#### ARCH-AUTH-02：Turn、Task、Goal、Plan、ChangeTransaction 不是同一个生命周期

它们可以是不同对象，但必须通过 ID 和 ownership 形成明确层级，不能由 chat handler 临时拼接。

建议层级：

```text
Session
  -> Task
      -> Turn
          -> Activity / Operation
          -> ChangeTransaction
          -> Checkpoint
      -> TaskSettlement
```

#### ARCH-AUTH-03：Memory 存在多种语义层

Session transcript、event journal、fact、episodic memory、semantic memory、core block、GBrain supplemental source 不能都叫“memory”并允许任意写入。

#### ARCH-AUTH-04：Agent/child runtime 状态容易形成第二权威

外部 runtime 自己的 session、进度和结果必须投影到 Aletheon-owned process/activity/receipt，不得成为父任务唯一恢复依据。

---

### 2.3 Turn 与认知主链风险

#### ARCH-TURN-01：Handler 容易承担过多业务

JSON-RPC/chat handler 不应同时负责：

- session 初始化。
- memory recall。
- prompt composition。
- runtime selection。
- Cognit 调用。
- tool loop。
- plan 更新。
- event emission。
- settlement。

Handler 只应验证 intent、调用 application use case、返回 typed response。

#### ARCH-TURN-02：多种 Harness/Loop 可能各自完成一套 Turn

Linear ReAct、Robot Harness、外部 Coding Runtime、specialized workflow 可以有不同策略，但必须实现同一个 `CognitiveSession/TaskExecutor` 生命周期，不能各自产生不兼容的完成语义。

#### ARCH-TURN-03：模型输出与 Host 状态容易混合

模型输出只能提出：plan proposal、tool call、finding、answer。Host 才能写入 permission、transaction、validation、settlement。

---

### 2.4 Capability 与执行路径风险

#### ARCH-CAP-01：工具很多，但调用治理可能分散

普通 tool、managed command、MCP、Skill、external runtime、hardware action 必须共享 capability invocation envelope：

```text
resolve
  -> authorize
  -> reserve budget/lease
  -> execute
  -> observe events
  -> produce terminal receipt
  -> account/settle
```

#### ARCH-CAP-02：普通 Bash 与 managed command 语义重叠

`bash_exec` 适合短命令，`managed_command` 适合持续、可写 stdin、可取消、可恢复观察的任务。选择规则必须由 capability policy 决定，不能只由模型根据两个相似 tool description 猜测。

#### ARCH-CAP-03：ChangeTransaction 可能只被部分修改路径使用

所有修改型能力——`file_write`、`apply_patch`、structured patch、shell side effect、external coding runtime——都必须进入同一 transaction 或明确声明 outside-transaction 并被拒绝/审批。

#### ARCH-CAP-04：Receipt 类型过多但缺少统一封装

ToolResult、patch receipt、validation receipt、runtime receipt、EpisodeReport 应有统一 envelope，但不应强行抹平各领域 payload。

---

### 2.5 依赖方向与 crate 边界风险

#### ARCH-DEP-01：`fabric` 容易膨胀为共享垃圾场

跨 crate 类型都往 `fabric` 放，会使所有模块被迫一起变化。只有稳定 ID、协议 envelope、权限/receipt 等真正跨域 value object 才能进入。

#### ARCH-DEP-02：`executive` 容易成为上帝模块

Executive 应编排 use case，但不能重新实现 Cognit、Mnemosyne、Corpus 或 Hardware 领域规则。

#### ARCH-DEP-03：`interact` 仍含兼容 CLI 和业务分支

UI adapter 中存在 command parsing、不同 request selection、session command 行为，会逐渐形成 presentation-owned application logic。

#### ARCH-DEP-04：assembly 入口容易积累业务

`crates/aletheon/src/main.rs` 已包含多个命令和配置 handler。继续增加会让 composition root 变成新的 monolith。

#### ARCH-DEP-05：兼容 re-export 长期存在

例如 `pub use tui::cli`。兼容层若没有删除日期、调用计数和 CI gate，就会永久成为生产 API。

---

### 2.6 测试与“稳定”定义风险

#### ARCH-TEST-01：类型存在和单测通过被等同于能力完成

README 的 Stable/Done 必须同时要求：唯一生产路径、端到端 fixture、失败证据、恢复证据。

#### ARCH-TEST-02：已有 architecture tests，但缺少本轮专项门禁

现有 checker 已覆盖 crate inventory、Executive layer、wire/persistence 登记、部分 forbidden imports、依赖增长和兼容债务棘轮（`scripts/libexec/aletheon/architecture-check.sh:62-171,184-243`）。本轮只能在原 checker/fixture 上补充：

- `interact -> corpus` 直接依赖。
- `cognit -> executive` 反向依赖。
- domain crate 依赖 assembly。
- hardware adapter 绕过 permit。
- TUI 直接访问 DB/store。
- 生产 parser/dispatcher 数量增长。
- 同语义 ID wrapper 重复增长。
- Session append authority 或 mutation writer 绕行。

禁止新建第二份 architecture checker、第二套 ownership inventory 或平行 acceptance 命令。

#### ARCH-TEST-03：fallback 可能静默

任何 fallback 必须产生 event/metric/receipt；不能回退到 legacy path 后仍报告正常生产路径。

---

## 3. 目标架构

### 3.1 逻辑层次

```text
Presentation Adapters
  interact CLI/TUI, gateway transports
        |
        v
Application Use Cases
  executive commands, task lifecycle, recovery, settlement
        |
        v
Domain Authorities
  cognit, agora, mnemosyne, dasein, hardware, runtime policy
        |
        v
Capability / Infrastructure Adapters
  corpus tools, platform, MCP, execd, providers, external runtimes
        |
        v
Kernel Primitives and Stable ABI
  kernel + fabric
```

依赖只能向下。Domain 通过 port 接受基础设施实现；不得反向 import adapter。

### 3.2 唯一请求主链

```text
CLI/TUI/Gateway
  -> ClientIntent
  -> Executive ApplicationCommand
  -> SessionAuthority.append(IntentAccepted)
  -> TaskController
  -> Cognit CognitiveSession
  -> CapabilityGateway
  -> Tool/Runtime/Hardware executor
  -> ActivityEvent + DomainReceipt
  -> Validation/Settlement
  -> SessionAuthority.append(TaskSettled)
  -> Projection
  -> CLI/TUI/Gateway
```

### 3.3 Authority Matrix

| 对象 | 唯一写入权威 | 其他模块权限 |
|---|---|---|
| Session/Event journal | Executive SessionAuthority | read/projection/append intent through port |
| Task lifecycle/settlement | Executive TaskController | propose/observe |
| Turn lifecycle/state transition | Executive `TurnPipelineLifecycle` | Cognit/Runtime 提交 typed observation，不直接结算 |
| Turn reasoning/tool-selection strategy | Cognit | Executive 启动/取消/监督并拥有 terminal lifecycle |
| Active Plan graph | Agora | Cognit propose/update through port；UI read-only |
| Identity/long goals | Dasein | Cognit read/propose；Executive enforce boundary |
| Durable experience/memory | Mnemosyne | hooks submit observation；recall through query port |
| Tool implementation | Corpus | CapabilityGateway 调用 |
| Permission/lease/budget | Executive + Kernel primitives | domain request，不直接修改 |
| External child process | Executive supervision | runtime adapter executes |
| Hardware safety decision/domain receipt | Hardware | Cognit proposes，Executive admission/supervision，Safety authorizes |
| UI state | projection only | 不得成为业务恢复权威 |

### 3.4 ID 与对象层级

目标是统一以下稳定 ID 的**语义和父子字段**，不是一次性增加一批新 wrapper：

- `SessionId`
- `TaskId`
- `TurnId`
- `OperationId`
- `ActivityId`
- `TransactionId`
- `CheckpointId`
- `RuntimeProcessId`
- `ReceiptId`
- `ArtifactId`

禁止使用字符串拼接 ID 隐式表达父子关系；父子关系必须是字段。

执行前必须先生成 ID migration matrix。`OperationId` 的重复已经定案，不再作为待确认项：

| 类型 | 定义 | 内层表示 | 结论 |
|---|---|---|---|
| `fabric::OperationId` | `crates/fabric/src/types/operation.rs:9` | `Uuid` | 保留为跨域稳定 ID |
| `hardware::OperationId` | `crates/hardware/src/device.rs:8` | `String` | 语义与表示均不同，**禁止合并**；重命名为领域专属名（如 `DeviceOperationId`）并登记 wire/persistence 影响 |

内层表示不同即证明二者不是同一语义，因此该项的动作是重命名而非统一。P1 的 duplicate-ID 检测必须把"同名不同 crate 且内层类型不同"列为直接失败，而不是提示。

`TaskId`、`ActivityId`、`RuntimeProcessId`、`ReceiptId` 等若当前不存在，应在有至少一个真实跨域契约消费者时引入，禁止先造全套空类型。

### 3.5 统一 Capability Envelope

```text
CapabilityInvocation {
  invocation_id
  session/task/turn/activity ids
  capability_id + version
  actor/runtime identity
  workspace authority
  permission subject
  budget reservation
  deadline/cancellation
  idempotency key
  input payload
}

CapabilityReceipt {
  invocation_id
  terminal status
  started/finished timestamps
  resource accounting
  mutation summary
  artifact refs
  domain payload
  error classification
}
```

统一 envelope，不统一所有 domain payload，并遵守以下反统一规则：

- public Session/Turn event、runtime progress、provider stream、tool result、hardware telemetry 使用不同 schema。
- `accepted/queued`、`started`、`cancel_requested` 不是 terminal success。
- async capability 必须观察权威 terminal snapshot、terminal event 或 durable receipt 后才能结算。
- cancellation 分为 requested、acknowledged、terminal；超时不能伪装成 cancelled。
- envelope 只承载治理元数据和引用；大输出、遥测和二进制进入 Artifact。
- provider inference rounds、provider retries、tool calls、terminal tool results、active context、累计 usage/cache usage 分开统计，禁止互相推导。

---

## 4. 模块级优化方案

### 4.1 `crates/aletheon`

目标：纯 composition root。

保留：

- Clap adapter 初始化。
- config/bootstrap。
- tracing。
- dependency wiring。
- process exit mapping。

迁出：

- config 业务 handler。
- doctor 业务逻辑。
- memory command 业务逻辑。
- extension command 业务逻辑。
- daemon/core 具体启动决策。

这些进入 Executive application use case 或对应 adapter。

### 4.2 `interact`

目标：纯用户 adapter + projection。

必须删除：

- 第二套 Clap parser。
- line mode 自己维护的 slash match。
- 直接推断 command availability。
- presentation-owned session 业务状态。

保留：

- input/editor。
- rendering。
- keyboard/mouse mapping。
- typed ClientIntent transport。
- daemon snapshot/event reducer。

### 4.3 `executive`

目标：应用编排而非领域大杂烩。

内部按 use case 分区：

```text
application/
  command_dispatch
  session
  task
  turn
  capability
  validation
  recovery
  admin
```

禁止增加宽泛 `manager.rs`。每个 use case 明确输入、port、输出、事件和事务边界。

Executive 不能：

- 解析 LLM reasoning。
- 实现 memory ranking。
- 实现 tool 业务。
- 实现 robot verification 规则。

### 4.4 `cognit`

目标：唯一 Native Cognit Turn 权威。

- 统一 `CognitiveSession` 生命周期。
- Linear、Robot、external-assisted workflow 实现相同 stop/cancel/event contract。
- Plan proposal 通过 Agora port。
- Tool call 只通过 TurnServices/CapabilityGateway。
- 不直接访问具体 Corpus tool。
- 不直接写 session store。

### 4.5 `corpus`

目标：能力实现，不拥有任务。

- 工具注册和 exposure policy 分离。
- mutation tool 统一要求 ChangeTransaction context。
- `bash_exec` 与 `managed_command` 建立明确 selection policy。
- Git、patch、file write、external runtime mutation 统一 workspace version。
- ToolResult 适配到 CapabilityReceipt。
- 工具不得自行把任务标记为完成。

### 4.6 `mnemosyne`

目标：唯一记忆生命周期权威。

明确分层：

```text
Session/Event evidence     Executive authority
Observation intake        Mnemosyne governed input
Episodic experience       Mnemosyne
Facts/semantic knowledge  Mnemosyne
Core identity memory      Dasein-authorized, Mnemosyne stored
GBrain                    supplemental backend, not authority
```

所有写入必须有 provenance、scope、sensitivity、lifecycle receipt。

### 4.7 `agora`

目标：只保存活动认知状态。

- Plan graph、attention、scratch、task working set。
- 不保存长期 transcript 真相。
- 重启恢复来自 event/snapshot reconstruction。
- finding 能通过 typed proposal 修改 plan graph。

### 4.8 `runtime`

目标：外部 Agent Runtime 契约。

- manifest/selector 只是选择，不拥有 process lifecycle。
- Executive 创建 child process record、lease、budget。
- Runtime adapter 返回 receipt 和 artifact。
- external session id 只是 metadata。
- child success 必须由 Host acceptance 校验。

### 4.9 `hardware`

目标：领域化的外部执行域安全边界。

- permit、safety decision、execution receipt、EpisodeReport 保留领域语义。
- 通过统一 Capability envelope 接入，但不退化成普通 shell tool。
- Robot Harness 的 completed 必须由 verifier 决定。

### 4.10 `fabric`

目标：最小稳定 ABI。

进入标准：

1. 至少两个独立 owner 真实共享同一个稳定 wire/ABI 语义，或该类型已是外部协议边界；crate 数量本身不是充分条件。
2. 语义稳定，不依赖 application policy。
3. 可独立序列化/版本化。
4. 有兼容策略和 owner。

不满足则留在领域 crate。

### 4.11 Provider Runtime 与模型事实

目标：保持现有机器/provider 级 admission、cooldown 和观测语义，并把它们接入任务投影，而不是新增 session-local 限流器。

owner 边界（见 §0.2）：provider usage 解析、provider capability 差异、prompt 稳定前缀与各级 cache 的**字段定义与写入**属于 C-plan。本节只规定治理约束和投影要求；出现语义分歧时以 C-plan 为准，本计划不得重定义 cache 字段。

- provider/model/context capacity 来自 effective configuration 或 typed host state，不能来自模型自报。
- 同 provider key 的 concurrency、pacing、`Retry-After` 和 circuit health 由 machine core 权威协调。
- 外部 Agent Runtime 的 provider 请求也必须接入同一机器级权威；未接入前标记为已知缺口。
- provider failure 必须以 typed failure 到达 TUI/CLI/settlement；最终自然语言回答不能覆盖失败状态。
- cache read/write/unknown、active context occupancy、累计 billed usage 分开投影。

### 4.12 Recovery、fencing 与版本偏差

任何可恢复状态机必须定义：

- daemon/core generation 或 epoch，旧 writer/旧 child 不得在新 generation 继续结算。
- client/daemon/core protocol version handshake 和不兼容时的 fail-closed 行为。
- stale socket、重复启动、多 client 并发启动的锁和 readiness 语义。
- child 已产生外部副作用但 receipt 未到达时的 orphan reconciliation。
- idempotency 只能防止已知重复，不能把跨进程 exactly-once 当成既成事实。
- wall-clock 回拨不改变 monotonic deadline；重启后 deadline 采用持久化策略重新计算并记录原因。
- schema upgrade 前备份，升级失败恢复；不承诺未经设计的 schema downgrade。

### 4.13 Mutation coverage 等级

ChangeTransaction 不得声称天然覆盖任意 shell 副作用。每个修改能力声明：

| 等级 | 含义 | 允许结算 |
|---|---|---|
| `full` | workspace 内目标、内容、metadata、版本均可捕获并原子恢复 | 可自动 Accept/Repair/Rollback |
| `best_effort` | 可检测主要 changed paths，但 binary、权限、submodule 或外部进程可能不完整 | 需风险提示和显式审批 |
| `non_rollbackable` | workspace 外部、远端或设备副作用无法由文件 checkpoint 回退 | 禁止承诺 rollback；使用领域补偿/人工处置 |

覆盖审计必须包含 untracked/ignored/staged 文件、symlink/hardlink、权限、binary/large files、submodule、workspace 外写入和并发外部修改。当前 checkpoint 的 UTF-8 `Option<String>` content 结构（`crates/fabric/src/types/workspace_checkpoint.rs:133-139`）只能作为已有能力基线，不能作为完整 workspace snapshot 的完成证据。

---

## 5. 分阶段迁移计划

> **Phase ≠ 调度单位。** 本节的 A0–A9 是**契约分组**，不是执行节点；执行节点是统一计划 §3 的 X0–X14。
> 每个 `A-*` 验收 ID 的调度归属见统一计划 §4.3（注意 A4 的两个 ID 被分开归属：`A-TURN-001` → X7，
> `A-TURN-002` → X8c；A6 整体 → X14）。
>
> **验收 ID 必须有代码绑定。** 基线实测：本节所有 `A-*` ID 在仓库中（排除 `docs/plans/`）命中为 0，
> 即它们目前只是散文。统一计划 §4.1 定义了唯一机械判定手段——测试函数名以 ID 小写、`-` 换 `_` 为前缀
> （`A-ENTRY-002` → `fn a_entry_002_*`），并由 X1 建立的 `config/architecture/acceptance-ids.tsv`
> 台账 + `architecture-check.sh` 校验。**没有对应测试函数的 ID 不算通过。**

### Phase A0：基线对账与增量冻结

目标：阻止继续失稳。

任务：

1. 暂停新增一级 crate、顶层 Manager、第二入口。该冻结从 B0 合入后生效；§0.3 的历史分支均已合入，不再存在豁免。
2. 以 `config/architecture/` 现有 inventory 为唯一机器可读基线，不重新生成平行地图。
3. 对本计划涉及的 Turn、Session、Plan、Memory、Agent、Command、Receipt 和 ID 类型生成 `existing/extend/new/delete` 矩阵。
4. 清点兼容 re-export、legacy handler、deprecated type 的真实调用点和删除前置条件。ARCH-ENTRY-01 的两级结论已完成，P0 只需复核并补齐其余条目。
5. 将本轮新增 wire/persistence surface 登记到既有 TSV，并刷新 frozen commit：`config/architecture/compatibility-debt.tsv:1` 与 `config/architecture/metrics.env:1` 当前仍为 `efde4ca11a7c45a72ce7a03fee4cdd0c3c9d8e1e`，必须推进到统一计划 §0 的 `PLAN_BASELINE_SHA=c080b08bf3170dd8a09acdb738a255133abbf11b`，并记录 `efde4ca1 → c080b08b` 期间的计数变化；计数上升必须给出理由或立即修复。
6. 建立本计划 ADR index，链接现有架构完成报告，避免重复决定。
7. 按 §0.3 保存 14 条历史分支均为 `already_merged` 的命令证据；发现新增未合入分支或工作树生产改动时另行逐项判定 owner。
8. 按 §0.2 与 C-plan、R-plan 互相登记 owner 边界，确认无契约双写。

产物：

- 本计划内的 authority delta 和 contract migration matrix。
- `config/architecture/*` 的 reviewed delta，含刷新后的 frozen commit 与计数差异说明。
- compatibility/deprecation ledger 增量。
- 架构计划与 UX 计划的共同 PR DAG。
- in-flight 分支对账表与跨计划 owner 边界确认。

验收：

- 每个变更中的核心状态只有一个声明 owner。
- 所有“当前缺失”结论均有 `path:line` 或机器可读 inventory 证据。
- 未确定 owner 或迁移策略的能力标记 blocked，禁止进入实现。
- 两个 frozen commit 已指向基线 SHA，且计数差异有结论。
- 14 条历史分支均有 `already_merged` 证据；任何新增未登记分支不得在 X1 后静默合入。

### Phase A1：扩展现有依赖和边界门禁

目标：让错误依赖无法合并。

任务：

1. 保留既有 dependency maximum、architecture checker 和 negative fixtures。
2. 在原 checker 中增量检测禁用边：
   - interact -> corpus/mnemosyne store。
   - cognit -> executive/interact。
   - domain -> aletheon assembly。
   - hardware adapter bypass hardware domain。
3. 为本轮涉及的公开协议建立稳定、低噪声 API/wire snapshot，明确生成工具、排除项和 baseline 更新流程。
4. 新增 `fabric` 类型必须同时更新 owner/consumer inventory 和 ADR reference；CI 不依赖只能在托管平台读取的 label。
5. 检测 duplicate Clap parser、duplicate RPC method string、duplicate ID wrapper 和新增 writer 绕行。

验收：

- A-DEP-001：既有 architecture acceptance 与 negative fixtures 保持通过。
- A-DEP-002：故意添加禁用依赖、未登记 wire/persistence surface 或重复生产 parser 时 CI 失败。
- A-DEP-003：新增 fabric 类型缺少 owner/consumer/ADR metadata 时 CI 失败。

### Phase A2：唯一 Command/Intent 主链

目标：删除多入口控制面。

任务：

1. 定义 `CommandSpec` 和 `ClientIntent`。
2. 顶层 CLI、TUI slash、line mode、gateway 只负责转换 ClientIntent。
3. Executive `CommandDispatcher` 成为唯一 handler 入口。
4. 先迁移 `TaskKindArg`：把它从 `crates/interact/src/tui/cli.rs:97-107` 移到 CommandSpec 权威位置，`crates/aletheon/src/main.rs:15` 改为引用新位置，`From<TaskKindArg> for fabric::TaskKind` 随之迁移。此步不删除任何文件。
5. 再删除 `crates/interact/src/tui/cli.rs` parser、旧 handler 与 `crates/interact/src/lib.rs:28` 的 `pub use tui::cli`。
6. 删除 line mode 手写 slash match。
7. 运维脚本调用 admin contract 或专用内部 binary API。

任务 4 与任务 5 不得合并为同一 commit：先迁移使 `interact::cli` 的生产调用计数归零，再删除才能产生可验证的 `caller count=0` 证据。

验收：

- A-ENTRY-001：同一 command 从 CLI/TUI/gateway 进入相同 use case。
- A-ENTRY-002：不存在第二套生产 parser。
- A-ENTRY-003：`crates/aletheon` 不再 import 任何 `interact::cli::*`；assembly 对 presentation 兼容模块的类型依赖归零。

### Phase A3：唯一 Session/Event Authority

目标：恢复、投影、记忆输入都基于同一事实链。

任务：

1. 定义 authoritative event append contract。
2. 明确 event journal、snapshot、projection 的关系。
3. SessionStore 只通过 SessionAuthority 写入。
4. TUI state 可从 snapshot + events 重建。
5. Memory observation 绑定 source event/turn/provenance。
6. 增加 schema version 和 migration policy。

验收：

- A-SESSION-001：随机中断后 replay 得到相同 TaskSnapshot。
- A-SESSION-002：重复 event/idempotency key 不产生双重 patch 或双重记忆。
- A-SESSION-003：UI 本地状态丢失不影响恢复。

### Phase A4：Task/Turn/Plan 生命周期收敛

目标：拆掉过重 chat handler，形成明确应用状态机。

任务：

1. 建立 Session→Task→Turn→Activity 层级。
2. Executive TaskController 管 task phase 和 settlement。
3. Cognit 唯一执行 Turn reasoning。
4. Agora 唯一维护 active plan graph。
5. validation/review finding 通过 typed transition 更新 plan。
6. 模型输出不得直接写 settlement。

验收：

- A-TURN-001：Linear、Robot、external runtime 使用相同 Turn terminal contract。
- A-TURN-002：测试失败使 task 进入 repair，而不是 completed。

### Phase A5：Capability Gateway 收敛

目标：所有外部副作用通过统一治理路径。

任务：

1. 定义 CapabilityInvocation/Receipt envelope。
2. Tool、MCP、Skill、runtime、hardware adapter 接入 envelope。
3. Permission、lease、budget、deadline、cancellation 统一前置。
4. mutation 根据 `full/best_effort/non_rollbackable` coverage 进入 ChangeTransaction 或领域补偿路径。
5. receipt 统一注册到 event/session，但保留 domain payload。
6. 所有 fallback 产生显式 event。

验收：

- A-CAP-001：任何文件修改都能追溯到 invocation 和 transaction。
- A-CAP-002：取消能传播到 managed command/child runtime。
- A-CAP-003：预算耗尽不能继续调用外部能力。
- A-CAP-004：hardware action 没有 permit 时不可执行。
- A-CAP-005：异步 capability 未观察 terminal snapshot/receipt 时不可报告成功。

### Phase A6：Memory 权威与 GBrain 边界

> **调度归属：统一计划 X14。** 本阶段在本文件 §7 的历史 P 表里**没有对应的 P 节点**——那是原
> DAG 的漏项，而 §12.1 第 8 条又把 memory owner 边界列为完成条件。统一计划 §3 已补出 X14
> （依赖 X5b，不阻塞 X12），`A-MEM-001..003` 的归属见统一计划 §4.3。
> `auro/feat/20260801-unified-memory-design` 与 `auro/fix/20260804-gbrain-memory-pipeline` 已合入 `origin/dev`；X14 必须在当前代码上复核并只补剩余 authority gap，不得重复移植历史提交。

目标：消除多套 memory 写入/召回语义。

任务：

1. 建立 Memory Taxonomy 和 authority table。
2. 所有 observation 通过 governed intake。
3. recall 返回 provenance、scope、freshness、authority。
4. GBrain 作为 supplemental backend，通过 workspace binding/grant 接入。
5. 禁止 GBrain 直接覆盖 Dasein/core authoritative state。
6. session evidence 与 distilled memory 明确区分。

验收：

- A-MEM-001：记忆不跨 workspace/session scope 泄漏。
- A-MEM-002：当前证据与历史推断可区分。
- A-MEM-003：GBrain 不可用时 fallback 明确可观测。

### Phase A7：External Runtime 与 Sub-agent 恢复

目标：把 外部编码 runtime / sub-agent 变成真正受监督执行域。

任务：

1. RuntimeSelector 只负责选择。
2. Executive 管 child lifecycle、namespace、permission、budget、lease。
3. child 输出必须进入 RuntimeReceipt。
4. parent acceptance 校验 changed paths、validation 和 settlement。
5. 重启 reconcile child 状态。
6. 故障注入：崩溃、超时、网络断开、部分输出、重复 receipt。

验收：

- A-AGENT-001：child 自报成功不能推进父任务。
- A-AGENT-002：child 崩溃不会丢失已确认 checkpoint。
- A-AGENT-003：权限、预算和 workspace 不可越界。

### Phase A8：删除兼容层和旧实现

目标：完成迁移而不是永久双轨。

任务：

1. 每个 deprecated item 设置 owner、replacement、deadline。
2. 加入调用 telemetry/test coverage，确认无生产调用。
3. 删除 legacy parser、handler、re-export、type alias。
4. 更新 README capability matrix，只标记唯一生产路径。
5. 执行 dead code 和 public API audit。

验收：

- A-DELETE-001：deprecation ledger 无过期未处理项。
- A-DELETE-002：README Stable 项全部有 E2E 和恢复证据。

### Phase A9：机器人域接入统一主链（独立扩展门）

目标：验证架构不仅服务 Coding Agent。

本阶段依赖核心工程主链完成，但不反向阻塞核心架构/UX 计划关闭；bridge、simulator 或外部团队条件单独记录为 domain acceptance 状态。

领域实现与关闭条件由 `docs/plans/robot-vla-production-closure-plan.md` 的 R0–R8 拥有。本阶段只回答一个问题：robot 域是否复用统一 Task/Turn/Activity/Receipt 主链。不得在此重新定义 Policy/VLA、Perception 或 Episode 契约。

任务：

1. RobotCognitiveSession 接入统一 Task/Turn/Activity。
2. Hardware permit 和 EpisodeReport 适配 Capability envelope。
3. Artifact、validation、settlement 进入 SessionAuthority。
4. 保留 Robot domain verifier 和 safety authority。

验收：

- A-ROBOT-001：Kuavo 仿真闭环通过统一主链。
- A-ROBOT-002：没有把 hardware 退化为普通 Bash/MCP tool。

---

## 6. 数据迁移与兼容策略

### 6.1 Expand → Migrate → Contract → Delete

每次迁移必须遵守：

1. Expand：增加新字段/新投影，旧 reader 仍可读。
2. Migrate：后台/启动时迁移现有记录。
3. Contract：所有 writer 切到新权威。
4. Delete：删除旧 writer 和兼容读取。

禁止只完成前三步，永远保留旧路径。

### 6.2 Schema 原则

- 所有持久协议有 version。
- migration 是幂等的。
- 不可逆 migration 前必须备份和 dry-run。
- 旧字段弃用要有 deadline。
- 事件不得原地修改；使用新事件纠正。

### 6.3 Feature flag 原则

- Feature flag 只用于短期迁移或真正实验能力。
- flag 必须有 owner、expiry、default、fallback event。
- 两条生产主链不能长期通过 flag 并存。

### 6.4 每个持久化/协议变更的强制迁移表

每个 PR 必须逐 surface 填写，不允许只引用通用迁移原则：

| 字段 | 必填内容 |
|---|---|
| surface/owner | 对应 `wire-surfaces.tsv` 或 `persistence-surfaces.tsv` 行 |
| old/new schema | 精确 version、reader、writer |
| compatibility window | 哪些 client/daemon/core 版本允许共存 |
| expand/migrate/contract/delete | 每一步的代码和证据 |
| backup/restore | 备份位置、校验、失败恢复命令 |
| downgrade | supported 或 explicitly unsupported |
| fencing/idempotency | generation、key、重复/迟到消息处理 |
| exit evidence | 旧 row/旧 caller 数量归零的机器证据 |

---

## 7. 历史 PR DAG（DO-NOT-SCHEDULE）

> **本节仅为历史记录，禁止作为调度输入。** 唯一执行顺序是
> `Aletheon_Unified_Execution_Plan_2026-08-04.md` §3 的节点清单；P0–P10 → X0–X14 的映射见统一计划 §5。
> 自动执行器读到本节应直接跳过：这里的依赖列已被统一计划取代，且本节缺少验收 ID 归属（统一计划 §4.3）、
> 状态词表（§7.1）和失败/重试策略（§8.3）。下面保留原文只是为了追溯当初的切分理由。

架构与 UX 不再分别定义同一个契约。以下节点是**当初**设计的执行顺序；UX 计划的 PR 只能消费这里冻结的契约。

```text
C-plan C0-C7 (in-flight, 冻结前既有工作流)
  |  typed provider/usage/cache facts
  v
P0 baseline reconciliation
  -> P1 contract migration matrix + incremental gates
     -> P2 command/intent convergence
        -> P3 session/task/activity projection
           -> P4 capability + transaction coverage
           -> P5 TUI task console
              -> P6 checkpoint/review/settlement
     -> P7 provider/runtime/recovery fencing        <- consumes C-plan facts
        -> P8 compatibility deletion
           -> P9 installed engineering acceptance
              -> P10 optional robot-domain acceptance
                   ^
                   |  domain closure owned by R-plan R7/R8
```

C-plan 与 R-plan 不是本 DAG 的下游节点，而是**并行的输入源**：

- C-plan 独立推进并合并；P0 只对账，P5/P7 消费其冻结后的字段定义。若 C-plan 字段在 P5 之后仍变化，按 wire surface 变更走 §6.4 迁移表。
- R-plan 独立推进；P10 只验证 robot 域是否走统一主链，域内关闭由 R8 判定。
- 除这两项外，任何未登记在本 DAG 上的工作流不得在 P1 之后合入（见 §0.3）。

| PR | 依赖 | 主要写入范围 | 必须删除/迁移 | 最窄退出证据 |
|---|---|---|---|---|
| P0 `arch/baseline-reconciliation` | 无 | 本计划、`config/architecture/*` | 过期事实、重复 A0/A1 任务 | inventory diff reviewed + 两个 frozen commit 刷新至基线 + in-flight 分支对账表 |
| P1 `arch/contract-migration-gates` | P0 | `fabric` contracts、architecture checker/fixtures | 重复 ID/未登记 surface | negative fixtures（含 `hardware::OperationId` 同名不同表示的失败用例） |
| P2a `arch/taskkind-relocation` | P1 | CommandSpec 权威位置、`crates/aletheon/src/main.rs`、`crates/interact/src/tui/cli.rs` | 无（纯迁移，不删文件） | `interact::cli` 生产调用计数 1→0 |
| P2b `ux/command-intent-convergence` | P2a | `crates/aletheon`、`interact`、Executive command use case | 兼容 parser/handler/re-export | parser/help/completion contract tests + caller count=0 |
| P3 `ux/task-activity-projection` | P1,P2b | Executive projection、Fabric wire contract、Interact reducer | presentation-owned business state | deterministic replay tests |
| P4 `arch/capability-transaction-coverage` | P1,P3 | Executive application ports、Corpus adapters、domain adapters | 未声明 mutation path | capability/receipt/fault fixtures |
| P5 `ux/tui-task-console` | P3 | `crates/interact/src/tui/` | chat transcript 中的重复状态 | golden frame + PTY tests |
| P6 `ux/checkpoint-review-settlement` | P3,P4,P5 | checkpoint/validation/review projection | 同名 checkpoint DTO 或模型自报完成 | rollback/conflict/host-settlement tests |
| P7 `arch/runtime-recovery-fencing` | P1,P3,P4；消费 C-plan 冻结字段 | machine core、daemon recovery、runtime adapters | session-local provider governance | restart/orphan/version-skew fixtures |
| P8 `arch/compatibility-deletion` | P2a-P7 | compatibility ledger 指定路径 | 到期 parser/re-export/writer | caller count=0 + architecture gate |
| P9 `acceptance/installed-engineering-mainline` | P8 | tests/docs only，修复另开 PR | 无 | installed provenance + real TUI suite |
| P10 `robot/unified-domain-acceptance` | P6,P7,P9；域内关闭见 R-plan R7/R8 | Hardware/robot projection | robot 专用旁路 | simulator/domain receipt evidence（不含实机） |

每个 PR task packet 必须包含：

- 精确基线 SHA、依赖 PR 和允许写入路径。
- 当前 `path:line` 事实、目标契约及 owner。
- 被替代路径、迁移步骤、删除条件和 rollback。
- 最窄 deterministic validation；Rust 命令一律使用 `bash scripts/cargo-agent.sh`。
- failure/recovery fixture、wire/persistence inventory delta。
- 不得把 P10 外部仿真阻塞回写为 P9 核心产品失败。

---

## 8. 架构门禁

任何新功能合并前必须回答：

1. 它属于哪个现有 domain？
2. 谁是唯一状态 owner？
3. 通过哪个 application use case 进入？
4. 通过哪个 capability path 执行副作用？
5. 产生什么 event/receipt？
6. 如何取消、超时、恢复和重放？
7. 是否引入第二套入口或第二权威？
8. 替代了什么旧实现，何时删除？
9. 为什么不能放在现有 crate？
10. 端到端完成标准是什么？

任一问题无答案，PR 不应进入实现阶段。

### 新增一级 crate 的严格条件

只有同时满足以下条件才允许：

- 现有领域边界确实无法表达。
- 至少两个现有 crate 需要通过稳定 port 使用它。
- 新 crate 能减少依赖环或隔离外部域。
- 有 dependency test 证明收益。
- 有 ADR、owner、public API 和删除/迁移计划。

“文件太多”“概念听起来独立”不是新增 crate 的理由。

---

## 9. 测试与验证

### 9.1 Architecture tests

- crate allowlist。
- forbidden imports。
- single parser/single dispatcher。
- single SessionStore writer。
- mutation transaction enforcement。
- hardware permit enforcement。
- no domain dependency on assembly/presentation。

### 9.2 Replay tests

- event journal → TaskSnapshot。
- duplicate event/idempotency。
- crash between patch and receipt。
- crash between validation and settlement。
- child terminal receipt replay。
- memory observation replay。

### 9.3 Fault injection

- daemon/core restart。
- SQLite busy/corruption boundary。
- provider timeout。
- MCP/GBrain unavailable。
- managed command hangs。
- child runtime crash。
- workspace concurrent mutation。
- hardware denial。

### 9.4 Production acceptance

- 固定版本的工程任务 corpus 至少 20 个，成功不少于 16 个；记录 provider/model、预算、workspace fixture、评分 rubric 和每任务 evidence。corpus 落点为既有目录 `tests/coding/acceptance/`，当前只有 8 个 fixture（`approval_blocked_patch`、`budget_exhaustion`、`clippy_cleanup`、`config_schema_sync`、`dirty_workspace_preservation`、`rustdoc_contract`、`rust_multifile`、`rust_regression_test`），**缺 12 个**。补齐 fixture 是 P9 的前置任务，不是 P9 内的临时产物；不得新建平行 corpus 目录。
- 100% accepted 任务有 evidence/validation receipt。
- 重启不丢 Goal/Plan/预算/已确认 checkpoint。
- memory 不跨 scope 泄漏。
- review/test finding 能修改 plan。
- sub-agent 故障注入通过。
- 同一真实 TUI session 完成多轮任务，并额外完成三次 fresh-session routing/argument 验证。
- `provider_unavailable`、`provider_rejected_request` 或渲染出的 inference error 均判为失败，即使 monitor 报 PASS。

### 9.5 Installed provenance gate

影响 tools、profiles、config、persistence、IPC、daemon bootstrap 或 client 行为的 PR 只有在以下系统安装验收通过后才能关闭 P9：

1. `sudo bash scripts/aletheon.sh deploy` 成功；user-local deploy 不能替代系统验收。
2. `target/release/aletheon`、`/usr/bin/aletheon` 与动态枚举到的所有运行中 Aletheon daemon executable 的 SHA-256 完全一致；至少核对 machine core、user daemon，以及运行中的 Memory Agent。
3. systemd units 保持 active，`NRestarts` 在观察窗口内稳定。
4. `/usr/bin/aletheon` 通过 official user socket 完成真实 LLM 请求。
5. rendered frame、persisted session、audit/receipt、daemon logs 与 monitor verdict 一致。

### 9.6 Rust 验证资源规则

- 禁止直接运行 `cargo`；使用 `bash scripts/cargo-agent.sh <arguments>`。
- 每个 PR 先运行最窄 package/test target；只有最终 integration owner 运行 workspace-wide check/test。
- 不并发运行 Executive 或 workspace build。
- 格式检查使用 `bash scripts/cargo-agent.sh fmt --all -- --check`。

---

## 10. 观察指标

### 架构指标

- 生产 CLI parser 数量：1。
- Session/Event writer：1 个权威 port。
- Turn engine authority：1。
- Capability side-effect path：1。
- 过期 deprecation：0。
- 禁用依赖边：0。
- 静默 fallback：0。

### 运行指标

- unreconciled running activity：0。
- duplicate settlement：0。
- mutation without transaction：0。
- receipt missing rate：0%。
- scope leakage：0。
- replay divergence：0。
- provider inference rounds、provider retries、tool calls、terminal tool results 分维度记录。
- active context occupancy 不从累计 billed/session tokens 推导。
- provider cache 未返回统计时为 `unknown`，不能记作 0 hit。

### 指标测量协议

每个百分比或延迟指标必须记录 fixture 版本、样本数、冷/热启动、硬件/OS、TTY/SSH、provider/model、p50/p95/p99 和失败样本。用户行为 telemetry 默认不作为功能权威；若采集，必须定义 opt-in、脱敏、保留期、owner 和删除方式。

上述“架构指标”的机器可读落点是既有的 `config/architecture/metrics.env`（当前 9 个计数器，`frozen_commit` 见 §5 Phase A0 任务 5）与 `config/architecture/compatibility-debt.tsv`，由 `scripts/libexec/aletheon/architecture-check.sh` 生成并棘轮。本计划新增的指标（生产 parser 数量、Session writer 数量、禁用依赖边、静默 fallback）必须作为新计数器加入同一文件，不得另建指标文件或只写在文档里。

**计数器名称与节点归属见统一计划 §3.2**：`PRODUCTION_CLI_PARSERS`（X1 定义 / X3c 达标）、`SESSION_APPEND_WRITERS`（X1 定义 / X5b 达标）、`FORBIDDEN_DEPENDENCY_EDGES`（X1）、`SILENT_FALLBACKS`（**X9c**，因为它需要 fallback event 先存在，无法在 X1 落地）。四者缺一即视为该节点未达退出证据。“运行指标”属运行期观测，落点为 event/receipt 与 daemon 指标，不进 architecture ratchet。

---

## 11. 明确禁止事项

1. 不新增第三套 Session、Plan、Memory、Agent 状态权威。
2. 不以新的 Facade/Manager 包住旧实现后称为完成重构。
3. 不让 TUI、CLI 或 RPC handler 直接访问 store/tool/domain internals。
4. 不让模型文本直接决定 accepted/completed。
5. 不让 child runtime 自报成功推进父任务。
6. 不让 GBrain 成为 Dasein/Mnemosyne 的隐藏权威。
7. 不让 Robot action 绕过 Hardware permit/Safety Supervisor。
8. 不把宏内核拆成一组 systemd 微服务。
9. 不同时重构多个领域并用大 PR 合并。
10. 不以更多文档、类型或单测代替端到端证据。
11. 不长期保留双轨 feature flag。
12. 不在未删除旧路径时继续横向新增同类能力。

---

## 12. 完成定义

### 12.1 核心架构与工程产品完成

本计划的核心部分完成时，Aletheon 必须满足：

1. 只有一个 Client Intent/Command 入口模型。
2. 只有一个 Session/Event Authority。
3. 只有一个受治理 Turn 生命周期。
4. 所有副作用通过 Capability Gateway。
5. 所有修改声明 transaction coverage；`full` 进入 ChangeTransaction，`best_effort/non_rollbackable` 进入显式风险、审批和领域补偿路径或被拒绝。
6. UI、CLI、Gateway 都只是 adapter/projection。
7. External runtime 是受监督 child，不是第二个 Agent 权威。
8. Mnemosyne、Dasein、Agora、Cognit、Executive 的 owner 边界可由测试证明。
9. 旧 parser、legacy handler、过期 re-export 和双轨 writer 已删除。
10. 真实任务、恢复、fault injection 和 installed provenance gate 达到验收标准。

### 12.2 Robot domain 扩展完成

Robot domain completion 独立于核心完成状态：

1. 至少一个 Kuavo simulator 闭环通过统一 Task/Activity/Receipt 主链。
2. safety denial、bridge unavailable 和 non-rollbackable device effect 均有领域化 terminal receipt。
3. 物理实机验证不是本计划关闭条件；需要时由独立 HIL/production plan 管理。

最终判断架构是否稳定，不看 crate 数量、代码行数或概念数量，而看：

> 任意用户请求是否都能沿唯一主链执行、产生权威收据、在中断后恢复，并且不存在另一套隐藏状态或旁路执行路径。

## 13. Owner / independent review 清单

1. 本计划是否把已完成的 architecture inventory/checker 作为基线，而不是重新建设 A0/A1？
2. Executive Turn lifecycle 与 Cognit reasoning authority 是否分离且无双 writer？
3. ID/Checkpoint/Approval/Receipt 是否先做 migration matrix，避免同名重复类型？
4. Capability envelope 是否只统一治理元数据，并保留不同事件/terminal semantics？
5. Provider machine authority、runtime facts、generation fencing、orphan reconciliation 是否进入主链？
6. ChangeTransaction 是否诚实区分 full/best-effort/non-rollbackable？
7. 每个 PR 是否有依赖、写入边界、删除项、schema/rollback 和 deterministic exit evidence？
8. Rust 验证是否遵守 `scripts/cargo-agent.sh` 和串行 workspace policy？
9. installed system runtime 是否是 P9 最终验收，Robot/HIL 是否作为独立扩展门？
10. 兼容层删除是否遵守"先迁 `TaskKindArg`、后删 parser"的强制顺序，而不是把 `caller count=0` 当作已成立事实？
11. C-plan 与 R-plan 的 owner 边界是否已双向登记，且本计划没有重定义 cache/robot 契约？
12. 14 条历史分支是否都有 `already_merged` 证据，B0 后新增的分支/改动是否都登记 owner，DAG 的串行假设是否成立？

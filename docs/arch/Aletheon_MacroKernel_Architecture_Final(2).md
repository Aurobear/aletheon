# Aletheon 宏内核架构设计（代码对齐修订版）

> **Status:** Canonical architecture constitution
>
> **Verified snapshot:** 2026-07-19。本文的运行不变量是长期约束；“当前代码”章节只描述所列代码锚点可验证的事实。
>
> 本文基于当前仓库真实代码修订，而不是继续扩充概念。
>
> 核心结论：Aletheon 应成为一个**单实例、宏内核式、内部模块化、外部执行域隔离的 Agent Runtime**；但它不应照搬 Linux 的全部对象与接口，也不应把所有认知步骤都包装成“进程”。

---

## 1. 这次修订解决什么问题

旧方案已经正确提出 Process、Space、Chronos、IPC、Supervision、Namespace 和 Governance，但仍有五个结构性问题：

1. **过度类比 Linux**：统一对象、统一 URI、统一 read/write/watch 容易产生一层庞大的抽象外壳。
2. **Process 粒度过细**：Planner、Reviewer、Executor 不一定都是进程，它们也可能只是一次认知 Harness 内的阶段。
3. **通信语义混在一起**：同步接口、命令、事件、消息和遥测流不应全部塞进一个 Bus。
4. **Executive 边界仍然过宽**：如果把全部机制都加入 Executive，它只会从 Runtime God Object 变成 Kernel God Object。
5. **迁移顺序没有对准现有代码的最大问题**：当前最需要先消除的，是多套认知主链和超重的 daemon chat handler，而不是先增加十几个 Manager。

因此，新版本的目标不是“补齐更多模块”，而是建立少量、可验证的运行不变量。

---

## 2. 当前代码的真实状态

当前生产结构已经从“多个候选架构并存”进入“边界收敛但仍有接线缺口”的阶段：

| 领域 | 当前权威实现 | 当前约束或缺口 |
|---|---|---|
| 应用入口 | `crates/aletheon/src/main.rs:1-23` 统一提供 TUI、daemon、exec、config 与 doctor | 入口只负责装配，不拥有领域策略 |
| 执行治理 | `crates/kernel/src/lib.rs:1-12` 拥有 admission、operation、process、chronos、space 与 supervision | Kernel 不实现认知、工具或 OS backend |
| 系统编排 | `crates/executive` 是 composition root，连接 Turn、Session、Agent control、Kernel 与领域服务 | 不能重新吸收 Kernel 机制或具体 OS 实现 |
| 外部 Runtime | `crates/runtime/src/lib.rs:1-8` 只定义 manifest 与 selector | 实例、registry、result 与 settlement 生命周期归 Executive/Fabric |
| Pi adapter | `crates/executive/src/impl/runtime/pi_rpc.rs:153-412` 实现 Executive `AgentRuntimeLauncher` | 返回 bounded `AgentResult`；最终验证归 Executive |
| 工具与 MCP | Corpus 拥有 MCP schema、client、manager、transport、auth 和工具包装 | Corpus 是唯一 MCP lifecycle owner，不再依赖 Cognit schema |
| Host Platform | `crates/platform/src/lib.rs:1-38` 统一 contract、selector 与 Linux/Windows/macOS backend | 不再按 OS 或 API/host 人为拆 crate |
| 隔离副作用 | Executive 按配置启动 `execd`，见 `crates/executive/src/impl/daemon/bootstrap/request.rs:452-479` | `execd` 暂时依赖完整 Corpus 的 structured patch 实现 |
| Hardware | `crates/hardware/src/lib.rs:1-29` 包含设备控制 contract 与 simulator | 尚无生产调用者，必须标为 experimental；不得声称 production-ready |

### 2.1 当前最重要的是保持单一权威

回收后的控制关系是：

```text
aletheon entry
    |
    v
Executive composition root
    |-- Kernel              lifecycle / admission / supervision
    |-- Cognit              cognition and turn harness
    |-- Corpus              governed tools and MCP execution
    |-- Runtime contract    external task executor lifecycle
    |-- Platform            host operating-system capabilities
    `-- execd               isolated low-level side effects
```

仍需持续检查的风险不是“模块不够多”，而是同一语义出现第二个 owner：

- 新入口绕过统一 Turn/Operation 管线；
- Runtime 自行判定全局任务完成；
- Corpus、Runtime 或领域服务自行授予权限；
- Cognit 与 Corpus 各自维护一套 MCP 配置；
- Executive 与 Event Spine 同时声称拥有 durable fact；
- Platform、execd 与 Corpus 重复实现同一种 Host 操作。

因此架构演进必须先删除重复权威，再增加能力。

---

## 3. Aletheon 的最终定位

Aletheon 是：

> **管理长期存在的 Agent Process、认知操作、上下文空间、能力调用和外部执行域的宏内核式运行时。**

它由四层构成：

```text
Interact / API / Automation
             |
             v
          Executive
      composition root
       /      |      \
      v       v       v
   Kernel   Services   External execution domains
 lifecycle  Cognit     Runtime / execd / Hardware edge
 governance Dasein
            Agora
            Mnemosyne
            Corpus
```

其中：

- **Executive Kernel** 定义运行语义，不实现认知业务；
- **Kernel Services** 提供认知、主体、共享空间和经验能力；
- **Agent Process** 是被内核调度和监督的长期执行实体；
- **External Execution Domain** 接触高风险、强隔离或实时世界；
- `systemd` 只管理整个 Aletheon 实例。

---

## 4. 宏内核的“最小宪法”

宏内核不是模块清单，而是一组任何实现都必须维持的不变量。

### 4.1 六条核心不变量

1. **唯一执行入口**：所有 Turn、Task、Automation 最终进入同一 Operation 执行管线。
2. **单一所有权**：每类可变状态只能有一个权威 owner，其他模块只能通过 Port 访问。
3. **结构化生命周期**：Process、Operation、Lease、Timer 必须有显式状态与终止原因。
4. **能力不能绕过治理**：所有外部副作用必须经过 Admission，不能直接调用 Tool 实现。
5. **共享状态只能提交**：Agent 不能直接修改 Agora 的全局可见状态。
6. **时间语义必须显式**：timeout 使用 monotonic time，用户时间使用 wall time，事件排序使用 logical time。

### 4.2 内核只管理七类对象

不要把所有记忆、事实、文件和认知对象都提升成 Kernel Object。第一阶段只管理：

```text
AgentProcess
Operation
ContextSpace
Mailbox
Timer
ResourceLease
NamespaceBinding
```

Memory、Fact、Plan、Artifact、World Entity 是服务领域对象，由各自服务提供强类型 API。

这避免 Kernel Object Model 变成新的万能数据库。

---

## 5. Process 不是 Cognit Stage

### 5.1 三个必须区分的概念

```text
AgentProcess：长期身份、状态、Mailbox、Space、权限与预算边界
Operation：一次可取消、可计量、有 deadline 的工作
Turn：某类 Operation，表示一次交互认知活动
```

Planner、Reviewer、Executor 默认只是 Cognit Harness 中的 Stage。只有满足以下任一条件时，才升级为独立 Agent Process：

- 需要并发执行；
- 需要独立 Mailbox；
- 需要独立权限、预算或 Context Space；
- 需要单独监督、重启或远程部署；
- 生命周期跨越单次 Turn。

### 5.2 Process Control Block

```rust
pub struct AgentProcess {
    pub id: AgentId,
    pub parent: Option<AgentId>,
    pub profile: AgentProfileId,
    pub state: ProcessState,
    pub backend: ExecutionBackend,

    pub space: SpaceId,
    pub mailbox: MailboxId,
    pub namespace: NamespaceId,
    pub principal: PrincipalId,

    pub supervisor: Option<AgentId>,
    pub created_at: WallTime,
    pub last_heartbeat: MonoTime,
    pub exit: Option<ExitStatus>,
}
```

建议第一阶段状态机保持最小：

```text
Created → Ready → Running ↔ Waiting → Stopping → Exited
                         └────────────→ Failed
```

`Suspended`、抢占、迁移以后再做。

### 5.3 Operation

```rust
pub struct Operation {
    pub id: OperationId,
    pub owner: AgentId,
    pub kind: OperationKind,
    pub state: OperationState,
    pub submitted_at: MonoTime,
    pub deadline: Option<MonoDeadline>,
    pub budget: BudgetId,
    pub cancellation: CancellationId,
    pub causation_id: Option<OperationId>,
}
```

Operation 才是 Scheduler、Accounting、Timeout 和 Cancellation 的基本单位。

---

## 6. 唯一认知执行主链

### 6.1 目标流程

```text
Input Adapter
    -> Session Service
    -> Executive submits Operation
    -> Admission Permit
    -> Cognit Session / selected Runtime
    -> governed Capability Invocation
    -> Corpus / Platform / execd / Hardware
    -> Evidence + Turn Result
    -> Executive verification and settlement
    -> Mnemosyne / Agora / durable events
```

daemon、TUI、`exec`、Automation 都只负责构造 `OperationRequest`，不再各自运行 LLM Loop。

### 6.2 Cognit 的对象安全边界

当前 `ReActLoop::run` 使用泛型 LLM 和泛型 Tool closure，导致 Harness 难以成为可替换的 trait object。建议改为：

```rust
#[async_trait]
pub trait CognitiveSession: Send {
    async fn run_turn(
        &mut self,
        request: TurnRequest,
        services: &dyn TurnServices,
        events: &dyn TurnEventSink,
    ) -> Result<TurnResult, CognitError>;
}

pub trait HarnessFactory: Send + Sync {
    fn create(&self, profile: &CognitProfile) -> Box<dyn CognitiveSession>;
}

#[async_trait]
pub trait TurnServices: Send + Sync {
    async fn invoke(&self, req: CapabilityRequest) -> CapabilityResult;
    async fn recall(&self, req: RecallRequest) -> RecallResult;
    async fn dasein_view(&self, process: AgentId) -> DaseinView;
    async fn agora_view(&self, space: SpaceId) -> AgoraView;
}
```

这样 Cognit 不依赖 Corpus、Mnemosyne、Dasein 的具体类型，也不需要让 Executive 组装 Closure。

### 6.3 daemon Handler 的目标

`handle_chat` 最终只应做：

```text
parse request
resolve session/process
submit operation
forward events
format response
```

Skill 注入、Memory recall、Dasein view、Hooks、Reflection、Agora commit 应进入可组合的 Turn Pipeline，而不是继续留在 JSON-RPC Handler。

---

## 7. Executive 的准确边界

Executive 是宏内核实现，但不是所有系统能力的集合。

### 7.1 Executive 负责

```text
ProcessTable
OperationTable
Cooperative Scheduler
Chronos / Timer
Context Space Binding
Mailbox Routing
Supervision Tree
Admission Coordination
Namespace Binding
Lifecycle Events
```

### 7.2 Executive 不负责

```text
Prompt 拼接
模型 Provider 实现
ReAct 具体循环
Fact SQL 查询
Memory consolidation 算法
SelfField 内部哲学状态
Tool 具体执行
机器人控制算法
UI JSON 格式化
```

### 7.3 CoreSystems 应被拆成 Kernel State + Service Ports

当前 `CoreSystems` 持有大量具体 `Arc<Mutex<T>>`。目标结构：

```rust
pub struct KernelState {
    processes: ProcessTable,
    operations: OperationTable,
    spaces: SpaceTable,
    timers: TimerWheel,
    supervisors: SupervisorTree,
    admissions: AdmissionTable,
}

pub struct ServicePorts {
    cognit: Arc<dyn CognitService>,
    dasein: Arc<dyn DaseinService>,
    agora: Arc<dyn AgoraService>,
    memory: Arc<dyn MemoryService>,
    capability: Arc<dyn CapabilityService>,
    audit: Arc<dyn AuditSink>,
}
```

Kernel State 只能被 Executive 内部修改；Service Ports 不暴露具体锁与数据库。

---

## 8. Chronos：系统时间与存在时间必须分离

当前 Dasein `TemporalStream` 已实现 retention / present / protention，这属于**体验时间**，不能拿来实现系统 timeout。

### 8.1 两套时间系统

| 时间 | Owner | 用途 |
|---|---|---|
| Kernel Chronos | Executive | timeout、deadline、lease、heartbeat、timer、事件排序 |
| Lived Temporality | Dasein | retention、present、protention、tempo、意义连续性 |

Dasein 订阅经过筛选的体验事件，并将其转化为 lived temporality；它不提供系统 `sleep()` 或 `deadline()`。

### 8.2 Chronos 最小接口

```rust
pub trait Clock: Send + Sync {
    fn wall_now(&self) -> WallTime;
    fn mono_now(&self) -> MonoTime;
}

pub trait Chronos: Clock {
    fn logical_tick(&self) -> LogicalTime;
    fn create_timer(&self, spec: TimerSpec) -> TimerId;
    fn cancel_timer(&self, id: TimerId) -> bool;
}
```

规则：

- `Instant`/monotonic：timeout、duration、lease、heartbeat；
- wall clock：展示、日历、审计、记忆日期；
- logical sequence：commit 顺序、事件因果；
- domain time：机器人/仿真/回放，由外部 Runtime 提供 Clock Adapter。

不要把 `expires_at` 一律定义为 WallTime；运行时过期应优先使用 monotonic deadline。

---

## 9. Context Space：可见性与继承，不是另一个 Memory Store

### 9.1 Space 只保存绑定和版本

Context Space 不复制所有文本，而保存对象引用、快照版本和私有 overlay：

```rust
pub struct ContextSpace {
    pub id: SpaceId,
    pub owner: AgentId,
    pub parent_snapshot: Option<SpaceSnapshotId>,
    pub bindings: Vec<ContextBinding>,
    pub overlay: VersionedOverlay,
    pub policy: SpacePolicy,
}
```

### 9.2 四个不同的数据域

```text
Private Context：单 Agent 的临时假设、草稿、局部计划
Agora：多 Agent 共享且经过提交的工作状态
Mnemosyne：长期持久经验与事实
World Projection：外部世界的有时效版本化投影
```

Space Manager 决定“当前进程能看见哪些 view”，但不实现 Mnemosyne 数据库或 Agora 黑板。

### 9.3 Fork 与 Commit

第一阶段只实现对象级 snapshot + overlay，不实现页级 COW：

```text
fork = inherit immutable bindings + empty private overlay
commit = proposal(base_version, patch, evidence)
conflict = current_version != base_version
```

---

## 10. Agora：从 HashMap 变成事务化共享工作空间

### 10.1 Agora 的准确职责

Agora 保存：

```text
Active Goal
Task Graph
Shared Plan
Accepted Evidence
Working Hypothesis
Coordination Claims
World Projection Summary
```

它不保存完整对话历史，不替代 Mnemosyne，也不承担 IPC。

### 10.2 不再允许 publish(key, value) 作为主写接口

建议接口：

```rust
#[async_trait]
pub trait AgoraService: Send + Sync {
    async fn view(&self, space: SpaceId, selector: ViewSelector) -> Result<AgoraView>;
    async fn propose(&self, proposal: AgoraProposal) -> Result<ProposalId>;
    async fn commit(&self, id: ProposalId, permit: CommitPermit) -> Result<CommitReceipt>;
    async fn reject(&self, id: ProposalId, reason: RejectReason) -> Result<()>;
    async fn watch(&self, space: SpaceId, cursor: CommitCursor) -> AgoraStream;
}
```

`AgoraProposal` 至少包含：

```text
author process
space / namespace
base version
typed operation
evidence references
confidence
TTL
causation id
```

### 10.3 当前实现的最小迁移

1. 保留 `AgoraRegistry` 作为内存后端；
2. 为 Workspace 增加 `version`；
3. 将 `publish/update` 降级为内部 backend 方法；
4. 对外只暴露 `view/propose/commit`；
5. Tool Evidence 首先进入 private trace，再由 Harness 或 Reviewer 提交；
6. turn end 不要每次无条件把完整 snapshot 当作普通 RecallMemory 字符串存储。

---

## 11. Mnemosyne：统一门面，内部多存储

Mnemosyne 应对 Executive 只暴露一个服务端口：

```rust
#[async_trait]
pub trait MemoryService: Send + Sync {
    async fn recall(&self, req: RecallRequest) -> Result<RecallSet>;
    async fn record(&self, event: ExperienceEvent) -> Result<MemoryReceipt>;
    async fn consolidate(&self, scope: MemoryScope) -> Result<ConsolidationReport>;
    async fn forget(&self, policy: ForgetPolicy) -> Result<ForgetReport>;
}
```

CoreMemory、RecallMemory、FactStore、EpisodicMemory、AutoMemory 是 Mnemosyne 内部策略或 backend，不应全部成为 `CoreSystems` 的平级字段。

生产启用状态必须写清楚：

- always-on backend；
- feature-gated backend；
- experimental backend；
- design only。

---

## 12. Dasein：受保护主体服务，而不是安全与记忆的混合体

Dasein 负责：

```text
Identity
Care / Values
Boundary Interpretation
Continuity
Self Model
Lived Temporality
```

它通过 Port 请求 Policy、Memory 或 Capability 信息，不应直接依赖 Corpus 与 Mnemosyne 的具体实现。

建议拆出两个不同判断：

```text
Dasein Deliberation：这个行为是否符合“我是谁、我在乎什么”
Authorization Policy：这个 principal 是否被系统允许执行操作
```

两者可以共同影响 Admission，但不能混成一个模糊 Verdict。

---

## 13. Corpus：从 BodyRuntime 收敛为 Capability Execution

“Body”这个隐喻会不断吸收工具、Driver、Sandbox、Skill、Hook、平台适配和安全。建议逐步把 Corpus 的核心定义改为：

> **能力目录、调用适配与执行隔离层。**

统一入口：

```rust
#[async_trait]
pub trait CapabilityService: Send + Sync {
    async fn describe(&self, selector: CapabilitySelector) -> Vec<CapabilityDescriptor>;
    async fn invoke(&self, permit: InvocationPermit, req: CapabilityRequest)
        -> CapabilityResult;
}
```

Tool、MCP、Skill、Browser、Robot 都是 Capability Provider。副作用能力必须持有 `InvocationPermit`。

### 13.1 修正 SandboxFirst

`SandboxFirst` 不能只是 Prompt note，也不能在日志中警告后继续裸执行。它必须变成 Admission 约束：

```text
SandboxRequired
→ acquire sandbox lease
→ verify backend available
→ execute only inside sandbox
→ unavailable = deny/fail closed
```

---

## 14. 通信：不要让一个 Bus 统治所有交互

### 14.1 五种语义

| 语义 | 使用场景 | 首选机制 |
|---|---|---|
| Call / Query | 同进程、需要立即返回、维护不变量 | 强类型 trait call |
| Command | 改变状态、可排队、需要 receipt | Kernel command queue |
| Event | 已发生事实、零或多订阅者 | append + publish |
| Message | Agent Process 协作 | mailbox request/response/signal |
| Stream | token、日志、机器人 telemetry | bounded stream + backpressure |

因此应废止这条旧原则：

```text
所有跨子系统通信必须经过 CommunicationBus
```

替换为：

> 状态所有者通过强类型 Port 维护不变量；异步协作和事实传播才经过 Fabric。

### 14.2 CommunicationBus 的定位

`CommunicationBus` 只负责 Envelope 路由和 transport，不负责业务语义，也不负责替代所有 trait call。

新的 Envelope 需要补充：

```rust
pub struct Envelope {
    pub id: MessageId,
    pub source: Endpoint,
    pub target: Target,
    pub pattern: DeliveryPattern,
    pub schema: SchemaId,

    pub operation_id: Option<OperationId>,
    pub causation_id: Option<MessageId>,
    pub correlation_id: Option<MessageId>,
    pub namespace: NamespaceId,
    pub logical_time: LogicalTime,
    pub deadline: Option<MonoDeadline>,
    pub priority: Priority,
    pub payload: Payload,
}
```

### 14.3 先统一语义，再统一 Transport

当前 daemon JSON-RPC、CommunicationBus、IpcManager 和 Agent orchestration 不要立刻强行合并成一个实现。先让它们共享：

- Endpoint/Envelope；
- OperationId/AgentId；
- Error/ExitReason；
- deadline/cancellation；
- schema/version。

然后再逐步让 Unix socket 成为跨进程 transport。

---

## 15. Supervision 与结构化并发

Process 必须挂在监督树上，所有异步任务必须属于某个 Operation 或 Process。

```text
InstanceSupervisor
└── MainAgent
    ├── ChildAgent
    ├── ModelCall Operation
    └── Capability Operation
```

第一阶段只支持：

```text
NeverRestart
RestartOnFailure { max_retries, backoff }
StopChildrenOnParentExit
```

统一退出原因：

```rust
pub enum ExitReason {
    Completed,
    Cancelled,
    DeadlineExceeded,
    BudgetExceeded,
    QuotaExceeded,
    PermissionDenied,
    SandboxUnavailable,
    ProviderUnavailable,
    CapabilityFailed,
    Panic,
    Killed,
}
```

禁止把全部失败压成字符串，也禁止脱离父 Operation 的 `tokio::spawn`。

---

## 16. Governance：概念分离，入口原子化

Permission、Budget、Quota、Resource、Accounting 必须保持概念分离；但执行前不能由调用方手工依次检查，否则会产生 TOCTOU 竞争。

### 16.1 Admission Controller

```rust
#[async_trait]
pub trait AdmissionController {
    async fn admit(&self, request: AdmissionRequest)
        -> Result<ExecutionPermit, AdmissionError>;
    async fn settle(&self, permit: ExecutionPermit, usage: UsageReport)
        -> Result<(), SettlementError>;
}
```

`admit()` 内部原子协调：

```text
Authorization
→ Budget reserve
→ Quota reserve
→ Resource lease
→ ExecutionPermit
```

执行完成后：

```text
Accounting record
→ Budget settle
→ Lease release
→ Audit event
```

### 16.2 Resource 只管理可占用实体

```text
Model worker slot
Sandbox slot
Browser instance
GPU slot
Robot control channel
Remote worker
```

Token 和费用不是 Resource；它们是 Usage 与 Budget。

---

## 17. Namespace 与 Capability

### 17.1 Namespace

第一阶段只做层级 Namespace + Binding：

```text
/personal
/work/aletheon
/robot/sim
/robot/lab
/robot/production
```

每个 Process 绑定默认 Namespace，访问其他 Namespace 必须显式授权。

### 17.2 Capability Grant

权限不要只绑定 Tool 名称，应绑定类型化操作：

```text
fs.read:/workspace/project
fs.write:/workspace/project/docs
shell.exec:sandboxed
model.invoke:anthropic/*
agora.commit:/work/aletheon
robot.command:/robot/lab/kuavo-01
```

Capability 描述“能做什么”，Grant 描述“谁能在哪个范围做”，Resource 描述“执行时占用什么”。

---

## 18. 机器人边界

Aletheon 不执行 1 kHz 控制循环，也不把高频 telemetry 全部写入 Agora。

```text
Aletheon supervised Operation
          |
          v
Hardware Capability Provider
          |
          v
Robot Edge Runtime
  safety / estimation / RL-MPC-WBC
       |                 |
       v                 v
Physical Hardware   State summaries / events
                         |
                         `----> Aletheon
```

Aletheon 负责：

- 高层目标与任务规划；
- 模式选择与能力授权；
- Robot control lease；
- 状态摘要、异常事件和恢复决策；
- 仿真/实机 Namespace 隔离。

Robot Runtime 负责：

- 硬实时循环；
- State Estimation；
- MPC/WBC/RL；
- 硬件接口与本地安全；
- 高频数据缓存与降采样。

任何 Aletheon 命令都不能越过 Robot Runtime 的本地 Safety Supervisor。

---

## 19. 当前 Crate 边界

当前 workspace 采用领域级 crate，而不是按 `api`、`broker`、操作系统或临时实现阶段拆分。

| Crate | 唯一职责 | 禁止吸收 |
|---|---|---|
| `aletheon` | 最终用户 CLI、daemon/TUI/ACP 入口装配 | 领域策略、Kernel 机制 |
| `executive` | composition root、Turn/Session/Agent 编排、最终验证 | OS backend、硬件驱动、认知算法 |
| `kernel` | Process、Operation、Chronos、Space、Supervision、Admission | Prompt、Memory、Tool 与平台实现 |
| `runtime` | 外部 Runtime capability manifest 与 deterministic selector | 实例生命周期、结果、权限授予、最终完成裁决 |
| `cognit` | cognition、reasoning、planning、review 与 harness | MCP transport、OS 副作用、系统权限 |
| `corpus` | capability/tool registry、治理后的调用、MCP adapter | Agent 生命周期与最终授权策略 |
| `platform` | Host contract 与 Linux/Windows/macOS backend | Agent、模型、机器人设备语义 |
| `execd` | 独立进程中的受约束文件和进程副作用 | Prompt、Goal、Agent、Runtime session |
| `hardware` | 设备身份、命令、租约、遥测、安全语义与 simulator | Host 通用进程/文件系统、硬实时控制环 |
| `fabric` | 跨领域共享协议、ID、envelope 与兼容基础设施 | 新的领域业务与“放不下”的公共类型 |
| `dasein` | identity、care、continuity 与 lived temporality | 系统时钟、权限授予、Memory backend |
| `agora` | 共享认知工作空间及其提交语义 | 任意全局 key-value 状态 |
| `mnemosyne` | 经验、记忆、召回与持久化知识门面 | Turn 编排、系统授权 |
| `metacog` | 受治理的候选生成、评估与演化 | 绕过审批修改生产系统 |
| `interact` | TUI 与用户交互 adapter | daemon 业务与系统状态 authority |
| `gateway` | 外部请求/协议入口适配 | 领域状态与执行策略 |

### 19.1 Crate 准入

新 crate 必须同时给出：

1. 唯一领域 owner；
2. 至少一个真实生产调用者；
3. 不能作为现有 crate 内模块的理由；
4. 独立编译、依赖隔离、部署或安全边界中的至少一项收益；
5. 单向依赖与验证命令。

仅有 DTO、trait、`api`/`broker` 分层名称、未来 provider 占位或自己的单元测试，不足以成立新 crate。

### 19.2 当前允许的独立进程边界

`execd` 可以独立，因为它提供故障与权限隔离，并作为单独 binary 被 Executive 启动。Runtime adapter 只有在具有独立依赖、部署或进程生命周期时才拆出；操作系统 backend 继续留在单一 `platform` crate 内，以 `cfg(target_os)` 隔离。

---

## 20. 当前收敛顺序

架构工作按真实依赖顺序推进，不再使用预建 Wave crate：

1. 保持唯一 Turn/Operation 生产链及其 parity tests；
2. 将 MCP 配置所有权从 Cognit 迁入 Corpus；
3. 让 Runtime registry/selector 接入真实外部执行选择，同时由 Executive 保留监督与最终验证；
4. 将 `execd` 对完整 Corpus 的依赖收窄为最小 structured patch contract；
5. 接入 Hardware 前先完成 Kernel Capability、lease、deadline 与 fail-safe 的纵向测试；
6. 建立真实 `tests/coding` fixture/harness/receipt 闭环，禁止以自返回成功的 mock benchmark 代替。

每一步必须先证明生产调用链，再考虑拆分新的 adapter crate。

---

## 21. 当前已知非完成项

- Hardware 只有 contract 与 simulator，没有生产调用者；
- Runtime contract 已由 Pi adapter 实现，但 registry/selector 尚未统一所有生产路由；
- MCP 的执行归 Corpus，配置 canonical owner 仍在 Cognit；
- `execd` 的 structured patch 仍依赖 Corpus；
- Platform 已合并为单 crate，但现有 backend 中仍有未使用或占位实现；
- Coding Benchmark 尚未以真实 fixture、Executive harness 和独立验收落地。

这些条目是缺口，不是已交付能力。

---

## 22. 明确不采用的设计

暂不采用：

- 每个 crate 一个 systemd service；
- 所有跨模块调用都经过 Bus；
- 所有领域对象都实现统一 `read/write/watch`；
- 用 Dasein Temporality 实现系统 timeout；
- 将所有 Cognit stage 建模成 Process；
- 将 Agora 当成全局 HashMap 或全部 Context；
- 将 Token/费用当成 Resource；
- 在主进程运行不可信代码或机器人硬实时循环；
- 只为名称整齐而拆分或移动 crate，或保留没有生产调用者的抽象；
- 在唯一主链建立前继续增加新的 Harness/Frontend。

---

## 23. 目标运行结构

```text
Aletheon Instance
|
|-- aletheon          user entry and assembly
|-- Executive         composition, orchestration, final verification
|   |-- Kernel        process / operation / admission / supervision
|   |-- Cognit        cognition and harness
|   |-- Dasein        identity and continuity
|   |-- Agora         shared cognitive workspace
|   |-- Mnemosyne     persistent experience
|   `-- Corpus        governed capability execution and MCP
|
|-- Runtime           external task-executor lifecycle contract
|-- Platform          host OS contracts and backends
|-- execd             isolated low-level side effects
`-- Hardware          governed device domain; experimental until wired
```

稳定的主链必须只有一套：

```text
Input
  -> Operation
  -> Admission
  -> Supervised execution
  -> Cognit harness or selected Runtime
  -> Governed capability invocation
  -> Evidence + Receipt
  -> Executive verification and settlement
  -> Durable authority append / projection
```

模块数量不是目标；单一 owner、不可绕过的治理与可验证的退出语义才是目标。

---

## 24. 一句话结论

> Aletheon 不应继续模仿 Linux 的表面模块，而应继承 Linux 的核心纪律：少数权威运行对象、单一状态所有者、结构化生命周期、不可绕过的能力治理，以及稳定而可替换的服务边界。

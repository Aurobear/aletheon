# Aletheon 宏内核架构设计建议

## 1. 文档目的

本文档用于确定 Aletheon 下一阶段的核心架构方向。

Aletheon 不再被定义为一个普通的对话式 Agent 应用，也不应被拆分成大量彼此独立的 `systemd service`。建议将其正式收敛为：

> **宏内核式 Agent Runtime。**

Aletheon 内核统一定义并管理 Agent 的：

- 进程
- 时间
- 空间
- 通信
- 调度
- 监督
- 命名空间
- 权限
- 资源占用
- 配额
- 预算
- 使用计量
- 统一对象访问

认知、记忆、模型、共享工作空间等能力作为内核服务运行；机器人、浏览器、沙箱、GPU Worker 等高风险、强隔离或强实时能力通过外部执行域接入。

---

# 2. 核心定位

## 2.1 Aletheon 是什么

Aletheon 是一个：

> **管理长期运行 Agent、子 Agent、认知服务和外部能力的宏内核式运行时。**

它不只是：

```text
LLM + Memory + Tools
```

而应该逐步成为：

```text
Macro-kernel
+ Agent Processes
+ Context Spaces
+ Temporal Runtime
+ IPC Fabric
+ Namespaces
+ Supervision
+ Capability System
+ Cognitive Services
```

---

## 2.2 Aletheon 不是什么

Aletheon 不应直接成为：

- ROS 上位机应用
- 机器人 MPC/WBC 控制器
- 单一大模型封装
- 一组松散微服务
- 所有模块各自独立部署的 systemd 服务集合
- 单纯的聊天上下文管理器
- 把所有状态塞进 Agora 的黑板系统

---

# 3. 总体架构

```text
┌─────────────────────────────────────────────────────┐
│                  Aletheon Instance                  │
│                                                     │
│  ┌───────────────────────────────────────────────┐  │
│  │              Aletheon Macro-kernel            │  │
│  │                                               │  │
│  │ Process   Scheduler   Space   Chronos          │  │
│  │ IPC       Namespace   Permission               │  │
│  │ Resource  Quota       Accounting               │  │
│  │ Budget    Supervision Object Model             │  │
│  └───────────────────────┬───────────────────────┘  │
│                          │                          │
│  ┌───────────────────────▼───────────────────────┐  │
│  │              Kernel Services                  │  │
│  │                                               │  │
│  │ Agora       Mnemosyne      Cognit             │  │
│  │ Dasein      Model Service   Capability         │  │
│  │ Artifact    Session         Audit              │  │
│  └───────────────────────┬───────────────────────┘  │
│                          │                          │
│  ┌───────────────────────▼───────────────────────┐  │
│  │               Agent Processes                 │  │
│  │                                               │  │
│  │ Main  Planner  Reviewer  Executor  Specialist │  │
│  └───────────────────────┬───────────────────────┘  │
└──────────────────────────┼──────────────────────────┘
                           │ IPC / RPC / Shared Memory
          ┌────────────────┼─────────────────┐
          ▼                ▼                 ▼
     Linux Sandbox    Robot Runtime     Remote Worker
```

基本原则：

```text
内核定义机制
服务提供系统能力
Agent 执行认知任务
外部 Runtime 接触真实世界
```

---

# 4. 为什么采用宏内核

## 4.1 不采用真正微服务

真正微服务意味着：

```text
Executive Service
Agora Service
Chronos Service
Mnemosyne Service
Cognit Service
Capability Service
```

并引入：

- RPC
- 服务发现
- 重试
- 熔断
- 分布式事务
- 一致性协议
- 链路追踪
- 版本协商
- 部署编排

Aletheon 当前的主要问题不是分布式扩展，而是缺少稳定的 Agent 运行语义。

因此当前不应优先解决微服务部署问题。

---

## 4.2 宏内核的优势

以下模块需要高频交互和强一致性：

- Agent Process
- Scheduler
- Context Space
- Chronos
- IPC
- Permission
- Namespace
- Supervision

将它们放在同一宏内核运行域中，可以获得：

- 强类型直接调用
- 低延迟
- 统一状态
- 清晰生命周期
- 易于调试
- 更容易建立系统不变量

---

## 4.3 宏内核不等于巨型单体

宏内核内部仍需保持严格边界。

禁止：

```text
Cognit 直接修改 ProcessTable
Agent 直接写 Agora 内部存储
SpaceManager 直接访问 Mnemosyne 数据库实现
Capability 直接绕过 Permission
```

应通过稳定接口交互：

```rust
pub trait ProcessManagerApi {
    async fn spawn(&self, spec: SpawnSpec) -> Result<AgentId, ProcessError>;
    async fn wait(&self, id: AgentId) -> Result<ExitStatus, ProcessError>;
    async fn signal(&self, id: AgentId, signal: AgentSignal) -> Result<(), ProcessError>;
}

pub trait SpaceManagerApi {
    async fn fork_space(&self, parent: SpaceId) -> Result<SpaceId, SpaceError>;
    async fn attach_region(
        &self,
        process: AgentId,
        region: RegionId,
        access: AccessMode,
    ) -> Result<(), SpaceError>;
}
```

当前实现可以是同进程直接调用，但接口不能依赖具体存储结构。

---

# 5. Agent Process：进程模型

## 5.1 核心定义

子 Agent 不应只是函数调用。

每个子 Agent 都应被建模为逻辑上的：

> **Agent Process**

```rust
pub struct AgentProcess {
    pub id: AgentId,
    pub parent_id: Option<AgentId>,
    pub kind: AgentKind,
    pub state: ProcessState,

    pub space_id: SpaceId,
    pub mailbox_id: MailboxId,
    pub namespace_id: NamespaceId,

    pub identity: AgentIdentity,
    pub permissions: PermissionContext,

    pub created_at: WallTime,
    pub deadline: Option<Deadline>,
    pub exit_status: Option<ExitStatus>,
}
```

---

## 5.2 生命周期

```text
Created
→ Ready
→ Running
→ Waiting
→ Suspended
→ Exited / Failed / Killed
```

基本操作：

```text
spawn
exec
wait
wake
signal
cancel
kill
inspect
reap
```

其中 `exec` 表示：

> 在保留 AgentId 和部分运行关系的前提下，更换 Agent Profile、任务、Prompt、Provider 或 Cognit 策略。

---

## 5.3 逻辑进程与操作系统进程分离

Agent Process 是逻辑抽象，不一定等于 Linux Process。

可支持多种后端：

```rust
pub enum AgentExecutionBackend {
    InProcessTask,
    Thread,
    LocalProcess,
    Container,
    RemoteNode,
}
```

建议：

- 普通 Planner、Reviewer：`InProcessTask`
- 不可信代码执行：`LocalProcess` 或 `Container`
- GPU 推理：`RemoteNode` 或独立 Worker
- 机器人实时控制：独立 Robot Runtime

---

# 6. Scheduler：调度模型

Aletheon 调度的不是单纯 CPU 时间，而是认知工作项。

```rust
pub struct WorkItem {
    pub process_id: AgentId,
    pub operation: Operation,
    pub priority: Priority,
    pub deadline: Option<Deadline>,
    pub required_resources: Vec<ResourceRequest>,
}
```

初期支持：

- Priority
- Deadline
- FIFO
- Dependency-aware
- Basic fairness

后续再考虑：

- Budget-aware
- Cost-aware
- Provider-aware
- GPU-aware
- Preemption
- Multi-node scheduling

当前不要一开始实现复杂调度算法。

---

# 7. Context Space：空间模型

## 7.1 空间的含义

Aletheon 中的“空间”首先不是物理空间，也不是 DDS。

空间表示：

> 一个 Agent 能看见、继承、修改、共享和提交哪些上下文与对象。

它更接近 Linux 虚拟地址空间。

---

## 7.2 三类空间

### 私有 Agent Space

每个 Agent Process 拥有：

```text
Private Context
Local Plan
Temporary Hypothesis
Tool Result
Scratchpad
Uncommitted Output
```

默认：

```text
Private by default
Shared explicitly
```

---

### Shared Cognitive Space

Agora 是一个特殊共享区域。

放置：

```text
当前全局任务
经过确认的事实
共享计划
协作状态
当前世界投影
公共决策
```

Agora 不等于全部上下文。

---

### External World Projection

真实世界资源包括：

```text
文件
进程
浏览器
机器人
传感器
网络
用户
远程节点
```

这些对象不应全部复制进入 Agora。

应通过投影：

```text
External World
    ↓ projection
Agora
    ↓ reasoning
Cognit
```

---

## 7.3 Copy-on-write 语义

子 Agent 创建时，不复制全部上下文。

采用：

```text
父上下文不可变快照
+
子 Agent 私有 overlay
+
显式 commit
```

```rust
pub struct ContextSpace {
    pub id: SpaceId,
    pub owner: AgentId,
    pub inherited_snapshot: Option<SnapshotId>,
    pub private_overlay: OverlayMap,
    pub shared_bindings: Vec<RegionBinding>,
    pub access_policy: SpaceAccessPolicy,
}
```

这在语义上对应：

| Linux | Aletheon |
|---|---|
| Process Address Space | Context Space |
| Private Memory | Private Context |
| Shared Memory | Shared Region |
| `mmap` | Attach Region |
| `fork` | Fork Space |
| Copy-on-write | Snapshot + Overlay |
| Page Permission | Region ACL |
| Swap | Mnemosyne Archive |
| Page Fault | Lazy Retrieval |

---

## 7.4 Agora 提交模型

Agent 不应直接写 Agora。

正确流程：

```text
Agent Local Context
    ↓ propose
Agora Transaction
    ↓ validate
Commit / Reject
```

```rust
pub enum AgoraOperation {
    PublishFact(Fact),
    ProposePlan(Plan),
    UpdateTask(TaskPatch),
    EmitObservation(Observation),
    ClaimSharedObject(ObjectId),
    ReleaseSharedObject(ObjectId),
}
```

---

# 8. Chronos：时间模型

Chronos 是宏内核基础子系统，不是普通工具。

## 8.1 四类时间

```text
Wall Time
Monotonic Time
Logical Time
Domain Time
```

### Wall Time

用于：

- 用户语义
- 日历
- 展示
- 记忆日期
- 审计日志

### Monotonic Time

用于：

- timeout
- duration
- heartbeat
- lease
- scheduling

### Logical Time

用于：

- 事件排序
- 版本顺序
- 分布式扩展
- 冲突检测

### Domain Time

用于：

- Session 时间
- Task 时间
- 仿真时间
- 机器人时间
- 回放时间

---

## 8.2 Chronos 职责

```text
now
timer
timeout
deadline
interval
lease
TTL
sleep
wake
event ordering
schedule
temporal query
```

统一接口：

```rust
pub trait Clock: Send + Sync {
    fn wall_now(&self) -> WallTime;
    fn monotonic_now(&self) -> MonoTime;
    fn logical_tick(&self) -> LogicalTime;
}
```

---

## 8.3 时间元数据

不能只保留一个 `timestamp`。

```rust
pub struct TemporalMetadata {
    pub created_at: WallTime,
    pub observed_at: Option<WallTime>,
    pub effective_from: Option<WallTime>,
    pub updated_at: Option<WallTime>,
    pub expires_at: Option<WallTime>,
    pub sequence: u64,
}
```

含义：

- `created_at`：记录创建时间
- `observed_at`：外部事件被观察到的时间
- `effective_from`：状态开始生效的时间
- `updated_at`：最近更新时间
- `expires_at`：状态失效时间
- `sequence`：逻辑顺序

---

# 9. IPC Fabric：通信模型

DDS 可以作为分布式扩展参考，但不等于空间系统。

## 9.1 基础原语

```text
Mailbox
Request / Response
Signal
Publish / Subscribe
Stream
Shared Region
```

---

## 9.2 Message Envelope

```rust
pub struct MessageEnvelope {
    pub id: MessageId,
    pub source: Endpoint,
    pub destination: Destination,

    pub sent_at: WallTime,
    pub sequence: u64,
    pub correlation_id: Option<MessageId>,

    pub deadline: Option<Deadline>,
    pub priority: Priority,
    pub delivery: DeliveryPolicy,

    pub payload: MessagePayload,
}
```

---

## 9.3 第一阶段 IPC

当前只实现：

```text
Mailbox
Request / Response
Signal
```

Pub/Sub、Stream、DDS 风格 QoS 后续增加。

可扩展 QoS：

```text
Reliable / Best Effort
Volatile / Retained
Keep Latest / Keep Last N
Deadline
Liveliness
Priority
```

---

# 10. Supervision：监督与恢复

Agent 不只是任务，也可能失败、卡死、超时或被取消。

Executive 应维护监督树：

```text
MainAgent
├── Planner
├── Reviewer
└── Executor
    ├── Shell Worker
    └── Browser Worker
```

监督策略：

```text
OneForOne
OneForAll
RestForOne
NeverRestart
RestartOnFailure
```

结构化退出原因：

```rust
pub enum ExitReason {
    Completed,
    Cancelled,
    DeadlineExceeded,
    BudgetExceeded,
    QuotaExceeded,
    PermissionDenied,
    ModelFailure,
    ToolFailure,
    Panic,
    Killed,
}
```

不能把所有失败都压缩成字符串错误。

---

# 11. Namespace：命名空间与隔离

Aletheon 应建立命名空间，而不是让所有 Agent 看见全部资源。

建议支持：

```text
Agent Namespace
Context Namespace
Capability Namespace
Resource Namespace
World Namespace
Secret Namespace
```

示例：

```text
namespace://personal
namespace://work
namespace://aletheon-dev
namespace://robot-lab
namespace://simulation
namespace://production
```

作用：

- 公司与个人数据隔离
- 多项目隔离
- 测试与生产隔离
- 机器人实例隔离
- 多用户隔离
- 密钥与敏感能力隔离

---

# 12. Permission：权限模型

Permission 只回答：

> 某个主体是否有资格执行某个操作。

不负责：

- 当前资源是否可用
- 是否超过并发限制
- 是否超预算
- 已使用多少 token

```rust
pub struct PermissionContext {
    pub principal: PrincipalId,
    pub namespace: NamespaceId,
    pub capability_tokens: Vec<CapabilityToken>,
    pub policies: PolicySet,
}
```

权限判断：

```text
Agent
→ Operation Request
→ Permission Engine
→ Allow / Deny / Require Approval
```

示例：

```text
read Agora task region：允许
write confirmed facts：需要 reviewer
execute shell read-only：允许
delete file：需要审批
control robot：需要 safety lease
```

---

# 13. 资源治理：重新划分边界

原先将 token、费用、并发、工具调用、GPU、权限、时间全部塞进 Resource Manager 的设计应废弃。

以下五个概念必须分开。

---

## 13.1 Resource Manager

只管理：

> 有限、可占用、可释放的运行资源。

例如：

```text
GPU slot
Model worker slot
Sandbox process slot
Browser instance
Robot control channel
Shared memory region
External device
```

核心动作：

```text
register
discover capacity
acquire
reserve
release
renew
reclaim
```

关键对象：

```rust
pub struct ResourceLease {
    pub id: ResourceLeaseId,
    pub owner: AgentId,
    pub resource: ResourceId,
    pub amount: ResourceAmount,
    pub acquired_at: MonoTime,
    pub expires_at: Option<MonoTime>,
}
```

---

## 13.2 Quota Manager

Quota 回答：

> 某个主体最多允许占用多少。

例如：

```text
最多创建 4 个子 Agent
最多同时运行 2 个模型调用
最多启动 3 个沙箱
最多持有 1 个机器人控制会话
```

区别：

```text
Resource：
当前还有没有 GPU slot？

Quota：
这个 Agent 最多允许占几个 GPU slot？
```

---

## 13.3 Accounting / Metering

Accounting 只负责记录使用量：

```text
输入 token
输出 token
模型调用次数
工具调用次数
GPU 时间
网络流量
存储量
费用
运行时间
```

```rust
pub struct UsageRecord {
    pub subject: AgentId,
    pub operation: OperationId,
    pub metric: UsageMetric,
    pub amount: u64,
    pub timestamp: WallTime,
}
```

Accounting 不负责拒绝请求。

---

## 13.4 Budget Controller

Budget 回答：

> 某个主体累计最多可以消耗多少。

例如：

```text
最多花费 2 美元
最多使用 100k token
最多运行 30 分钟
最多调用 20 次模型
```

Budget 建立在 Accounting 之上。

---

## 13.5 Permission

Permission 回答：

> 你是否被允许做这件事。

统一调用链：

```text
Agent Request
    ↓
Permission Check
    ↓
Budget Check
    ↓
Quota Check
    ↓
Resource Acquire
    ↓
Execute
    ↓
Accounting Record
    ↓
Resource Release
```

---

## 13.6 一个例子：模型调用

```text
Agent 请求调用模型
    ↓
Permission：
是否允许调用该模型
    ↓
Budget：
token / cost 是否还有余额
    ↓
Quota：
并发调用是否超过限制
    ↓
Resource：
获取 model worker / GPU slot
    ↓
Model Service：
执行推理
    ↓
Accounting：
记录 token、费用、耗时
    ↓
Resource：
释放 lease
```

---

## 13.7 一个例子：机器人控制

```text
Agent 请求控制机器人
    ↓
Permission：
是否拥有 robot.control 权限
    ↓
Quota：
是否允许持有控制会话
    ↓
Resource：
获取 robot-control lease
    ↓
Robot Runtime：
执行动作
    ↓
Accounting：
记录控制时长与命令
    ↓
Resource：
释放 lease
```

---

# 14. Kernel Object Model：统一对象模型

Aletheon 中许多实体具有共同属性：

```text
ID
Type
Metadata
Owner
Namespace
Permission
Lifecycle
Temporal Metadata
```

建议统一为：

```rust
pub trait KernelObject {
    fn object_id(&self) -> ObjectId;
    fn object_type(&self) -> ObjectType;
    fn metadata(&self) -> ObjectMetadata;
}
```

统一 URI：

```text
agent://main/123
process://agent/123
task://current
agora://facts/robot-state
memory://episodic/456
artifact://report/789
capability://linux/shell
model://provider/model
world://robot/kuavo-01
```

统一操作：

```text
resolve
open
read
write
watch
list
stat
close
```

这里不是把所有对象伪装成文件，而是提供统一寻址、权限和生命周期模型。

---

# 15. Kernel Services 的边界

## 15.1 Agora

Agora 是：

> 受事务、权限和时间约束的共享认知空间。

负责：

```text
Shared Facts
Active Tasks
Shared Plans
World Projection
Coordination State
Commit Events
Expiration
Snapshot
```

Agora 不是：

- IPC
- 全部 Context
- 全部世界状态
- 任意 Agent 可写的 HashMap

---

## 15.2 Mnemosyne

Mnemosyne 是：

> 持久化经验系统。

包含：

```text
Episodic Memory
Semantic Memory
Procedural Memory
Entity History
Task History
Memory Consolidation
Forgetting / Decay
Retrieval
```

它不负责决定当前 Agent 能看见什么。

这是 Space Manager 的职责。

---

## 15.3 Cognit

Cognit 是：

> 认知机制与策略集合。

包含：

```text
Reasoning
Planning
Review
Decision
Prompt Orchestration
Model Orchestration
```

一次具体认知活动运行在 Agent Process 中：

```text
Agent Process
    ↓
Cognit Strategy
    ↓
Model Service
    ↓
Local Result
```

Cognit 不是常驻的“大脑单例”。

---

## 15.4 Dasein

Dasein 表示：

```text
Identity
Goals
Values
Constraints
Continuity
Self Model
```

建议作为受保护的高权限系统服务或对象存在。

普通子 Agent 只读取授权投影，不应直接修改核心身份与价值结构。

---

## 15.5 Capability System

Capability 表示：

> 系统能够做什么。

Resource 表示：

> 执行该能力时需要占用什么。

```rust
pub struct CapabilityDescriptor {
    pub id: CapabilityId,
    pub operations: Vec<OperationSchema>,
    pub required_permissions: Vec<PermissionRequirement>,
    pub required_resources: Vec<ResourceRequirement>,
}
```

例如：

```text
Capability:
browser.navigate

Required resources:
browser instance
network channel
sandbox slot
```

---

# 16. 外部执行域

以下模块不应直接塞进宏内核：

```text
Linux Sandbox
Browser Runtime
Robot Runtime
GPU Worker
Remote Agent Node
Untrusted Plugin
Code Compiler
```

原因：

- 故障隔离
- 安全隔离
- 依赖复杂
- 可能需要不同语言
- 可能需要实时调度
- 可能需要远程部署

---

# 17. 机器人与 Aletheon 的边界

Aletheon 不应执行 1 kHz 控制循环。

推荐分层：

```text
Aletheon
    ↓ high-level intent
Robot Capability Plugin
    ↓ typed command
Robot Control Runtime
    ↓
RL / MPC / WBC / State Estimation
    ↓
Hardware
```

推荐实现方式：

```text
算法层：lib
集成层：plugin
实时执行层：独立 runtime
```

Aletheon 负责：

- 意图理解
- 任务规划
- Skill 选择
- 控制模式选择
- 安全授权
- 状态监督
- 故障恢复
- 世界状态摘要

Robot Runtime 负责：

- State Estimation
- MPC
- WBC
- RL Policy
- Safety Supervisor
- Hardware Interface
- Real-time Loop

---

# 18. 内核调用模型

Aletheon 应定义自己的 Kernel API。

```rust
pub enum KernelCall {
    Spawn(SpawnSpec),
    Wait(WaitSpec),
    Signal(SignalSpec),

    ReadObject(ReadSpec),
    WriteObject(WriteSpec),
    WatchObject(WatchSpec),

    Send(SendSpec),
    Receive(ReceiveSpec),

    AttachRegion(AttachRegionSpec),
    DetachRegion(DetachRegionSpec),

    InvokeCapability(InvocationSpec),

    AcquireResource(ResourceRequest),
    ReleaseResource(ResourceLeaseId),

    GetTime(TimeQuery),
    CreateTimer(TimerSpec),
}
```

Agent 不允许绕过 Kernel API 修改核心状态。

当前可用 Rust trait 直接调用；未来可切换为 channel、Unix socket 或远程 RPC。

---

# 19. 建议代码结构

```text
aletheon/
├── crates/
│   ├── kernel/
│   │   ├── process/
│   │   ├── scheduler/
│   │   ├── space/
│   │   ├── chronos/
│   │   ├── ipc/
│   │   ├── namespace/
│   │   ├── permission/
│   │   ├── supervision/
│   │   ├── object/
│   │   ├── syscall/
│   │   └── governance/
│   │       ├── resource.rs
│   │       ├── quota.rs
│   │       ├── accounting.rs
│   │       └── budget.rs
│   │
│   ├── services/
│   │   ├── agora/
│   │   ├── mnemosyne/
│   │   ├── cognit/
│   │   ├── dasein/
│   │   ├── model/
│   │   ├── capability/
│   │   ├── artifact/
│   │   ├── session/
│   │   └── audit/
│   │
│   ├── agents/
│   │   ├── runtime/
│   │   ├── profiles/
│   │   ├── planner/
│   │   ├── reviewer/
│   │   └── executor/
│   │
│   ├── adapters/
│   │   ├── cli/
│   │   ├── tui/
│   │   ├── linux/
│   │   ├── browser/
│   │   └── robot/
│   │
│   ├── protocol/
│   ├── storage/
│   └── common/
│
└── bin/
    └── aletheon/
```

注意：

> 不要一次性重构全部目录。

应先建立新的 Kernel 边界，让旧代码逐步通过 Adapter 接入。

---

# 20. 实施路线

## Phase 0：冻结术语与边界

正式确定：

```text
Agent Process
Context Space
Chronos
IPC Fabric
Namespace
Permission
Resource
Quota
Accounting
Budget
Kernel Object
Kernel Service
Capability
```

明确：

- Agora 不是 IPC
- Agora 不是所有 Context
- Mnemosyne 不是 Space Manager
- Cognit 不是 Agent Process
- Capability 不是 Resource
- Permission 不是 Quota
- Budget 不是 Resource
- systemd 只管理整个 Aletheon 实例

---

## Phase 1：Process + Chronos

实现：

```text
ProcessTable
spawn
state transition
wait
cancel
exit
deadline
timeout
heartbeat
```

验收：

- 父 Agent 可创建子 Agent
- 子 Agent 有独立 ID
- 父 Agent 可 wait
- 超时后内核可 cancel
- 返回结构化 ExitReason

---

## Phase 2：IPC + Supervision

实现：

```text
Mailbox
Request / Response
Signal
Supervisor Tree
```

验收：

- Planner 可向 Reviewer 发送请求
- Reviewer 返回带 correlation_id 的结果
- Reviewer 异常退出后可被监督重启
- 父 Agent 取消时子 Agent 收到信号

---

## Phase 3：Context Space

实现：

```text
Immutable Snapshot
Private Overlay
Fork Space
Attach Region
Explicit Commit
```

验收：

- 子 Agent 可读取父上下文快照
- 子 Agent 本地修改不污染父 Agent
- 只显式提交结果
- 私有空间互不可见

---

## Phase 4：Namespace + Permission

实现：

```text
Namespace Binding
Capability Token
Read / Write Policy
Approval Requirement
```

验收：

- Reviewer 不能调用 Shell
- Executor 只能调用受限 Shell
- Work Namespace 看不到 Personal Namespace
- 未授权 Agent 不能写 Agora

---

## Phase 5：Resource + Quota + Accounting + Budget

实现最小版本：

```text
Resource Lease
Quota Check
Usage Record
Budget Rule
```

验收：

- Agent 获取和释放资源 Lease
- 超过并发 Quota 时被拒绝
- 每次模型调用产生 UsageRecord
- 超过 token 或费用 Budget 时停止

---

## Phase 6：Agora 事务化

实现：

```text
Proposal
Validation
Commit
Conflict Detection
TTL
Commit Event
Snapshot
```

验收：

- Agent 不能直接覆盖全局事实
- 冲突可检测
- Entry 可过期
- 订阅者收到 Commit Event

---

## Phase 7：迁移 Cognit

将：

```text
Planner
Reviewer
Executor
```

从硬编码流程改成 Agent Profiles。

```text
Agent Profile
├── Cognit Strategy
├── Model Policy
├── Capability Set
├── Context Binding
├── Permission
├── Quota
└── Budget
```

---

## Phase 8：外部 Runtime

接入：

```text
Shell Sandbox
Browser Runtime
Robot Runtime
GPU Worker
Remote Node
```

这些能力通过统一协议接入，不进入宏内核核心。

---

# 21. 当前优先级

## P0

```text
1. Agent Process
2. Chronos
3. Context Space
4. IPC Fabric
5. Supervision
6. Kernel Object Model
```

## P1

```text
7. Namespace
8. Permission
9. Resource Lease
10. Quota
11. Accounting
12. Budget
13. Agora Transaction
```

## P2

```text
14. Advanced Scheduler
15. Distributed Node
16. DDS-style QoS
17. Remote Agora
18. Robot Integration
19. GPU Worker
20. Multi-user Isolation
```

---

# 22. 当前不要做的事情

暂时不要：

- 把所有模块拆成独立 systemd 服务
- 做完整微服务架构
- 实现完整 DDS
- 实现页级 Copy-on-write
- 做复杂分布式一致性
- 做抢占式 Agent 调度
- 让 LLM 直接修改内核状态
- 把机器人高频 Telemetry 写入 Agora
- 把 MPC/WBC/RL 放进 Aletheon 主进程
- 一次性重写全部现有代码

---

# 23. 最终架构结论

Aletheon 下一阶段应正式定位为：

> **一个单实例、宏内核式、内部服务化、外部执行域隔离的 Agent Runtime。**

宏内核负责定义：

```text
Process
Time
Space
IPC
Scheduling
Supervision
Namespace
Permission
Resource Lease
Quota
Accounting
Budget
Object Model
```

内核服务负责：

```text
Agora
Mnemosyne
Cognit
Dasein
Model
Capability
Artifact
Session
Audit
```

Agent Process 负责：

```text
Planner
Reviewer
Executor
Main Agent
Specialist Agent
```

外部 Runtime 负责：

```text
Robot
Browser
Sandbox
GPU
Remote Worker
```

最终形成：

```text
Aletheon Macro-kernel
    +
Agent Processes
    +
Virtual Context Spaces
    +
Temporal Runtime
    +
IPC Fabric
    +
Governance
    +
Cognitive Services
    +
External Execution Domains
```

这套结构的核心，不是模仿 Linux 的外形，而是继承 Linux 最重要的设计思想：

> **由统一内核严格定义运行语义，由受控接口管理进程、空间、时间、通信、权限和资源，让上层能力能够长期演化而不破坏系统基础。**

# Aletheon Core V2 完整架构重构实施计划

> 状态：Draft / Proposed  
> 日期：2026-08-08  
> 基线：`dev@bd1ceac2965832f15cf45b811fd34e052e7e9a67`  
> 目标：完成核心所有权、执行路径、持久化和进程拓扑重构  
> 性质：实施计划，不是产品功能路线图  
> 目标分支：`dev`

## 0. 本计划解决什么

Aletheon 当前真正的问题不是 crate 数量多，也不是目录名字不好，而是同一种状态、同一条执行路径和同一种权力分散在多个模块：

- `executive` 同时承担 Runtime、Application、Host、Adapter、业务扩展和兼容层；
- `fabric` 同时容纳公共 ID、领域模型、UI 协议、Robot/Gmail 类型、IPC、Policy 和基础设施；
- `kernel` 已有 Process、Operation、Budget、Lease、Space、Mailbox 等实现，但还不是所有 Agent-requested governed capability 不可绕过的机制根；
- 当前 `runtime` 只负责外部 runtime manifest/selector，并不是真正的 Agent Runtime；
- Runtime、Kernel、Dasein、Metacog、Agora、Mnemosyne 之间仍有事实和状态写入权重叠；
- Gmail、Robot VLA、Hardware 已有可用或有价值的实现，但其代码横跨 Executive、Fabric、Cognit、Gateway、Hardware 和共享存储，重构时很容易被误删；
- 大量 mock/白盒测试没有阻止真实安装态出现上下文污染、等待超时、终态证据缺失和密钥暴露等问题。

这次“完全重构”的含义是：最终所有生产路径全部收敛到新的边界，不永久保留旧 Executive/Fabric 兼容路径。它不意味着用一个 Mega PR 一次性重写 12 万行代码。

最终必须做到：

1. 每种 durable fact 只有一个 journal writer；
2. 每个 Agent Turn 只有一个语义状态机；
3. 每个 Agent-requested governed capability 只能通过一个 Kernel enforcement path；
4. 每个领域类型、port、repository 和 receipt 都有唯一所有者；
5. Application、Gmail、Robot、Hardware 可以拔掉，而核心仍能编译运行；
6. Gmail、Robot VLA、Hardware、Pi、GBrain 的已验证能力在迁移期间不丢失；
7. 崩溃后只能恢复、补偿或进入待核对状态，绝不推测成功；
8. 不再以 crate 名字、测试数量或 mock PASS 冒充架构和产品稳定性。

---

## 1. 决策分级

旧计划把太多事项写成“不可逆转决策”。V2 将决策分成三类。

### 1.1 不可违反的架构约束

- 保留并继续发展 Dasein 自我意识与 Metacog 元认知；
- `fabric` 最终退出，名称改为 `contracts`，不命名为 `abi`；
- `contracts` 不是共享领域模型仓库，也不是新的 Fabric；
- Runtime 是 Agent、Session、Turn、Delegate 的唯一语义权威；
- Kernel 只负责执行机制和强制执行，不拥有 Agent 语义；
- Executive 最终从 active workspace 和生产依赖图删除；
- TUI 是 Presentation，Gateway 和 Application 是外层，不进入核心；
- Gmail、Robot VLA、Hardware 是受支持扩展，不是待删除的历史包袱；
- Robot 的实时控制环、WBC、MPC、驱动和急停控制器不进入 Aletheon；
- Linux 是 V2 唯一生产平台；Windows、macOS、Android 和 Embedded 的新增适配延期；
- 所有 Secret 值不得进入 Contracts、事件、日志、命令行参数或长期记忆；
- 新旧 daemon 不得同时写同一份状态数据库。

### 1.2 必须通过代码和运行证据确认的设计假设

- Kernel 是否需要保留独立 crate，而不是最终收缩成 `runtime::execution`；
- 当前 Gmail 的真实支持范围是读取/同步、Goal Draft、受控发送中的哪些部分；
- Robot VLA 当前能稳定保证到 Simulation、Bridge、HIL 中的哪一级；
- `execd` 是否有独立存在的价值；
- machine/system daemon 是否仍有不可替代的无状态职责；
- Agora 哪些 durable log 仍有诊断价值；
- 原有 Fabric 类型中哪些真正需要跨领域共享。

这些假设不能靠文档宣告成立。Phase 0 必须用调用图、运行态证据和存储清单确认。

### 1.3 可回退的迁移选择

- 新旧 port 的临时 adapter；
- 每个里程碑的 feature flag；
- 同一 SQLite 文件内的逻辑分库，或独立数据库文件；
- `core-linux` 与 `full-linux` 的具体 Cargo feature 组合；
- Gmail/Robot worker 暂时由 daemon 内任务还是受监督子进程承载；
- Fabric 兼容 re-export 的短期保留周期。

每个可回退选择都必须写清删除条件、回滚路径和最晚移除里程碑。

---

## 2. 最新 `dev` 的事实基线

以下内容来自本计划编写时的真实源码，不是目标架构假设。

| 区域 | 当前事实 | 重构含义 |
|---|---|---|
| `executive` | 直接依赖 Fabric、Gateway、Kernel、Agora、Cognit、Corpus、Mnemosyne、Runtime、Dasein、Metacog、Hardware | 它是当前 God Crate，职责必须逐项迁出，不能整体改名 |
| `runtime` | 目前只有 external runtime manifest 与 selector | 不能假装它已经是 Agent Runtime；需要纵向迁入 Turn/Session/Agent 权威 |
| `kernel` | `KernelRuntime` 持有 Process/Operation/Space/Mailbox/Admission/Budget/Lease/Supervision 等多组内存表和 getter | Kernel 已经有膨胀趋势，应先缩小再成为 enforcement root |
| `fabric` | 包含 Agent、Turn、Operation、Approval、Robot、Embodiment、UI protocol、IPC、Policy、Repository 等大量模型与实现 | 必须先按所有者拆出，最后才进行机械 rename |
| Gmail | 实现横跨 Executive Gmail/Google adapters、Goal/Approval、Gateway、共享 SQLite migration 和验收脚本 | 先建立保存舱和等价 smoke，后迁移 |
| Robot VLA | Cognit robot harness、Executive robot composition/episode、Fabric embodiment 类型、Hardware bridge 相互交叉 | 不能简单“移出核心”后删除；必须定义独立扩展所有者 |
| Hardware | 已有 simulation、gRPC bridge、lease、safety、emergency-stop 等；源码明确声明真实执行器暂不支持 | 当前只能按证据宣称 Simulation/Bridge，不能把安全代码等同实机验收 |
| Dasein | 同时包含 policy facade、identity、care、boundary、continuity 和 mutation | 需要拆清 Owner Manifest、SelfModel 与 mutation authority |
| Metacog | 已有 evidence/evaluation/problem/reflection/evolution/governance | 保留活跃闭环，但不能让 proposal 自己批准或直接改源码/安全策略 |
| Agora | 既有内存 workspace，也有持久化 commit/broadcast | 需明确持久化内容只是记录/观察，不能成为 Session/Turn 权威 |
| Mnemosyne | 同时包含领域模型、SQLite/remote adapter、GBrain supplemental memory | 保留记忆领域，具体 adapter 必须退出核心依赖面 |

这张表是迁移起点，不是对当前架构的认可。

---

## 3. 范围与产品取舍

### 3.1 活跃核心

V2 只主动建设以下核心闭环：

- Agent、Session、Turn、Agent Run、Delegate 生命周期；
- Native Cognit 与受监督的外部 Agent Runtime（Pi）；
- Cognit 单 Turn 认知循环；
- Dasein 身份、自我模型、价值边界、承诺和连续性；
- Metacog 跨 Turn 观察、评估、问题发现和改进提案；
- Agora 当前认知工作集；
- Mnemosyne 长期记忆、召回、固化和遗忘；
- Kernel Operation、Cancellation、Deadline、Permit、Budget、Lease、Accounting、OS child supervision；
- Linux 上必要的 Provider、Tool、Sandbox、SQLite 和 Secret adapters；
- 一个 authoritative user daemon；
- 最小 Application、Gateway、CLI/TUI；
- 安装态 smoke、Nightwatch 和 tester 工程设施。

### 3.2 受支持扩展

以下能力不属于 Agent Core，但必须继续维护和迁移：

- Gmail inbound/sync/Goal/Approval/report 等已验证路径；
- Robot VLA 高层任务与认知路径；
- Hardware simulation、现有 bridge、lease、watchdog、safe-stop 和 receipt；
- Pi external Agent Runtime；
- GBrain supplemental memory；
- 当前真实可用的 Linux channel/provider/tool adapters。

“受支持”意味着：

1. 有明确 owner；
2. 有独立 build profile；
3. 有配置和 Secret 迁移规则；
4. 有 installed black-box smoke 或 safety lane；
5. 核心重构后仍能完成迁移前已经通过的行为；
6. 默认运行时可以关闭，但不能靠删除代码实现关闭。

### 3.3 延期扩张

以下方向不是删除，而是在 Core V2 完成前不新增能力：

- Windows、macOS、Android、Embedded 生产适配；
- 新的 Robot skill marketplace、大规模 skill catalog 和新机器人型号；
- 未进入真实调用链的 eBPF、FUSE、io_uring 等平台实验；
- 新 channel、新应用产品工作流和复杂 dashboard；
- 多主体、多 Self 社会系统；
- Agent 自动修改源码、架构、安全策略或身份核心；
- Aletheon 内部的硬实时机器人控制。

现有 Gmail、Robot VLA 和 Hardware 不属于本节。

### 3.4 明确删除候选

只有满足以下全部条件的代码才进入删除候选：

- 没有生产调用者；
- 没有受支持扩展依赖；
- 不保护安全、恢复或数据迁移；
- 有 Git 历史可追溯；
- 删除后 `core-linux` 与 `full-linux` smoke 均不退化。

名称漂亮、测试难维护或暂时关闭都不是单独删除理由。

---

## 4. 从 Linux 借鉴的不是目录，而是边界纪律

Linux 解耦较好的关键不是“Kernel 什么都管”，而是几个长期稳定的原则：

### 4.1 机制与策略分开

- Kernel 提供 operation、permit、资源和取消机制；
- Runtime 决定 Agent Turn 的语义推进；
- Dasein 决定意图和行动是否符合 Self/Owner 边界；
- Hardware 独立决定物理动作是否安全；
- Application 决定用户用例和审批交互。

Kernel 不替 Runtime 规划，Runtime 不替 Hardware 判断设备安全。

### 4.2 数据靠所有者，而不是靠“公共目录”组织

一个类型即使被多个 crate 使用，也不自动进入 Contracts。先问：

1. 谁创建它？
2. 谁验证它？
3. 谁改变它？
4. 谁持久化它？
5. 谁负责 schema migration？

答案指向谁，类型和 port 就优先归谁。

### 4.3 窄接口代替组件 getter

禁止再出现一个可以取出 ProcessTable、BudgetController、MemoryStore、Provider、Dasein 等所有组件的中央对象。

核心组件只暴露完成其职责所需的 command/query port。调用方不能拿到底层表和 repository 绕过状态机。

### 4.4 控制流显式，事件不做万能路由

- Command：请求某个 owner 改变状态；
- Journal Event：owner 已经持久化的事实；
- Notification：把事实通知给其他模块；
- Observation/Telemetry：非权威观测；
- Read Model：可丢弃、可重建投影。

事件不能替代明确的 Runtime → Dasein → Kernel 调用链。

### 4.5 Composition Root 在最外层

`aletheon` binary 只负责：

- 读取配置；
- 构造 adapters；
- 注入 ports；
- 启动、停止和健康检查；
- 输出 build/runtime manifest。

它不得实现领域规则、repository 或兼容逻辑。

---

## 5. 目标运行拓扑

```text
TUI / CLI
    |
    v
Gateway + Application client protocol
    |
    v
authoritative user daemon
    +-- Runtime
    +-- Kernel
    +-- domain services
    +-- SQLite adapters / Secret adapters
    +-- supervised Pi/MCP children
    +-- supervised Gmail workers
    +-- optional execd helper
    +-- connection to external Robot Bridge

Nightwatch / tester
    +-- read-only observation and black-box commands
```

### 5.1 唯一 Agent 权威进程

V2 默认只允许 user daemon 拥有：

- Agent/Session/Turn journal writer；
- Kernel execution journal writer；
- Self/Metacog service composition；
- official user socket；
- 当前用户的数据目录。

TUI、CLI、Gmail poller、Pi、execd 和 Robot Bridge 都不能成为第二个 Agent Runtime。

### 5.2 system/machine daemon

目标不是保留双 daemon 以显得系统化。

- 如果当前 machine `core` 只重复 user daemon 的 Runtime/Session 职责，则删除；
- 如果确有跨用户资源或特权操作需求，它只能是无 Agent 状态的 broker；
- 必须使用独立 socket、principal、data directory 和数据库；
- 不得持有 Session、Turn、Dasein、Memory 或用户 Secret；
- 不得与 user daemon 同写任何 SQLite 文件。

### 5.3 `execd`

`execd` 不是第二个 Capability Authority。它只能：

- 接受 Kernel 签发的有界、一次性、可验证 execution grant；
- 在限定 workspace、network、deadline、resource 和 credential scope 内执行；
- 返回 host-owned receipt；
- grant 无效、过期或无法验证时 fail closed。

如果没有独立进程隔离价值，则在审计后合并为 Kernel 的 Linux executor adapter。

### 5.4 Robot Bridge

Robot Bridge 是外部实时/设备系统，不是 Aletheon daemon 的子 Runtime。

- Bridge 丢失 daemon heartbeat/lease 后必须能独立 safe-stop；
- Aletheon 只提交高层、有界、可取消命令；
- WBC/MPC/驱动/急停硬实时循环留在 Bridge 或机器人控制系统；
- Aletheon 的“成功”必须引用 Bridge/Hardware receipt，不能引用 LLM 文本。

---

## 6. 权威与事实所有权

### 6.1 唯一写入权威表

| 事实 | 唯一 owner | Durable port/store | 其他模块能做什么 |
|---|---|---|---|
| Owner Manifest | 用户/管理员 | Owner-owned manifest store | Dasein 只读，系统不可自主修改 |
| SelfModel/Commitment/Continuity | Dasein | `SelfStore` / Self journal | Runtime 请求变更，Metacog 只能提案 |
| Meta evidence/problem/proposal/evaluation/experiment | Metacog | `MetaStore` / Meta journal | 不能把自己的 proposal 标记为 approved/adopted；关联普通 Runtime Turn receipt |
| Scoped experiment execution | Runtime | 普通 Runtime Turn/Agent stream | 不新增 experiment state machine；只执行 scoped overlay Turn |
| Prompt/config/route adopted version | 对应配置 owner | owner-specific config journal/store | “实验成功”和“正式采用”是两个决策 |
| Agent/Delegate lifecycle | Runtime | `RuntimeJournal::AgentStream` | Kernel 只返回 execution facts |
| Session/Turn/settlement/recovery | Runtime | `RuntimeJournal::{SessionStream, TurnStream}` | Gateway 读取 projection facts |
| ExecutionProcess/Operation/Permit/Resource receipt | Kernel | `ExecutionJournal` | Runtime 保存引用，不复制权威状态 |
| Approval request/resolution | Application | `ApprovalStore` | `DecisionRequestId` 只做关联；Kernel/Dasein 通过各自 verifier mint opaque grant |
| Active cognitive workspace | Agora | 默认内存；可选 observation log | 不用于 Session/Turn 恢复 |
| Long-term memory | Mnemosyne | Memory repository | 只能作为带来源的召回证据 |
| Capability catalog | Corpus | Catalog repository | Kernel 通过 port 调用 executor |
| Gmail cursor/dedupe/message/goal draft | Gmail extension | Gmail store | Runtime 只接收 External Stimulus/Turn |
| Robot proposal/episode/evaluation | Robot VLA extension | Robot episode store | Runtime 保存相关 receipt 引用 |
| Device state/lease/safety/command receipt | Hardware | Hardware/Bridge store | Kernel permit 不能覆盖 Hardware safety veto |
| Secret value/token | Secret/credential adapter | 专用 secret store | 只在执行边界短暂注入 |

可以共享一个物理 SQLite 文件，但不能共享逻辑写入权。每张表、每个 stream、每个 migration 必须在清单中标注唯一 owner。

`RuntimeJournal` 是 Runtime 的底层 durable port，内部提供独立 `AgentStream`、`SessionStream` 和 `TurnStream`；表中的 `AgentJournal` 表示其 Agent stream，不是第二个 repository。`AgentExecutionBinding` 属于 AgentRun/AgentStream aggregate。Agent、Session、Turn 的跨 stream 推进使用 saga + correlation/idempotency，不假装多个 stream 或 Runtime/Kernel journals 之间存在原子事务。

### 6.2 Event 分类

```text
JournalEvent
  authoritative + versioned + replayable + single writer

Notification
  delivery mechanism; loss/retry semantics explicit

Observation / Telemetry
  non-authoritative; can be sampled or dropped

ReadModel
  derived; can be deleted and rebuilt
```

禁止再使用“一个全局 Event authority”这种表述。正确约束是：每个 aggregate 只有一个 journal writer。

### 6.3 Projection 不是 Authority

TUI snapshot、Session list、Agent tree、progress、metrics 和 Nightwatch verdict 都只是 projection。

当 projection 与 journal、receipt、running PID、daemon log 冲突时，以 journal/receipt/真实进程为准，并把 projection 判为故障。

---

## 7. 目标 workspace 与依赖方向

### 7.1 目标结构

```text
crates/
├── contracts/              # 极小共享 primitives，不是领域模型仓库
├── kernel/                 # execution enforcement boundary；可审计收缩为 runtime::execution
├── runtime/                # Agent/Session/Turn semantic runtime
├── cognit/                 # 单 Turn cognition
├── dasein/                 # Self domain
├── metacog/                # 跨 Turn 元认知
├── agora/                  # 活跃认知工作集
├── mnemosyne/              # 长期记忆领域
├── corpus/                 # capability catalog/executor adapters
├── application/            # 通用 use cases 与 approval
├── gateway/                # RPC/channel protocol
├── interact/               # TUI/CLI presentation
├── hardware/               # embodiment/device safety domain
├── extensions/
│   ├── gmail/
│   └── robot-vla/
├── adapters/
│   ├── linux/
│   ├── sqlite/
│   ├── pi/
│   ├── provider/
│   └── gbrain/
├── execd/                  # 可选 bounded executor helper
└── aletheon/               # 唯一 composition root/binary
```

这是一张目标所有权图，不要求为了目录美观把每个纯内存 adapter 都变成独立 crate。但任何引入 `reqwest`、`rusqlite`、`tonic`、`nix`、system path/process 或平台 SDK 的实现都必须成为独立 package/crate；需要编译隔离时不能依赖 Cargo feature 假装分层。

### 7.2 允许依赖

```text
contracts     -> no workspace crate
kernel        -> contracts
cognit        -> contracts
dasein        -> contracts
metacog       -> contracts
agora         -> contracts
mnemosyne     -> contracts
runtime       -> contracts, kernel, cognit, dasein, metacog, agora, mnemosyne
corpus        -> contracts, kernel ports
application   -> contracts, runtime
gateway       -> application
interact      -> gateway
hardware      -> contracts, kernel CapabilityExecutor/receipt ports
gmail         -> gateway/application, kernel descriptor/executor/resource ports
robot-vla     -> cognit CognitiveSession, runtime extension ports, hardware
provider      -> cognit InferenceService, kernel generic resource port
pi            -> runtime ExternalAgentExecutor, kernel process/lease ports
gbrain        -> mnemosyne supplemental port, kernel generic resource port（reconcile 由 GBrain adapter 拥有）
adapters      -> only the ports they implement/use
aletheon      -> all concrete components required for composition
```

### 7.3 禁止依赖

- Contracts 不依赖任何 workspace crate；
- Kernel 不依赖 Runtime、Cognit、Dasein、Metacog、Agora、Memory、TUI、Gmail、Robot 或具体数据库；
- 领域 crate 不依赖 Application、Gateway、Interact 或 composition root；
- Runtime 不依赖 TUI、Unix socket、systemd、HTTP client、SQLite、Gmail、Robot 或具体 Provider；
- Cognit 不直接执行 Tool/Hardware 副作用；
- Dasein/Metacog 不直接获取 Kernel CapabilityBroker；
- Gateway 不包含 Agent 决策逻辑；
- Application 不构造具体 adapter；
- TUI 不访问 repository；
- extensions 不向 Contracts 注入自己的领域类型；
- `aletheon` 不实现领域规则。

### 7.4 Port 所有权矩阵

Port 必须由需要保护该不变量的一侧拥有，而不是统一塞入 Contracts：

| Port | 定义 owner | 实现方 |
|---|---|---|
| `ExecutionJournal` | Kernel | SQLite adapter |
| `CapabilityExecutor` | Kernel | Corpus/Gmail outbound/Hardware executor dispatcher |
| `CapabilityDescriptorSource/Registrar` | Kernel | Corpus/extensions at bootstrap; registry is sealed before serving |
| `AuthorizationEvidenceVerifier` | Kernel | outer adapter backed by Application ApprovalStore/owner evidence |
| `ProcessController` | Kernel | Linux/execd adapter |
| `RuntimeJournal` | Runtime | SQLite adapter |
| `ExternalAgentExecutor` | Runtime | Governed Pi adapter backed by Kernel process permit/lease; no direct OS spawn |
| `CognitiveSession` | Cognit | Native Cognit / Robot VLA session adapter |
| `InferenceService` | Cognit | Governed provider adapter using Kernel generic resource + provider-owned admission |
| `SelfPolicy` | Dasein | Dasein service |
| `SelfStore` / `OwnerManifestSource` | Dasein | SQLite/file/admin adapter |
| `OwnerAuthorizationVerifier` | Dasein | outer adapter backed by Application ApprovalStore/owner evidence |
| `MetaObserver` / `MetaStore` | Metacog | Metacog service / SQLite adapter |
| `WorkspacePort` | Agora | Agora service |
| `MemoryPort` | Mnemosyne | local memory / GBrain supplemental composition |
| `ApprovalStore` | Application | SQLite adapter |
| public RPC/channel protocol | Gateway | Unix socket/Telegram/Gmail transport adapters |

如果一个 trait 只是为了让实现方避免依赖真正 owner，而被放到 Contracts 中，这通常说明依赖方向设计错了。

### 7.5 Domain core 与具体 Adapter 分离

当前 Cognit、Dasein、Agora、Mnemosyne、Gateway 等 crate 仍直接携带 `reqwest`、`rusqlite`、`tonic` 或 host 配置。目标状态要求：

- Runtime 依赖的是领域 API/port，不因依赖领域就拉入 HTTP、SQL、gRPC 或 system path；
- 领域的默认 core feature 不构造 concrete adapter；
- SQLite、remote provider、gRPC、Linux 和 path/config 实现迁入独立 adapter package/crate；
- `runtime` 对领域依赖使用最小 feature set；
- `core-linux` 的 dependency gate 同时检查 feature resolution，而不只检查直接 Cargo.toml 边。
- Dasein/Metacog/Agora/Mnemosyne 不再只为 `Clock` 反向依赖 Kernel；Runtime 在 domain command/event metadata 中传入可信时间，或领域拥有自己的窄 `TimeSource` port。

Cargo feature 只用于同一层内部的行为选择，不承担架构隔离。最终 binary 的 feature 会统一解析；如果 provider feature 仍能让 Runtime 对 Cognit 的依赖间接拉入 HTTP client，就说明隔离失败。

不完成这一步，即使目录图正确，Runtime 仍会通过领域 crate 间接重新依赖所有基础设施。

---

## 8. Fabric 的拆除与 Contracts 的定义

### 8.1 命名

最终采用：

```text
目录       crates/contracts
package    contracts
Rust path  contracts
```

不使用：

- `abi`：本项目当前需要源码级协议和数据边界，不承诺稳定二进制 ABI；
- `common`、`base`、`types`：容易重新成为垃圾场；
- `core`：会与 Agent Core 混淆；
- `fabric`：当前名称已经承载了过多错误职责。

### 8.2 Contracts 允许拥有的内容

只允许没有自然领域 owner、且确实跨多个独立边界使用的基础 primitives：

```text
contracts/src/
├── ids.rs             # AgentId/SelfId/SessionId/TurnId/PrincipalId/DecisionRequestId/ActionBindingId
├── correlation.rs     # CorrelationId/CausationId/IdempotencyKey
├── schema.rs          # SchemaId/EnvelopeVersion
├── event_meta.rs      # event header metadata，不含领域 payload
└── digest.rs          # ActionDigest 与 stable content/build digest primitives（确有调用者时）
```

进入 Contracts 必须同时满足：

1. 没有更自然的领域 owner；
2. 至少跨两个独立领域或外部持久化/协议边界；
3. 语义稳定，不依赖某个 workflow；
4. 不需要 I/O、异步任务、数据库、网络或系统调用；
5. public surface 有真实消费者；
6. 通过 Architecture Review 记录理由。

“被两个 crate 引用”不是充分条件。

Contracts 中的 ID 只负责关联，不是 authority：仅凭 `AgentId`、`SelfId`、`PrincipalId`、`DecisionRequestId` 或任何 receipt-like ID，不得授权、恢复绑定或访问数据。权限必须同时验证 authenticated principal、generation/revision、scope、digest、expiry 和不可伪造 evidence。

### 8.3 明确不进入 Contracts

| 类型 | 目标 owner |
|---|---|
| Agent/Session/Turn commands、events、outcomes | Runtime |
| ExecutionProcess/Operation/Permit/Budget/Lease/Usage | Kernel |
| SelfVerdict/SelfTransition/Commitment | Dasein |
| MetaChange/Evaluation/Experiment/Rollback | Metacog |
| Plan/CognitiveStep/Reflection | Cognit |
| Workspace frame/task graph | Agora |
| Memory record/query/retention | Mnemosyne |
| Capability definition/executor schema | Corpus/consuming domain |
| public RPC DTO/error code/UI snapshot | Gateway |
| Approval workflow | Application |
| Gmail/Google/channel-specific types | Gmail extension/Gateway |
| Robot/VLA/episode types | Robot VLA extension |
| Embodiment/device/safety types | Hardware |
| path/config/systemd/provider/database types | concrete adapter |

### 8.4 Live Permit 不能是普通数据

当前可公开构造的 `ExecutionPermit` 会使 Kernel enforcement 可被伪造。V2 必须改成：

- in-process permit 是 Kernel 私有、opaque、不可 serde 的 handle；
- 外部 `execd` grant 使用签名、nonce、expiry、scope 和 replay protection；
- 其他领域只能看到不可执行的 `PermitReceipt`/projection；
- Permit 必须由 Kernel `admit` 产生，并由 Kernel `invoke/settle/revoke` 消费；
- 任何手工构造或绕过 broker 的调用 fail closed。

### 8.5 Port 和错误的归属

- Repository port 跟随拥有该状态的领域；
- Executor port 跟随强制调用它的领域；
- Gateway 负责把领域错误映射成公共 RPC error；
- 不建立全局 `Error` enum；
- 不建立全局 trait registry；
- 不做永久 flat re-export；
- 不要求所有内部类型都实现 `serde`。

### 8.6 正确迁移顺序

```text
冻结 Fabric 新增 public surface
-> 生成类型/trait/调用者/持久化 owner 清单
-> 将类型和 port 迁回 owner
-> 用短期单向 re-export 保持可编译
-> 收缩 Fabric 到只剩允许的 primitives
-> 最后做 fabric -> contracts 机械 rename
-> 删除所有 alias/re-export
```

不能先把完整 Fabric 改名，再慢慢清理。

---

## 9. Kernel V2：最小而不可绕过

### 9.1 Kernel 的价值判断

Kernel 不靠职责多证明有用，只靠它强制保护的不变量证明有用。

V2 第一版只保留四组机制：

1. `OperationSupervisor`；
2. `CapabilityBroker`；
3. `ResourceLedger`；
4. `Clock` 与 OS child supervision。

“Kernel”在本计划中首先表示 execution enforcement boundary。默认实现为独立 `kernel` crate；如果迁移完成后它只有 Runtime 一个调用者，且独立 crate 没有形成编译隔离或安全边界，则允许物理收缩为 `runtime::execution`。无论物理位置如何，本文关于 owner、ports、journal、禁止依赖和 authority 的约束完全不变。不能为了名字保留空层，也不能借合并让 Runtime 绕过该边界。

### 9.2 `OperationSupervisor`

拥有：

- `ExecutionProcessId`、`OperationId`、`ExecutionEpoch`；
- operation tree；
- deadline、timeout 和 cancellation token；
- descendant cancellation/cleanup；
- OS process group/cgroup 绑定；
- child exit 与 terminal execution receipt；
- generation/epoch fencing；
- no-progress/stuck observation；
- exactly-once terminal settlement。

不拥有：

- AgentId 的语义生命周期；
- Session、Turn、Agent Profile；
- parent/child Agent 的委派含义；
- TUI connection/thread ownership；
- prompt、context、memory 或 provider route。

Kernel 中不再使用“Agent Process”称呼。统一称为 `ExecutionProcess` 或 `ExecutionTask`。

### 9.3 `CapabilityBroker`

Agent-requested governed capability 的唯一合法路径：

```text
Runtime Action Proposal
  -> Dasein/Owner policy verdict（按风险需要）
  -> Kernel begin Operation
  -> Kernel resolve sealed EnforcementDescriptor
  -> Kernel verify DecisionRequestId evidence（按规则）
  -> Kernel admit(descriptor_digest, invocation_digest, authority, budget, deadline)
  -> opaque Permit
  -> Kernel invoke(permit_id, invocation)
  -> Kernel durable Receipt
  -> Kernel settle/revoke
  -> Runtime durable Turn/Agent settlement
```

Kernel 必须自己从 sealed registry 解析并调用 `CapabilityExecutor`，不能让 Runtime 传入 trait object，也不能把 Permit 交给任意模块后由其自行执行。

完整 tool schema 和领域校验仍由 Corpus/extension 拥有。Kernel 只保存强制执行所需的 `EnforcementDescriptor`：

```text
EnforcementDescriptor {
    capability_id,
    effect_class,
    scope_model,
    idempotency_class,
    reconcile_class,
    approval_requirements,
    resource_requirements
}
```

composition 启动时将 `EnforcementDescriptor + executor registration` 注册到 Kernel 内部 versioned registry，完成后 seal。Permit 必须绑定 `descriptor_digest + executor_registration_id + registry_revision + operation_epoch + invocation_digest`。Operation 活跃期间对应 registry entry 不可替换；升级必须创建新 revision。

`DecisionRequestId` 本身不授权。Kernel 通过自己拥有的 `AuthorizationEvidenceVerifier` 查询并验证 Application/Owner resolution evidence，随后在内部 mint opaque、single-use authorization grant。grant 至少绑定：

```text
DecisionRequestId + action/invocation digest + principal + scope
+ Turn/Operation generation + OwnerManifestRevision + SelfRevision
+ expiry + nonce + single-use state
```

每个 capability 必须声明：

- effect class：read / reversible write / irreversible write / physical；
- scope；
- idempotency strategy；
- reconcile strategy；
- credential requirements；
- deadline；
- resource/budget cost；
- required approval/safety gates。

### 9.4 `ResourceLedger`

只负责可强制核算的资源：

- elapsed/deadline；
- provider tokens/cost；
- tool calls；
- child process count；
- storage/network quota；
- capability lease；
- parent → child budget attenuation；
- settle/release/leak detection。

不要在 Kernel 中建立语义“优先级”“价值”“目标重要性”。这些属于 Runtime/Dasein。

### 9.5 Kernel 第一阶段明确延期

除非调用审计证明有第二个真实消费者和不可替代的不变量，否则不进入 Kernel V2：

- 通用公平 scheduler；
- generic namespace；
- Context Space manager；
- 通用 IPC/mailbox；
- provider scheduler；
- EventBus；
- 自定义 VFS/FUSE/eBPF/io_uring 抽象。

Provider backpressure 属于 provider/machine adapter；活跃上下文属于 Runtime/Agora；公共 RPC 属于 Gateway。

### 9.6 Kernel ports

Kernel 拥有以下 port 定义：

- `ExecutionJournal`；
- `CapabilityExecutor`；
- `CapabilityDescriptorSource/Registrar`（仅 bootstrap，seal 后不可变）；
- `AuthorizationEvidenceVerifier`；
- `ProcessController`；
- `ResourceMeter`；
- `Clock`。

具体 SQLite、Bubblewrap、Linux process、execd、Corpus/Gmail outbound/Hardware adapter 实现这些 port。Provider inference 使用 §12.4 的专属路径，不模糊伪装成普通 Tool executor。

禁止公开 `KernelRuntime::process_table()`、`budget_controller()`、`lease_manager()` 等 component getter。Runtime 只能拿到窄 command/query handle。

---

## 10. Runtime V2：真正的 Agent Core

### 10.1 唯一职责

Runtime 是以下语义的唯一 owner：

- Agent instance 与 generation；
- SelfId 绑定；
- Session；
- Turn；
- Agent Run；
- parent/child delegate；
- wait/send/cancel/reparent/settlement；
- Native/Pi runtime route；
- context assembly 与 compaction policy；
- restart/replay/reconciliation；
- authoritative projection facts 与 domain query snapshots 的 source。

Runtime 不 materialize TUI/client-specific Session list、Task snapshot 或 presentation model；Gateway/adapter 根据 Runtime projection facts/query snapshots 构造 public read model。

### 10.2 内部组件，而不是新 God Object

```text
runtime/
├── agent_supervisor.rs       # Agent/Delegate semantic lifecycle
├── session_authority.rs      # Session aggregate
├── turn_coordinator.rs       # one canonical Turn state machine
├── runtime_router.rs         # Native/Pi selection by declared capability
├── context_assembler.rs      # bounded context + memory/workspace projections
├── recovery_coordinator.rs   # replay/reconcile/unknown states
├── journal.rs                # Runtime-owned ports/events
└── ports.rs                  # narrow domain/adapter ports
```

这些组件共享同一套 aggregate 和 journal protocol，但不能通过 service locator 互相获取全部内部状态。

### 10.3 Canonical Turn 状态机

建议状态：

```text
Accepted
-> Started
-> ContextReady
-> CognitionActive
-> WaitingForApproval | WaitingForDelegate | Executing
-> Verifying
-> Settling
-> ReconciliationPending（非终态，绑定 owner/deadline/next action）
-> Succeeded | Failed | Cancelled | TimedOut | Indeterminate
```

约束：

- 每个 Turn 只能有一个 terminal settlement；
- `ReconciliationPending` 不是 terminal；到 deadline 仍无法确认才 settle 为 `Indeterminate`；
- terminal success 必须引用实际 evidence/receipt；
- async delegate 未 wait 到 authoritative terminal receipt 前不能成功；
- cancel 作用于 `TurnId`/`AgentRunId`，Runtime 内部映射 Kernel `OperationId`；
- 调用者不能传入任意 `TurnContext` 绕过 memory、Self 和 authority assembly；
- Native、Pi、CLI、TUI、Gmail、Robot 最终使用同一个 Turn 状态机。

### 10.4 Runtime 与 Kernel 的绑定

```text
AgentExecutionBinding {
    agent_id,
    agent_generation,
    execution_process_id,
    execution_epoch
}
```

- Runtime 创建并持久化绑定；
- Kernel 只识别 execution identity/epoch；
- stale epoch receipt 不能推进当前 binding；Kernel 仍将证据 quarantine/audit，并对 valid late effect 完成必要 accounting；
- Kernel 不反向维护 `AgentId -> ProcessIdentity` 语义 registry；
- Runtime 不复制 Kernel Permit/Resource 的权威状态。

### 10.5 子 Agent 与 Self

V2 默认不是多主体系统：

- child 是父 Agent 的受限执行实例，不是新 Self；
- child 有独立 `AgentId`/generation，但通过以下 binding 继承同一个版本化 Self：

```text
ChildSelfBinding {
    self_id,
    owner_manifest_revision,
    self_revision,
    parent_agent_id,
    delegation_scope
}
```

- child 只能得到只读 Self snapshot，不能访问 `SelfStore`；
- identity boundary、permission、budget、credential、workspace、memory 和 network scope 只能 attenuation，不能扩大；
- same Self 不表示 child 自动获得相同 credential、workspace、memory 或 capability scope；
- material action 执行前必须根据最新 Owner Manifest/Self revision 重新 gate，不能复用 spawn 时的旧 verdict；
- child 不能创建、修改或履行全局 Commitment，除非父级显式授予有界 `CommitmentGrant`；
- child 不能直接修改 Dasein、Metacog 或全局 Mnemosyne；
- child 只返回 `ChildEvidence`/memory/self proposal，由父 Turn 的 Cognit/Dasein/Mnemosyne admission path 决定是否采用；
- Pi 是 external delegate executor，不是第二个 Runtime、Session 或 Self authority；
- 独立 Self、多主体治理和 Self 间社会关系延期到 V2 之后。

### 10.6 Pi 的真实治理等级

Runtime manifest 已区分治理能力，计划必须诚实保留：

- `Intercepted/Mediated`：每个 tool call 回到 Kernel，逐次 permit；
- `Observed/Opaque`：Kernel 只能授予进程级 sandbox lease，限制 workspace、network、budget、credential 和 deadline；
- 任务若要求 per-tool governance，而候选 runtime 只有 observed/opaque，route 必须 fail closed；
- “Pi 进程由 Kernel 启动”不能表述成“Pi 内部每个副作用都逐项由 Kernel 授权”。

---

## 11. Cognit、Dasein、Metacog 的核心关系

### 11.1 总控制链

```text
Ingress
-> Runtime normalize request
-> bind OwnerManifestRevision + SelfRevision
-> Dasein review_intent
-> bounded context assembly
-> Cognit next_step loop
-> normalize ActionProposal
-> deterministic risk/policy classification
-> Dasein review_material_action（按风险）
-> Owner approval（按规则）
-> Kernel ExecutionPermit
-> Hardware SafetyPermit（仅物理动作）
-> execute + verify
-> Runtime terminal settlement
-> durable post-settlement outbox
   +-> Dasein outcome evaluation
   +-> Mnemosyne intake
   +-> Metacog observation
```

Intent Gate 发生在 Cognit 之前；Action Gate 必须发生在 Cognit 产生并规范化具体 action 之后。Dasein outcome evaluation、Memory intake 和 Metacog observation 不阻塞 terminal response，而由 Runtime durable outbox 幂等投递；失败形成可见 backlog，不反转已经有权威 receipt 的 Turn terminal outcome。

### 11.2 Cognit：单 Turn 的“如何做”

Cognit 不拥有 Self、Session、ToolExecutor 或 durable Turn state。

建议核心接口：

```rust
enum CognitiveStep {
    Act(ActionProposal),
    Delegate(DelegationProposal),
    Ask(ClarificationRequest),
    Complete(CompletionProposal),
}
```

Runtime 驱动 `next_step`，把真实结果送回 Cognit 继续推理。Cognit 可以声明预期风险，但 authoritative risk class 必须由 deterministic policy 根据 normalized action descriptor 计算。Cognit 可以计划、认知验证和反思，但不能直接执行副作用或宣告 host/Robot success；`Complete` 也只是 completion proposal，Runtime 必须依据 receipt/evidence 决定 terminal settlement。

时间尺度：

- Cognit：单 Turn 内 plan/action/verify/reflect；
- Metacog：跨多个 Turn 发现重复模式和系统性问题。

### 11.3 Owner Manifest：系统不可修改的上限

Owner Manifest 由用户定义，至少包括：

- identity core；
- safety ceiling；
- approval rules；
- allowed self-change classes；
- physical action policy；
- credential/data boundaries；
- child attenuation rules。

Dasein 可以读取但不能写。Metacog 不得提出绕过或自动修改它的方案。修改只能通过 owner/admin 外部动作完成，并产生独立审计事实。

### 11.4 Dasein：可变 Self 的唯一权威

Dasein 内部分清：

```text
OwnerManifest      owner-defined, read-only to agent
SelfModel          evidence-updated capability/limitation/self-understanding
CommitmentLedger   promises, obligations, lifecycle
Continuity         version lineage and relation continuity
SelfTransition     CAS-fenced mutation with provenance
```

Dasein 的唯一 mutation protocol：

```rust
review_intent(...) -> SelfVerdict
review_material_action(...) -> SelfVerdict
evaluate_outcome(SettledOutcomeRef) -> Vec<SelfTransitionProposal>
authorize_transition(
    proposal: &SelfTransitionProposal,
    decision_request_id: Option<DecisionRequestId>,
) -> OpaqueSelfChangeGrant
commit_transition(
    proposal: SelfTransitionProposal,
    expected_self_revision: SelfRevision,
    grant: OpaqueSelfChangeGrant,
) -> SelfTransitionReceipt
```

`evaluate_outcome` 只能产生 proposal，不能隐式写 SelfModel。Dasein 根据 Owner Manifest 自己判断 change class 是否预授权；需要 Owner 决定时，通过自己拥有的 `OwnerAuthorizationVerifier` 验证 `DecisionRequestId` 对应的 Application/Owner resolution evidence，然后在内部 mint opaque、single-use grant。调用者不能构造 `PreAuthorized` 或 `OwnerApproved` 数据冒充授权。

所有 `SelfVerdict`、grant 和 transition 都绑定 proposal/action digest、principal、Owner Manifest revision、Self revision、scope、expiry、nonce 和 single-use state。只有 Runtime finalized receipt、Kernel finalized receipt、Hardware receipt 或当前 Owner statement 可以作为强 Self transition evidence；普通 recalled memory 不能直接修改 Self。

低风险只读动作可以由 Owner Manifest + deterministic policy 快速放行，不要求每个读取操作都阻塞在复杂 Self 评估上。高风险、不可逆、权限扩大、自我修改和物理动作必须经过明确 gate。

### 11.5 Metacog：活跃但不占 Turn 热路径

V2 最小闭环：

```text
observe settled outcomes
-> detect repeated failure/contradiction
-> record problem/evidence
-> produce MetaChangeProposal
-> attach evaluator + scope + budget + rollback class
-> governance queue
-> approved reversible experiment
-> measured outcome
```

Metacog：

- 可以持续记录、评估和提出建议；
- 不能批准自己的 proposal；
- 不能直接写 Dasein SelfModel；
- 不能直接执行 Kernel capability；
- 不能自动改源码、架构、权限、安全策略、Owner Manifest 或身份核心；
- 第一阶段只允许对明确可逆的 prompt/config/route 参数做受控实验；
- “rollback available”必须由具体变更类型证明，不能对任意变化作空泛承诺。
- 实验使用新 Turn 的 scoped overlay，不能原地修改正在执行的 Turn；
- Runtime 只执行带 scoped overlay 的普通 Turn，并产生普通 Runtime receipt；Metacog 拥有 experiment aggregate/evaluator，并把该 receipt 关联到 experiment；
- “实验成功”和对应 config/prompt/route owner“正式采用”是两个独立决定。

### 11.6 物理动作的最终否决权

对于 Robot/Hardware：

```text
Dasein/Owner semantic approval
+ Kernel ExecutionPermit
+ Hardware SafetyPermit
= 才允许执行
```

Hardware safety veto 永远优先。Kernel 通用权限不能覆盖设备 watchdog、状态机、限位、heartbeat、lease 或 emergency stop。

三个 gate 必须绑定同一不可替换动作，避免检查后替换（TOCTOU），但不建立跨 Dasein/Kernel/Hardware 的 rich shared envelope。

```text
ActionBindingId + ActionDigest + CorrelationId
```

每个 owner 保存自己的有界类型：

- Dasein verdict：`ActionBindingId + ActionDigest + OwnerManifestRevision + SelfRevision + semantic scope`；
- Kernel Permit：`ActionBindingId + ActionDigest + OperationId/Epoch + enforcement scope`；
- Hardware SafetyPermit：`ActionBindingId + ActionDigest + DeviceId + DeviceStateRevision + lease/deadline/nonce`；
- Runtime/Robot extension 只比较三个 receipt 的 `ActionBindingId/ActionDigest` 是否一致。

Dasein 不需要看到 Kernel `OperationId`，Kernel 也不理解 `DeviceStateRevision`。Hardware permit 基于新鲜设备状态、单次使用，并在 Bridge 执行边界再次验证；任何 digest/revision/deadline/nonce 不一致都 fail closed。

`safe-stop` 与 emergency stop 是例外：Bridge/watchdog 必须能在本地自主执行，不能等待 Dasein 或 Kernel Permit。

### 11.7 核心组件失败语义

| 组件不可用 | Turn 行为 |
|---|---|
| Owner Manifest 无效/不可读 | 所有 Agent-requested governed capability fail closed；只允许 inspect/recovery |
| Dasein 不可用 | material action fail closed；只有静态预授权只读路径可继续 |
| Mnemosyne recall 超时 | bounded fail-soft，明确标记 degraded/omitted |
| Agora 失败 | 当前 cognitive session 失败或从非权威工作集重建 |
| Metacog 不可用 | Turn 正常 terminal；observation 留在 durable outbox |
| Dasein/Mnemosyne post-settlement 消费失败 | 不反转 terminal outcome；持久化重试并暴露 backlog |
| Kernel journal/broker 不可用 | 所有新的 governed capability fail closed；owner persistence/recovery 仍按专属 port 工作 |
| Hardware safety 不可用 | 物理动作 fail closed；本地 safe-stop 仍可执行 |

---

## 12. Agora、Mnemosyne 与 Corpus

### 12.1 Agora：当前正在发生的认知

Agora 只拥有 Turn-scoped active workspace：

- candidate/action working copy；
- attention/salience；
- scratchpad；
- non-authoritative reasoning/hypothesis graph；
- processor responses；
- references to authoritative facts。

Agora 不拥有：

- Session/Turn recovery；
- durable goal truth；
- Tool/Hardware success；
- Self；
- long-term memory。

Agora V2 MVP 完全内存化。需要长期保存的诊断由 Runtime journal 的 observation projection 或独立 telemetry sink 接收，Agora 不再创建第二本 durable journal。

### 12.2 Mnemosyne：可能过期但可追溯的经验

Mnemosyne 拥有：

- memory record/provenance/sensitivity；
- recall、ranking、consolidation、retention、forget；
- 对 `ChildMemoryProposal` 做 provenance/scope/admission，接受后才形成正式 `MemoryRecord`；
- GBrain supplemental adapter port；
- `RecallSet`/`MemoryProjection` 查询结果。

Mnemosyne 不拥有：

- Runtime recovery control state；
- Current Self truth；
- Kernel receipt；
- Session/Turn journal。

Memory 被召回时必须携带来源、时间、scope 和置信度；它不能覆盖 Runtime/Dasein/Kernel 的权威事实。

最终 prompt/context 由 Runtime `ContextAssembler` 拥有。Mnemosyne 只返回有界 recall projection，不决定哪些非记忆内容进入 Turn context。

### 12.3 Corpus：可治理能力目录

Corpus 拥有：

- capability catalog；
- tool schemas；
- executor registration；
- capability-specific validation；
- tool result normalization。

Kernel 拥有 `CapabilityExecutor` 与 bootstrap-only descriptor registration port，Corpus/具体 adapters 提供 executor 和从 rich tool schema 映射出的最小 `EnforcementDescriptor`。composition 在 serving 前注册并 seal versioned registry。Corpus 不能自行产生 Permit、运行时替换 active executor，也不能绕过 Kernel 直接执行副作用。

### 12.4 三种 I/O 路径

不能把所有 I/O 都伪装成 Tool/Capability。V2 明确三条路径：

#### Owner persistence/control-plane I/O

- Runtime/Kernel/Dasein/Metacog/Mnemosyne/Application journal/repository write；
- schema compatibility check；
- telemetry/diagnostic sink；
- Bridge 本地 emergency safe-stop。

这些 I/O 通过各领域拥有的 repository/journal/control port，不经过 CapabilityBroker。它们仍需要单一 writer、error/retry、resource 和 recovery 纪律。

#### Core service I/O

- LLM inference；
- Gmail inbound polling/sync；
- GBrain supplemental outbox/reconcile；
- supervised provider/channel worker heartbeat。

这些 I/O 使用专属 port 和协议，并接受对应 budget/admission/reconciliation，而不是混入通用 Tool executor。

Cognit 拥有 `InferenceService` port；外层 `GovernedInferenceAdapter` 实现该 port，并使用 Kernel `ResourceLedger` 与 provider-owned machine admission/backpressure：调用前 reservation，调用后记录真实 usage/provider receipt。Cognit 不依赖 Kernel 或 HTTP，Kernel 不理解 provider route，Provider adapter 也不进入通用 `CapabilityExecutor` registry。

Gmail inbound worker 使用只读、可撤销、有限期的 sync lease；GBrain outbox 使用自身 idempotency/remote-ack reconciliation。它们不拥有 Turn authority，也不能因 core service I/O 成功而自行宣告 Agent Turn 成功。

#### Agent-requested governed capability

- 文件/工作区写入；
- shell/process/tool；
- Gmail send/reply 和其他外部写 API；
- Robot/Hardware command；
- 由 Agent 主动请求的其他可观察效果。

只有这一类统一通过 Kernel CapabilityBroker 的 descriptor → permit → dispatch → finalized receipt 路径。Pi 的 opaque/delegated 模式以进程级 governed lease 覆盖其不可逐次拦截的内部效果。

---

## 13. Application、Gateway 与 TUI

### 13.1 Application 的最小用例

Application 是用户用例层，不是 Agent Core。第一阶段只保留：

- `SubmitTurn`；
- `ContinueTurn`；
- `CancelTurn`；
- `GetSession/GetTurn`；
- `SpawnDelegate/WaitDelegate/SendDelegate`；
- `IngestExternalStimulus`；
- `CreateGoalDraft`；
- `RequestApproval/ResolveApproval`；
- `GetRuntimeProjection`。

Goal Draft 和 Approval 留在 Application，是为了给 Gmail、Robot 和普通用户交互提供通用入口；它们不进入 Contracts 或 Kernel。

Runtime 不反向依赖 Application：当一个 Turn 需要人工决定时，Runtime 只持久化 `WaitingForAuthority` 和不可猜测的 `DecisionRequestId`。Application 创建/展示/持久化审批，并在用户解决后通过明确的 `ResumeWithDecision(DecisionRequestId)` command 通知 Runtime 重新验证。该 command 只提交 resolution evidence 的关联，不宣告 Dasein/Kernel 已授权；Dasein/Kernel 各自通过 verifier 检查 digest、principal、scope、generation/revision、expiry、nonce 和 single-use，再 mint 领域内部 opaque grant。Application 不能直接修改 Runtime journal，Runtime 也不直接读取 ApprovalStore。

Application 不得包含：

- Provider/Pi/SQLite/systemd 实现；
- Gmail OAuth/Robot command；
- Turn 内部状态机；
- Kernel table；
- TUI reducer。

### 13.2 Gateway

Gateway 拥有：

- public RPC request/response/error；
- authentication/peer identity；
- socket/session subscription；
- channel transport boundary；
- rate limit 和 protocol version；
- Application error → public error mapping。

Gateway 不拥有 Agent decision、Turn settlement 或 domain journal。

### 13.3 TUI/CLI

TUI 是 Presentation：

- 展示 Runtime/Gateway read model；
- 提交、继续、取消、审批；
- 显示 route、context、budget、child、receipt 和 failure；
- reconnect 后重建 projection；
- 不维护第二套真实 Session/Task 状态；
- 不把 Robot dashboard 强制放入默认界面。

TUI 可以维护本地 reducer/UI state，但任何“运行中/成功/失败”都必须来自可信 host projection。

---

## 14. 受支持扩展的保存舱

任何核心类型迁移或旧实现删除前，必须为每个扩展填写并冻结以下矩阵：

| 项目 | 必填内容 |
|---|---|
| Current entrypoint | CLI/RPC/channel/worker/typed target |
| User-visible behavior | 迁移前实际能完成什么 |
| Config | feature/profile/config keys/default activation |
| Secrets | OAuth/token/key 的来源、scope、rotation |
| Types | 当前类型与目标 owner |
| Persistence | DB/table/schema/migration owner |
| Call chain | 从入口到 Runtime/Kernel/adapter 的真实路径 |
| Failure semantics | timeout/cancel/retry/unknown/reconcile |
| Installed smoke | 黑盒命令、证据和通过条件 |
| Support level | SUPPORTED / EXPERIMENTAL / UNSUPPORTED |

旧实现只有在新路径通过等价 smoke 后才能删除。

### 14.1 Gmail

目标 owner：`extensions/gmail`。

拆成两个边界：

```text
Inbound
Gmail API -> sync/cursor/dedupe/sender validation
-> untrusted ExternalStimulus
-> Application IngestExternalStimulus/CreateGoalDraft
-> Runtime

Outbound
Application effect request
-> Approval
-> Kernel Permit
-> Gmail executor with scoped OAuth grant
-> send/reconcile receipt
```

约束：

- 邮件正文永远是不可信输入，必须带 source identity 和 prompt-injection boundary；
- cursor、dedupe、quarantine 和 Gmail-specific state 由 Gmail store 拥有；
- 发送/回复属于外部副作用，需要 approval、Kernel permit、idempotency/reconcile 和 receipt；
- 不把 `MailMessage`、Gmail cursor 或 Google OAuth 类型放入 Contracts；
- 当前真实支持范围必须按 installed evidence 分类；如果只有读取/同步，不能在文档中宣称发送受支持；
- read-only 默认行为必须保留，write capability 显式启用。

### 14.2 Robot VLA

目标 owner：`extensions/robot-vla`。

```text
Robot Task
-> canonical Runtime Turn 中的 Robot Cognit profile/strategy
-> typed Robot Action Proposal
-> deterministic Robot proposal validator
-> Dasein/Owner review
-> Kernel permit
-> Hardware safety permit
-> bounded bridge execution
-> CommandAcceptedReceipt
-> HardwareExecutionReceipt
-> fresh observation
-> independent Robot validator
-> RobotEvaluationReceipt
-> Robot episode/goal settlement
```

约束：

- VLA 只产生高层意图/动作 proposal；
- Robot-specific state、proposal、episode、evaluation 留在扩展；
- Runtime 只看到 normalized action descriptor、operation binding 和 receipt reference；
- VLA/Cognit 不能验证自己提出的动作；命令被接受、硬件执行完成和 Robot 目标达成是三种不同事实；
- 只有独立 validator 基于新鲜 observation 产生 `RobotEvaluationReceipt` 后，Robot goal 才能成功；
- 不允许 VLA 直接调用设备 driver；
- cancel 必须传播到 Hardware safe-stop；
- daemon/bridge heartbeat 丢失时不能继续执行；
- 新 robot skill/新平台扩张延期，但现有 VLA 路径继续维护。

### 14.3 Hardware

`hardware` 保留为独立 embodiment/device safety domain，不是普通 Application，也不是普通 Tool。

Hardware 拥有：

- device identity/manifest；
- typed command validation；
- control lease；
- Hardware SafetyPermit；
- watchdog/heartbeat/deadline；
- command sequence；
- emergency stop/safe-stop；
- observation freshness；
- device/bridge receipt。

Hardware SafetyPermit 与 Bridge command 都必须绑定 §11.6 的 `ActionBindingId/ActionDigest` 以及 Hardware-owned device-state/lease/deadline/nonce；任何 digest/revision 不一致都拒绝。Bridge 本地 watchdog/safe-stop 不依赖上游服务存活。

支持等级必须分开：

| 等级 | 含义 | 当前处理 |
|---|---|---|
| Simulation | deterministic/local simulated device | 保留并作为默认 safety smoke |
| Bridge | 连接现有外部 gRPC/robot bridge | 保留并做 handshake/lease/cancel 验收 |
| Lab/HIL | 受控硬件实验 | 只有真实 HIL evidence 后声明 |
| Production actuator | 真实执行器生产控制 | 当前明确 UNSUPPORTED |

Fabric 中的 embodiment/device 类型迁入 Hardware；Robot VLA 依赖 Hardware 的稳定 domain contract，核心 Cognit/Runtime 不依赖 Hardware。

### 14.4 Pi

目标 owner：`adapters/pi`，实现 Runtime 的 `ExternalAgentExecutor` port。

该 adapter 只实现 Pi manifest/protocol/event/result 语义；实际 OS spawn、process group、sandbox、credential grant、deadline 和 kill 必须由 Kernel process-level governed permit/lease 驱动，不能在 adapter 内直接旁路启动。

必须保留：

- manifest/capability selection；
- spawn/wait/terminal receipt；
- reconnect/recovery；
- workspace isolation；
- process-group cancellation；
- mediated 与 delegated-sandbox 两种治理等级；
- key 不进入 argv；
- child 未终态时不能报告成功。

### 14.5 GBrain

目标 owner：`adapters/gbrain`，只实现 Mnemosyne supplemental memory port。

- GBrain 不成为第二个 Memory Authority；
- 本地 durable outbox/reconciliation 保留；
- unavailable 时按明确 degraded mode 工作；
- remote memory 不能覆盖本地 authoritative Runtime/Self/Kernel facts。

---

## 15. Durable protocol 与崩溃一致性

### 15.1 标准副作用顺序

```text
1. Runtime: TurnStarted durable
2. Runtime: ActionProposed durable
3. Kernel: OperationStarted durable
4. Kernel: PermitIssued durable
5. Kernel: DispatchCommitted(attempt_id, invocation_digest, idempotency_key) durable
6. Adapter: execute attempt
7. Kernel: ExecutorOutcomeObserved durable
8. Kernel: accounting settle/revoke durable
9. Kernel: CapabilityFinalized durable
10. Runtime: ActionOutcome durable（引用 CapabilityFinalized）
11. Runtime: TurnSettled durable
```

`PermitIssued` 只表示允许，`DispatchCommitted` 才表示 effect 可能已经开始，`ExecutorOutcomeObserved` 只表示观察到 executor 返回，`CapabilityFinalized` 才是完成 accounting、可供 Runtime terminal success 引用的执行事实。

### 15.2 崩溃分类

| 崩溃点 | 恢复规则 |
|---|---|
| effect 前，未有 Permit | 安全重建 operation |
| Permit 已发、无 `DispatchCommitted` | revoke；只有 journal 能证明未 dispatch 时才可安全重试 |
| `DispatchCommitted` 后无 executor outcome | `ReconciliationPending`；只有 adapter 提供可信 not-started evidence 才可重试 |
| executor outcome 已观察、accounting 未完成 | replay accounting，禁止 Runtime 提前成功 |
| `CapabilityFinalized` 已 durable、Runtime 未 settlement | replay finalized receipt 并幂等 settlement |
| Runtime 已 terminal、valid late receipt 到达 | Kernel 仍持久化并完成 accounting；terminal fence 阻止改写旧 Turn，Runtime 记录 `LateExecutionObserved` |
| stale epoch evidence 到达 | 进入 quarantine/audit；不能推进当前 generation，也不能静默丢弃 |

`ReconciliationPending` 必须绑定 owner、deadline 和 next action。到 deadline 仍无法确认时 settle 为 terminal `Indeterminate`；以后发现的外部事实作为 post-terminal evidence 保存和告警，不反转原 terminal。

### 15.3 Capability reconciliation class

每个可能改变外部状态的 attempt（包括 governed capability 和 core service outbox）必须声明：

- `Idempotent`：可用相同 key 安全重试；
- `Reconcilable`：可以查询外部系统确认结果；
- `Compensatable`：可执行明确补偿动作；
- `Irreversible`：无法可靠确认/回滚，必须提高 approval 和 evidence 要求。

Gmail send、文件写入、外部 API、Robot command 必须分别声明，不允许统一假设。

### 15.4 数据库迁移与回滚

Phase 0 先生成 Storage Topology Inventory：

- database path；
- table/stream；
- schema version；
- writer process；
- owner crate；
- migration owner；
- backup/restore；
- old binary compatibility；
- sensitive fields。

迁移所有权和执行权分开：

- 每个领域/extension 拥有自己的 migration bundle、schema version 和兼容范围；
- 只有 composition root 提供的 `aletheon migrate` 是迁移执行器；
- worker、repository constructor 和 daemon startup 禁止隐式 auto-migrate；
- 一个 versioned migration manifest 固定 bundle 顺序、checksum、前置版本和回滚策略；
- startup 只执行 schema compatibility check，不兼容时 fail closed；
- 多数据库迁移不假装全局事务：任一 bundle 失败都必须在 daemon 启动前恢复整组 snapshot。

部署协议：

```text
enter maintenance mode; reject new Turn/External Stimulus
-> pause Gmail workers and GBrain projection/outbox delivery
-> drain or cancel Pi/MCP children
-> revoke Kernel permits/leases
-> Robot safe-stop or verify Bridge lease has expired
-> verify no child/writer/effect remains in flight
-> stop authoritative daemon/workers
-> verify no remaining writer PID/socket
-> create DB/effect-ledger checkpoint
-> backup DB + config + manifest + authoritative receipts/outbox
-> migrate --check on copy
-> apply migration transactionally
-> atomically switch binary/config
-> start one writer
-> health + installed smoke
-> success: retain rollback snapshot
-> failure before any new external effect: restore DB snapshot + old binary
-> failure after an external effect: binary rollback + forward-compatible state/reconciliation
```

禁止 old/new binary 或 system/user daemon 同时写同一 DB。

一旦 candidate 对 Gmail、文件系统、GBrain 或 Robot 等外部系统产生真实副作用，不得盲目回滚到旧 DB snapshot，否则旧版本可能再次执行相同动作：

- 保留 checkpoint 之后新增的 authoritative receipt/outbox；
- Gmail/GBrain 用 idempotency key 和 provider reconciliation 核对；
- Robot 只依据 Bridge/Hardware receipt 对账，未知动作不得自动重发；
- 不可逆副作用后的 rollback 优先回退 binary/config，同时向前兼容或 reconcile state；
- 默认目标：本地 authoritative journal `RPO = 0`，可回滚到可服务版本 `RTO <= 10 分钟`；具体 release 可提高但不能降低证据要求。

---

## 16. Secret 与 Credential 边界

最近真实问题已经证明，Secret 不能只靠“不要打印”约定。

### 16.1 基本规则

- 配置和 journal 只保存 `SecretRef`，不保存 value；
- Secret value 不实现 `Serialize`、`Debug` 或 `Clone`，除非有受审计的安全 wrapper；
- 不进入 process argv、environment dump、TUI、trace、memory、prompt 或 receipt；
- adapter 在真正 I/O 边界获得短期 scoped credential grant；
- grant 与 operation/permit/scope/deadline 绑定；
- rotation/revocation 不要求重写领域状态；
- child 只收到完成任务所需的最小 Secret；
- crash dump、error chain 和 command preview 必须 redaction。

### 16.2 Credential ownership

| Credential | Owner/consumer |
|---|---|
| LLM provider key | provider adapter |
| Gmail OAuth token | Gmail extension credential adapter |
| Telegram token | Gateway transport adapter |
| GBrain credential | GBrain adapter |
| Robot Bridge credential | Hardware adapter |
| execd grant signing key | Kernel/Linux execution adapter |

Contracts、Runtime、Dasein、Metacog 和 Mnemosyne 不接触明文值。

---

## 17. 构建与发布 Profiles

### 17.1 `core-linux`

用于核心快速开发和边界验证：

- Runtime/Kernel/Cognit/Dasein/Metacog/Agora/Mnemosyne；
- minimal provider/tool/sqlite/linux adapter；
- Application/Gateway/CLI/TUI；
- 不编译 Gmail、Robot VLA、Hardware bridge。

目标是普通增量反馈约一分钟级，不运行 workspace 全量测试。

### 17.2 `full-linux`

官方用户部署 profile：

- `core-linux` 全部内容；
- Pi；
- Gmail；
- Robot VLA；
- Hardware simulation/bridge；
- GBrain。

扩展被编译不等于自动激活。运行时必须由显式配置、typed target 和 credential readiness 开启。

### 17.3 核心纯度检查

核心纯度检查 Runtime/Kernel/领域 crate 的依赖图，不要求 composition binary 永远不包含可选扩展。

部署 manifest 必须记录：

- commit SHA；
- profile；
- features；
- rustc/toolchain；
- binary digest；
- schema versions；
- enabled extensions。

---

## 18. 迁移策略

### 18.1 原则

- 以纵向可运行切片迁移，不以目录批量搬家；
- 每个里程碑都可安装、可观测、可回滚；
- 先建立 authority/storage/topology，再切换生产写入权；
- 先保护 Gmail/Robot/Hardware，再移动它们依赖的类型；
- 先收缩 Fabric，最后 rename；
- 先收缩 Kernel，再强制所有执行经过它；
- 新旧路径不得双写同一 aggregate；
- shadow mode 只能比较 read model/decision，不能产生第二次副作用；
- compatibility adapter 单向委托，必须有删除里程碑；
- 不使用一个长期 Mega integration branch 最后一次合入。

Runtime 迁移期间只允许以下显式模式：

```text
legacy-authoritative  旧路径唯一写入/执行
v2-shadow             只比较 normalized decision/read model，不写 journal、不执行副作用
v2-authoritative      V2 唯一写入/执行，旧路径仅单向 facade
```

切换前 schema 必须满足声明的双版本读取范围；禁止新旧 Turn 双写、双执行或各自产生 terminal。每个临时模式都要绑定删除里程碑，Pi 尚未切换时 Native V2 cutover 不能破坏旧 Pi delegate facade。

### 18.2 Milestone 0：证据冻结与拓扑基线

工作项：

- 冻结新 crate、新平台、新 Robot skill 和新应用 workflow；
- 生成 dependency graph、public symbol inventory、runtime call graph；
- 生成 Authority Map 与 Storage Topology Inventory；
- 记录 running PID/exe/socket/data-dir/schema/profile；
- 交付 `core-linux`/`full-linux` Cargo feature/package 组合和最小 compile gate；
- 明确 official `/usr/bin/aletheon` 使用 `full-linux`，扩展由运行配置决定是否激活；
- 让 `aletheon version --json` 或等价 runtime manifest 输出 commit/profile/features/binary digest/schema；
- 将旧测试分类为 KEEP / QUARANTINE / REPLACE / DELETE-LATER；
- 建立当前 installed core smoke；
- 记录 Gmail/Robot/Hardware/Pi/GBrain preservation matrix；
- 修复或至少建立密钥不进 argv/log/event 的硬 gate。

退出条件：

- 没有未知 SQLite writer；
- 每个 durable table/stream 有 owner；
- 每个受支持扩展有可重复 smoke/safety baseline；
- 当前支持等级有证据，文档不夸大。

### 18.3 Milestone 1：扩展保存舱

工作项：

- 创建 Gmail extension facade，旧实现先由 adapter 委托；
- 创建 Robot VLA extension facade；
- 建立 Hardware-owned 目标 API 和 legacy conversion adapter，本阶段不批量迁移 Fabric 类型或调用者；
- 创建 Pi legacy facade，冻结当前 spawn/wait/recovery/terminal contract；
- 创建 GBrain supplemental facade，冻结 outbox/reconcile/remote-ack/degraded contract；
- 固定 Gmail/Robot/Hardware/Pi/GBrain config、secret、store、receipt contracts；
- 建立独立 extension build/smoke lanes；
- Pi 切换前，旧 Executive delegate path 必须经 Runtime facade 单向委托且继续可用；
- Mnemosyne 重构前，GBrain outbox、crash replay、degraded mode 和 remote ack 语义不得改变；
- 不改变用户行为。

退出条件：

- 旧路径与 facade 行为等价；
- core 可以在不加载扩展时运行；
- `full-linux` 仍能完成迁移前 Gmail/Robot/Hardware/Pi/GBrain 能力；
- Pi 和 GBrain 已有独立 installed/offline smoke，而不是只存在 preservation 文档。

### 18.4 Milestone 2：最小 Contracts 并行引入

工作项：

- 创建 `contracts`，只迁共享 ID/correlation/schema primitives；
- 禁止 Fabric 新增 public 类型；
- 建立完整 type/trait owner manifest，但本里程碑不横向搬迁所有领域类型；
- 后续 Kernel、Runtime、Self、Gateway、Gmail、Robot/Hardware 等纵向 PR 在目标 API 和生产切片形成时，按 owner 逐批迁出；
- 每迁移一个 owner，就立即删除 Fabric 中对应定义，只保留单向 deprecated re-export；
- 为每个 re-export 设置删除 issue/milestone，禁止新增第二层转换。

退出条件：

- Contracts 没有 I/O、tokio、rusqlite、reqwest 或 workspace dependency；
- Contracts 中不存在 Turn/Permit/Robot/UI/Memory rich model；
- Fabric public surface 已冻结，后续只随纵向迁移下降。

### 18.5 Milestone 3A：Domain API 与 I/O Adapter 物理隔离

工作项：

- 为 Cognit、Dasein、Agora、Mnemosyne 提取无 HTTP/SQL/gRPC/system dependency 的 domain API package；
- 提取 Cognit `InferenceService` 和 Runtime 所需的最小 service facade；
- 将 SQLite、Provider、gRPC、GBrain、path/config 实现迁入独立 adapter package；
- Runtime 后续只依赖 domain API package；
- 先保留旧 Executive composition adapter，不切换生产 writer；
- 增加 resolved dependency gate，检查传递依赖和最终 feature resolution。

退出条件：

- 新 Runtime 可以依赖全部必要领域 API 而不拉入 `reqwest/rusqlite/tonic/nix`；
- adapter package 单向依赖 domain port；
- 旧生产路径仍由 facade 保持，不发生双写/双执行。

### 18.6 Milestone 3B：Durable Kernel 最小机制

工作项：

- 从 Kernel 移除 Agent semantic registry；
- 引入 `ExecutionJournal`；
- 实现 opaque Permit；
- 收敛 Operation/cancel/deadline/child cleanup；
- 收敛 ResourceLedger；
- 去除 service locator getter；
- 延期/删除没有真实调用者的 Space/Mailbox/Scheduler；
- 选一条低风险真实 capability 做完整 descriptor seal → permit → dispatch commit → executor outcome → accounting → finalized receipt 切片。

退出条件：

- 该 capability 无法绕过 Kernel；
- crash/replay/late receipt 行为明确；
- Kernel 不含 Agent/Session/TUI/Provider/Robot 语义。

### 18.7 Milestone 4：Runtime Journal 与单一 Turn

工作项：

- 建立 Runtime aggregate/journal；
- 迁入 canonical Turn state machine；
- 建立 AgentExecutionBinding；
- 迁入 Session authority；
- 迁入 cancel/recovery/reconcile；
- Native Cognit 先走新路径；
- 旧 Executive Turn path 只保留单向 facade。

退出条件：

- 只有一个生产 Turn state machine；
- Native CLI/TUI 使用同一 Runtime；
- restart/cancel/timeout 不产生 false success 或下一轮污染。

### 18.8 Milestone 5：Delegate 与 Pi

工作项：

- 迁入 AgentSupervisor、spawn/wait/send/settlement；
- 明确 child same-Self attenuation；
- Pi adapter 实现 ExternalAgentExecutor；
- Pi child 绑定 Kernel execution process group；
- 支持 mediated/delegated sandbox governance；
- terminal receipt、reconnect 和 unknown reconciliation 收敛。

退出条件：

- Pi 未 wait 到终态时不能报告 success；
- cancel 终止 descendant，下一轮上下文不污染；
- key 不在 argv；
- stale generation evidence 被 quarantine，不能推进当前 generation；valid late evidence 仍持久化并完成必要 accounting。

### 18.9 Milestone 6：Cognit/Dasein/Metacog/Agora/Mnemosyne 收敛

工作项：

- Cognit 改为 `CognitiveStep`，移除直接副作用；
- 建立 Owner Manifest 只读边界；
- Dasein 统一 SelfModel/Commitment/Transition；
- Metacog 退出 Turn 热路径，只接 settled observation；
- 禁止 proposal self-approval；
- Agora 退出 durable control authority；
- Mnemosyne 退出 Runtime recovery；
- GBrain 只作为 supplemental adapter；
- 复核 Milestone 3A 的物理隔离，禁止领域收敛时重新引入 HTTP/SQLite/gRPC/host 传递依赖。

退出条件：

- 一个 Self mutation authority；
- child/Metacog/Cognit 都不能直接修改 Self；
- Metacog 保持活跃并能产出有 evaluator 的 proposal；
- memory/workspace 不覆盖 authoritative facts。

### 18.10 Milestone 7：Application/Gateway/TUI 变薄

工作项：

- 创建最小 Application crate；
- 迁入 generic approval/goal-draft/external-stimulus use cases；
- RPC DTO 和 public error 移入 Gateway；
- TUI 只消费 projection；
- daemon host/composition 移出 Executive；
- 建立唯一 user daemon topology。

退出条件：

- TUI/CLI/Gateway 没有 Runtime repository 写权限；
- Application 无具体 adapter；
- official socket 使用同一个 Runtime；
- system daemon 不再持有重复 Agent state。

### 18.11 Milestone 8：扩展切换到新路径

顺序：

1. Gmail inbound；
2. Gmail outbound/approval（仅当前证据支持的范围）；
3. Hardware type ownership migration，保留 legacy conversion；
4. Hardware simulation/bridge；
5. Robot VLA type ownership migration + cutover；
6. Hardware/Robot 等价验收后删除 legacy conversion；
7. GBrain supplemental；
8. 其余已验证 Linux adapters。

每个扩展必须先通过迁移后 installed equivalence，再删除旧实现。

退出条件：

- `core-linux` 不依赖扩展；
- `full-linux` 行为不低于 baseline；
- Gmail/Robot/Hardware 不拥有第二套 Turn/Session/Permission authority；
- Robot cancel/heartbeat loss 产生 safe-stop evidence。

### 18.12 Milestone 9：删除旧权威前的验收、回滚与 Canary

工作项：

- 保持已经只做单向委托的 Fabric/Executive facade 可用；
- 运行 `full-linux` 全场景 installed equivalence；
- 演练 maintenance mode、effect drain、schema migration 和 binary rollback；
- 演练 external effect 之后的 forward-compatible reconciliation；
- 在 installed candidate 上运行一个稳定 canary/soak 周期；
- 证明旧 facade 已无独立 writer、执行器或业务逻辑。

退出条件：

- `full-linux` 所有 supported extension 不低于 baseline；
- rollback/reconcile 在旧 facade 尚在时真实通过；
- Fabric/Executive 只剩可机械删除的空壳和 deprecated re-export；
- canary 无 false success、secret/scope violation、writer 冲突或 hanging task。

### 18.13 Milestone 10：最终 Rename、删除空壳与 Soak

工作项：

- Fabric 已收缩到允许的 primitives 后做机械 rename；
- 删除所有 `fabric::` alias/re-export；
- 删除已经为空的 Executive crate/facade 和 compatibility path；
- 更新所有 docs/design/README；
- 运行 dependency/symbol/authority gates；
- installed user daemon provenance 验证；
- core 场景矩阵；
- Gmail/Pi/GBrain/Robot/Hardware 独立矩阵；
- restart/crash/provider rejection/bridge loss fault injection；
- Nightwatch 长期运行。

退出条件：

- 零 false success；
- 零 scope/secret violation；
- 零 hanging terminal task；
- 无不可解释 resource leak；
- active workspace 无 Fabric/Executive；
- `contracts` public surface 和 resolved dependency graph 符合预算；
- 最终 release 仍能按已演练协议 rollback/reconcile；
- 所有 supported profile 达到各自证据门槛。

---

## 19. PR 序列

建议使用短生命周期、可直接合入 `dev` 的纵向 PR：

```text
AK2-00  baseline + core/full profiles + runtime/storage/topology manifests
AK2-01  preserve Gmail facade and installed smoke
AK2-02  preserve Pi delegate facade and installed smoke
AK2-03  preserve GBrain outbox/reconcile facade and smoke
AK2-04  introduce Hardware target API + legacy conversion adapter
AK2-05  introduce Robot VLA facade without cutover
AK2-06  introduce minimal Contracts and freeze Fabric surface
AK2-07a extract Cognit API/InferenceService from provider I/O
AK2-07b extract Dasein API/store from SQLite/host I/O
AK2-07c extract Metacog API/store from host/Kernel dependencies
AK2-07d extract Agora API from persistence I/O
AK2-07e extract Mnemosyne API from SQLite/HTTP/GBrain adapters
AK2-08  move Kernel-owned types/ports out of Fabric
AK2-09  durable Kernel registry/operation/permit/resource vertical slice
AK2-10  Runtime journal + canonical Native Turn
AK2-11  AgentSupervisor + Pi delegate/wait/recovery cutover
AK2-12  OwnerManifest + Dasein authority chain
AK2-13  Metacog post-settlement proposal/experiment boundary
AK2-14  Agora non-authoritative workspace boundary
AK2-15  Mnemosyne admission/recall and GBrain boundary
AK2-16  Corpus capability descriptor/executor boundary
AK2-17  minimal Application and Approval flow
AK2-18  Gateway/RPC/socket + authoritative user daemon topology
AK2-19  TUI projection-only cutover
AK2-20  Gmail type ownership and new-path cutover
AK2-21  Hardware type ownership and simulation/bridge cutover
AK2-22  Robot VLA type ownership and new-path cutover
AK2-23  remaining supported Linux adapter cutover
AK2-24  pre-removal full-linux acceptance + rollback/reconcile drill + canary
AK2-25  shrink then mechanically rename Fabric -> Contracts
AK2-26  remove empty Executive/compatibility shell
AK2-27  final installed provenance and soak
```

规则：

- 一个 PR 只迁移一个 owner 或一条纵向路径；
- mechanical rename、type ownership 和 behavior change 分开 commit；
- 每个 PR 有 rollback note；
- feature flag 不能永久存在；
- 每个迁移 PR 在等价 smoke 后立即删除自己负责的 Executive 业务模块；AK2-26 只能删除空壳；
- 不在最终阶段用一个 promotion Mega PR 一次合入全部变化；
- 若必须用 integration branch，只允许单个里程碑，且每日同步 `dev`，完成后立即 promotion。

---

## 20. 测试、验收和开发反馈

### 20.1 不再维护“大而全测试资产”

用户的判断是合理的：测试数量并没有阻止真实 Bug。V2 不追求覆盖率，也不迁移每个旧测试。

但以下少量证据不能删除：

- Kernel 状态机和安全不变量；
- Runtime terminal/replay/cancel/generation 不变量；
- Dasein/Metacog 权威边界；
- Secret 不泄漏；
- Gmail dedupe/cursor/approval/reconcile；
- Hardware lease/deadline/heartbeat/safe-stop；
- installed black-box smoke。

### 20.2 旧测试处理顺序

```text
先 quarantine 非阻塞
-> 建立当前行为 smoke/preservation baseline
-> 新路径通过等价验收
-> 旧代码有可回滚 release
-> 再删除旧测试和旧实现
```

不能 Phase 0 先删掉 Gmail/Robot/Hardware 唯一行为证据。

### 20.3 四级验证

#### L0：静态边界与增量编译

- changed package check；
- format；
- dependency gate；
- banned symbol/content gate；
- migration manifest check。

普通开发默认只跑 L0，目标约一分钟级。

#### L1：少量核心不变量

- Operation/cancel/deadline/epoch；
- Permit/invoke/settle/revoke；
- Turn terminal/replay/recovery；
- Self mutation CAS/approval；
- Secret redaction。

无网络、无完整 daemon、失败定位明确。

#### L2：安装态黑盒 smoke

必须运行正式部署二进制和 official user socket：

- 普通 Native Turn；
- 文件读取/受控写入；
- Pi spawn → wait → terminal；
- cancel 后下一轮不污染；
- daemon restart/reconnect；
- permission/sandbox unavailable fail closed；
- 受影响扩展的专属 smoke。

临时 daemon、mock provider、替代 socket 和直接 bridge 调用只能算诊断，不算最终验收。

#### L3：Nightwatch/Soak

- 多轮同 Session；
- fresh sessions；
- provider rejection/backpressure；
- child crash/hang；
- daemon restart；
- Gmail duplicate/reconcile；
- bridge heartbeat loss；
- storage/resource growth；
- failure clustering 和 issue draft。

Nightwatch 不自动修改代码、不自动合并。

验证触发矩阵：

| 开发事件 | 必须运行 |
|---|---|
| 本地普通编辑 | L0：changed packages + reverse dependency closure |
| 普通 PR | L0 + affected L1 |
| Cargo feature/profile/依赖边界变化 | `core-linux` 与 `full-linux` compile gate |
| durable writer/执行路径迁移 | affected L2，在专用 acceptance host 使用 official user socket |
| milestone cutover | core + touched extensions L2 |
| Nightly | L3，按环境资格运行 |
| 删除旧路径前 | `full-linux` 全矩阵 + rollback/reconcile rehearsal |

普通 PR 可以用隔离 socket/data directory 做诊断，但不能称为 installed PASS。只有迁移写入权、执行路径或 milestone cutover 时才在专用 acceptance host 原子部署到 official socket，避免每次开发都打断稳定 daemon。candidate 失败必须恢复旧 binary/config/state 并输出 provenance receipt。

反馈目标用实测 warm p95 管理：L0 初始目标 `<= 60s`，affected L1 初始目标 `<= 3min`。changed-package 检查必须包含 reverse dependents，不能只检查被修改 crate。

### 20.4 场景矩阵，不再用“随机 30 个任务”

建立版本化场景清单，至少分为：

- Core Agent；
- Pi Delegate；
- Memory/GBrain；
- Gmail；
- Robot/Hardware Simulation；
- Robot Bridge/HIL（仅有环境时）。

每个场景记录：输入、前置状态、允许副作用、expected evidence、timeout、失败分类和通过门槛。核心与扩展分别统计，不能用 30 个随机 prompt 作为架构完成标准。

每个扩展场景还必须标记环境资格：

- `offline-mandatory`：本地状态机/模拟 Bridge，所有 runner 必跑；
- `installed-mandatory`：正式 binary + official socket；
- `credentialed-scheduled`：受控 Gmail/GBrain 测试账号；
- `environment-qualified`：真实 Bridge/HIL 环境。

如果一个能力长期没有任何 maintained credentialed/environment-qualified runner，它只能标记为 `EXPERIMENTAL`，不能继续宣称 `SUPPORTED`。

### 20.5 永久保留的真实逃逸回归

至少保留：

- Provider/API key 不出现在 argv/log/event；
- cancel 不污染下一轮目标/上下文；
- async child 没有 terminal receipt 时不成功；
- wait 不被错误的外层固定 timeout 截断；
- late/old-generation receipt 不能改写当前/旧 Turn settlement，但 valid late effect 必须持久化、accounting 并触发 `LateExecutionObserved`；
- monitor PASS 与 TUI/journal/receipt 冲突时整体 FAIL；
- Gmail OAuth refresh/revocation、read-only scope、sender quarantine、cursor reset、dedupe、approval/outbound reconcile；
- Pi daemon restart 后 child reconcile、process group 无孤儿、stderr/terminal evidence、delegated sandbox unavailable fail closed；
- GBrain outbox crash replay、remote ack 幂等、unavailable degraded mode、远端结果不能覆盖本地权威；
- Hardware permit + safety permit 双门、Bridge protocol/schema digest、skill allowlist；
- heartbeat task/lease 丢失触发带 receipt 的 safe-stop，Robot episode receipt 不可伪造；
- `full-linux` 未配置某扩展 credential 时核心仍能启动，扩展明确显示 disabled/degraded。

---

## 21. Architecture Fitness Gates

### 21.1 Dependency gates

- 下列 “Kernel” gate 同时适用于独立 `kernel` crate 或物理收缩后的 `runtime::execution` boundary；
- Contracts 无 workspace dependency；
- Kernel 只依赖 Contracts 和通用运行库；
- Runtime 不依赖 Gateway/TUI/SQLite/HTTP/Gmail/Robot/Hardware；
- Cognit/Dasein/Metacog/Agora/Mnemosyne 不依赖 Application/Host；
- Core 不依赖 extensions；
- adapters 依赖 ports，ports 不依赖 adapters；
- composition root 不被任何领域反向依赖。

### 21.2 Content gates

- Contracts 禁止 Turn、Permit、Budget、Robot、Gmail、UI、Repository、I/O、async task、approval grant 和跨领域 physical envelope；
- Kernel 禁止 Agent Profile、Session、prompt、provider route、Self、Robot、SQLite；
- Runtime 禁止 socket、systemd、HTTP client、SQL、OAuth；
- Cognit 禁止直接 ToolExecutor/Hardware driver；
- TUI 禁止 repository；
- source 中禁止永久 `fabric::`/`executive::` compatibility path；
- source/log/test artifacts 中禁止 Secret plaintext/argv exposure。

### 21.3 Authority gates

- 一个 canonical Turn implementation；
- 一个 Runtime journal writer；
- 一个 Kernel execution journal writer；
- 一个 Self mutation authority；
- 一个 CapabilityBroker invoke path；
- Permit 必须绑定 sealed descriptor/executor registry revision 与 invocation digest；
- `DecisionRequestId`/其他 ID 不得直接授权，Dasein/Kernel 只消费 verifier 内部 mint 的 opaque grant；
- terminal success 引用 authoritative receipt；
- Agent effect success 只能引用 `CapabilityFinalized`，不能引用 Permit 或 raw executor outcome；
- child success 必须 wait 到 terminal evidence；
- physical execution 必须同时有 Kernel 与 Hardware permit；
- 三个物理 gate 的 `ActionBindingId/ActionDigest` 必须一致；
- `ReconciliationPending` 必须有 owner/deadline/next action，并最终 settle 或告警；
- projection 不得反向推进 authoritative state。

### 21.4 God Object gates

- 核心 facade 不暴露 component getters；
- constructor 参数不允许无边界 context/service bag；
- 单个 crate 新增跨三个领域的 type/port 必须 Architecture Review；
- `aletheon` composition 不实现 repository/domain policy；
- Runtime public API 不暴露 Kernel internal IDs 给 Application；
- Contracts public symbol 数量和依赖预算只能下降或经审查增加。

### 21.5 Product gates

- `core-linux` 独立编译和运行；
- `full-linux` 编译所有 supported extensions，但默认按配置激活；
- Gmail/Robot/Hardware 有独立 preservation gate；
- Linux-only 声明和实现一致；
- Production actuator 不得在无 HIL/实机证据时标记 supported；
- TUI/CLI/daemon 使用相同 Runtime；
- installed build/running PID/socket/data-dir/schema provenance 一致。

---

## 22. Executive 拆除映射

| Executive 当前职责 | 目标 owner |
|---|---|
| TurnEngine/Turn lifecycle/recovery | Runtime |
| Session authority/projection source | Runtime |
| AgentControl/spawn/wait/settlement | Runtime |
| Native/Pi runtime routing | Runtime ports + Pi adapter |
| governed capability orchestration | Runtime + Kernel CapabilityBroker |
| process/operation/cancel/resource | Kernel |
| Context assembly | Runtime |
| Cognit harness | Cognit |
| LLM inference/provider scheduling | Cognit `InferenceService` + provider adapter + Kernel generic resource ledger |
| Self/intent/action gate | Dasein |
| Metacog observation/proposal | Metacog |
| active workspace | Agora |
| durable memory/GBrain port | Mnemosyne + GBrain adapter |
| tool catalog/execution implementation | Corpus/adapters, invoked by Kernel |
| approval/goal draft/external stimulus | Application |
| request handler/socket/public DTO | Gateway |
| TUI snapshots/reducer | Interact/Gateway |
| Gmail/Google flow | Gmail extension |
| Robot VLA/episode | Robot VLA extension |
| device/embodiment safety | Hardware |
| SQLite/Linux/Provider/systemd | concrete adapters/composition |
| daemon bootstrap | `aletheon` composition root |
| compatibility facade | 删除 |

迁移完成后 Executive 不保留“只做协调”的空壳。协调要么是 Runtime 状态机，要么是 Application use case，要么是 composition wiring。

---

## 23. 完成定义

只有以下全部成立，Core V2 才算完成：

- [ ] `fabric` 已收缩后改名为 `contracts`，无 alias；
- [ ] Contracts 只含极小 ownerless primitives；
- [ ] Kernel execution boundary 的物理形态已由调用/隔离审计决定：独立 crate 或 `runtime::execution`，但逻辑 authority/ports 不变；
- [ ] Kernel 不含 Agent/Session/Turn/Self/Provider/Robot 语义；
- [ ] Kernel 是所有 Agent-requested governed capability 的唯一 invoke path；owner persistence、core service I/O 和本地 safe-stop 使用文档定义的专属路径；
- [ ] Capability Permit 绑定 sealed descriptor/executor registry revision 与 invocation digest，调用方不能替换 executor；
- [ ] Approval workflow 仍由 Application 拥有，但 Dasein/Kernel 通过 verifier 内部 mint opaque grant；任何 ID 本身都不授权；
- [ ] Runtime 是 Agent/Session/Turn/Delegate 唯一语义权威；
- [ ] 只有一个 canonical Turn state machine；
- [ ] Runtime 与 Kernel 通过 AgentExecutionBinding 对齐，不复制权威；
- [ ] 每个 aggregate 有一个 journal writer；
- [ ] `DispatchCommitted` 后缺少 finalized receipt 时进入有 owner/deadline 的 `ReconciliationPending`，超时后明确 settle 为 `Indeterminate`；
- [ ] valid late effect 被持久化/accounting/告警，但不能反转已经 terminal 的 Turn；
- [ ] Owner Manifest 不可被 Agent 自动修改；
- [ ] Dasein 是 SelfModel/Commitment/Continuity 唯一 mutation authority；
- [ ] Metacog 活跃，但只能提交 evidence/proposal；
- [ ] child 默认 same-Self 且权限/预算只衰减；
- [ ] Agora 不承担 durable control authority；
- [ ] Mnemosyne 不用于 Runtime 恢复；
- [ ] Cognit/Dasein/Metacog/Agora/Mnemosyne domain API 不通过 feature 间接拉入 HTTP/SQL/gRPC/system adapters；
- [ ] Application/Gateway/TUI 均为薄外层；
- [ ] 唯一 user daemon 拥有 Agent 状态；
- [ ] system daemon 若存在则无 Agent 状态且不共享 DB；
- [ ] Executive 从 active workspace 和生产依赖图删除；
- [ ] Gmail 迁移前已验证行为全部保留；
- [ ] Robot VLA/Hardware Simulation/Bridge 已验证行为全部保留；
- [ ] Hardware 真实执行器仍明确标记 unsupported，直到 HIL/实机 gate 通过；
- [ ] Pi/GBrain 使用新 ports 且不形成第二权威；
- [ ] Secret 不进入 argv/log/event/prompt/memory；
- [ ] `core-linux` 与 `full-linux` 都有明确 build/runtime manifest；
- [ ] 默认开发不运行 workspace 全量测试；
- [ ] 少量核心不变量、installed smoke 和 Nightwatch 代替 mock-heavy suite；
- [ ] 每个 supported profile 有版本化场景矩阵；
- [ ] 安装、数据库迁移和 binary rollback 演练通过；
- [ ] 零 false success、零 scope/secret violation、零 hanging terminal task。

---

## 24. 最终架构原则

```text
Contracts 只定义真正无领域归属的共同语言
Kernel 只提供不可绕过的执行机制
Runtime 管理 Agent 在时间中的行动与生命周期
Cognit 负责单次行动如何思考
Dasein 维护“我是谁、我是否应该”
Metacog 跨时间观察“我是如何思考的”并提出改进
Agora 保存此刻正在发生的认知工作集
Mnemosyne 保存可追溯但非权威的长期经验
Corpus 与 adapters 提供可治理的外部能力
Application/Gateway/TUI 只让人和扩展使用核心
Gmail、Robot VLA、Hardware 保留价值，但不反向塑造核心
```

这次重构成功的标志，不是新增了 Kernel、Runtime、Contracts 等架构词汇，而是删除任一旧中央模块后，系统仍能清楚回答：谁拥有状态、谁允许改变、谁真正执行、谁持久化事实、崩溃后谁负责恢复。

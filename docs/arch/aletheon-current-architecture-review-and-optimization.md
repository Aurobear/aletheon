# Aletheon 当前架构合理性审计与优化方案

> 文档版本：1.0
>
> 更新日期：2026-07-19
>
> 审计快照：`dev` / `294e76c`，本地 HEAD 与 `origin/dev` 一致
>
> 关注范围：模块边界、依赖方向、生产调用链、状态所有权、持久化、能力治理、Agent Runtime、可运维性

---

## 0. 总结论

Aletheon 的总体方向——**单进程模块化宏内核、Kernel 管生命周期与权限、Executive 做编排、Cognit 做认知、Corpus 做能力、Agora/Mnemosyne/Dasein/Metacog 分别管理共享状态、记忆、主体与受治理演化**——是合理的，不建议推倒重写，也不建议改成大量微服务。

但当前代码还不能算已经收敛的生产架构。它更准确的状态是：

> 核心安全骨架已经形成，领域概念很多，迁移兼容层很多，但唯一运行主链、唯一状态所有者和最小公共契约仍未完全落实。

当前最主要的问题不是“少一个新模块”，而是：

1. 同一职责存在多个入口、门面或兼容实现。
2. 设计完成度明显高于实际用户闭环完成度。
3. `fabric`、`executive`、`corpus` 三个 crate 逐渐形成“大公共层、大编排层、大执行层”。
4. Session、Event、Memory、Agent run 和各种 SQLite 投影的权威关系仍然过于复杂。
5. 旧 Cognit/Body/Runtime 抽象没有进入真实生产主链，导致“架构看起来像强 Agent，实际运行仍主要是 Linear ReAct harness”。
6. 生产价值与实验性“意识/演化”能力没有通过部署 profile 和成功指标充分隔离。

建议把 Aletheon 定位为：

> **具有强治理能力的 Agent Runtime 平台，而不是以哲学模块数量证明智能的系统。**

近期优化重点应是“收敛和删减”：唯一 Turn Engine、唯一 Capability syscall、唯一 Session/Event 权威、通用 Agent Runtime、Host Platform，以及可度量的编码任务成功率。

---

## 1. 审计方法与限制

本次重新核对了：

- Workspace 与各 crate 的 `Cargo.toml` 依赖。
- `bin → executive → cognit/corpus/kernel/...` 的启动和执行路径。
- `TurnService`、`TurnCoordinator`、`TurnPipeline`、Cognitive Session、Agent Runtime。
- Kernel Process/Operation/Admission/Budget/Lease/Supervision。
- Corpus Tool/MCP/Sandbox/Driver/Skill/Hook。
- Session store、Event spine、投影、Agora、Mnemosyne、Dasein、Metacog。
- `scripts/architecture-check.sh`、架构 allowlist、completion ledger 和设计文档。

代码规模只用于识别风险集中点，不作为质量结论：

| Crate | `src` 约行数 | 判断 |
|---|---:|---|
| `executive` | 78,169 | 集成和迁移复杂度最高 |
| `corpus` | 42,940 | 工具、MCP、安全、Driver、Skill 过度集中 |
| `fabric` | 28,730 | “纯契约层”已经包含大量实现与重导出 |
| `mnemosyne` | 21,643 | 多代 Memory 实现并存 |
| `cognit` | 18,313 | 旧 CognitCore 与真实 harness 并存 |
| `dasein` | 17,430 | 主体状态、策略、安全恢复概念较多 |
| `interact` | 12,295 | TUI/协议兼容面较大 |
| `kernel` | 4,947 | 相对克制，边界最清晰 |

`fabric` 与 `executive` 中均有一千级别的 `pub` 声明/重导出匹配，说明公共 API 面已经过大。代码中约有大量 `serde_json::Value/json!`、`anyhow::Result`、`Arc<dyn ...>` 和 `Arc<Mutex<...>>` 使用；这些单独都不是错误，但组合起来会降低编译期约束，使状态所有权和失败语义难以判断。

本环境没有可用 `cargo` 命令，因此没有重新执行 Rust test suite。`architecture-check.sh` 的静态部分被检查过，但它内部直接调用 `cargo metadata`，在缺少 Cargo 时无法完整运行。仓库原有三个已修改的设计文档属于用户工作，本次没有修改项目仓库。

---

## 2. 当前真实架构

### 2.1 Crate 依赖形态

```text
fabric
  ↑
kernel
  ↑
agora / cognit / corpus / dasein / mnemosyne / metacog
  ↑
executive ─── gateway
  ↑
interact + bin
```

这个图没有 Cargo 循环依赖，是优点；但 DAG 不等于职责没有循环。当前存在多处“概念反向依赖”：

- Corpus 的 MCP 配置直接复用 `cognit::config`。
- Corpus MCP 认证直接复用 `mnemosyne::credential`。
- Executive 的 Cognitive Session factory 直接持有 `mnemosyne::RecallMemory` 和 `AdvancedCompressor`。
- Dasein 的恢复、感知代码直接执行 `systemctl`、`btrfs`、`journalctl` 等 Host 命令。
- Exec Server 作为隔离执行边界，却直接依赖整个 Corpus crate。

因此真正需要优化的是语义依赖，而不只是 Cargo 图是否有环。

### 2.2 当前生产 Turn 路径

```text
Daemon 请求
  -> DaemonTurnOrchestrator
  -> TurnPipeline::run
  -> Executive CognitiveSessionFactory
  -> cognit::LinearCognitiveSession::run_streaming_turn
  -> TurnServices / Governed Capability

CLI exec
  -> ExecSessionBuilder
  -> TurnService::submit
  -> TurnCoordinator
  -> Executive CognitiveSessionFactory
  -> cognit::LinearCognitiveSession::run_turn
  -> ExecTurnServices / Governed Capability

Native child Agent
  -> AgentControlService
  -> NativeCognitRuntime
  -> CognitiveSession::run_turn
```

这些路径共享了重要的 Cognit Session 和部分 Admission/Capability 语义，这是实际进展；但它们并不是同一个完整 Turn pipeline。Daemon 与 CLI 的 pre/post、Agora、Dasein、Session、event streaming 和 recovery 仍然由不同外层实现。

所以“唯一 Turn Execution Path 已完成”只能理解为“认知循环内核部分汇合”，不能理解为“所有 Turn 已经具有完全一致的生命周期语义”。

### 2.3 当前权威状态

主要状态面至少包括：

```text
Kernel tables       Process / Operation / Budget / Lease / Space
Session             CanonicalSessionStore / SessionService / SessionGateway
Event               EventSpine / EventSourcedSessionStore / projection stores
Agent control       run repository / settlement / mailbox / recovery
Working state       Agora workspace / conscious workspace / broadcast
Memory              Core / Recall / Episodic / Fact / Consolidation / Retention
Self                SelfField / Dasein reducer ledger
Evolution           Metacog mutation state / lineage
External            objective / approval / channel / Google sync stores
```

分域持久化本身合理，但当前 daemon bootstrap 打开大量独立 SQLite/JSON 状态文件，并需要跨存储投影。若没有统一 `StorageManifest`、迁移顺序、备份快照和 reconciliation 规则，生产恢复会比正常执行更危险。

---

## 3. 架构合理性评分

以下是基于当前代码的工程判断，不是性能 benchmark：

| 维度 | 评分 | 结论 |
|---|---:|---|
| 总体领域划分 | 7/10 | Kernel、Cognit、Corpus、Memory、Agora 的方向合理 |
| Kernel 边界 | 8/10 | 领域中立、Process/Operation/Admission 基本克制 |
| 依赖方向 | 5/10 | 无 Cargo 环，但 Corpus/Cognit/Memory/Host 语义泄漏 |
| 唯一执行主链 | 5/10 | Cognit session 汇合，外层 Turn 仍是双轨/多轨 |
| 状态所有权 | 4/10 | 有 canonical 目标，但兼容 store、投影和多数据库过多 |
| Capability 安全 | 7/10 | Admission、permit、sandbox fail-closed 是强项 |
| Agent 实际能力 | 4/10 | 主要仍依赖 Linear harness，工具/Runtime 闭环不足 |
| 可替换 Runtime | 4/10 | Pi/Native 已出现，但缺少稳定通用 Runtime contract/broker |
| Host 多平台 | 2/10 | Linux/Android 局部实现，Windows/macOS 缺失 |
| Hardware Control | 1/10 | 领域 API、Provider、lease 与安全数据面尚未形成 |
| 可运维性 | 4/10 | 状态文件和迁移多，支持矩阵与恢复演练不足 |

总体判断约为 **5/10 的生产架构成熟度**：不是架构失败，而是已经到了必须停止横向扩张、集中清理主链和状态权威的阶段。

---

## 4. 哪些设计应保留

### 4.1 保留宏内核，而不是微服务化

Aletheon 当前适合模块化单体：

- Agent Turn 内部有大量低延迟交互。
- Process/Operation/Capability 需要统一治理。
- 单用户或单节点初期部署不值得承担服务发现、分布式事务和跨服务 schema 演进。
- 外部 Runtime、硬件 Edge 和 Sandbox 可以是进程边界，但领域模块不需要全部成为服务。

### 4.2 保留 Kernel 的对象模型

以下对象值得成为系统最稳定的内核语义：

- `Process`
- `Operation`
- `CapabilityGrant/Permit`
- `Budget`
- `Lease`
- `ContextSpace`
- `Supervisor`
- `Clock`

Kernel 继续保持只依赖 contract，不知道 Cognit、Memory、Robot 或具体工具。

### 4.3 保留领域分工，但收紧权限

- Cognit：产生决策和工具调用意图。
- Corpus：执行已授权 capability。
- Mnemosyne：长期记忆唯一门面。
- Agora：共享工作状态与协作事务。
- Dasein：主体连续性、偏好、关切和解释。
- Metacog：受治理的变更候选、验证和迁移。

Dasein 与 Metacog 不必合并：前者回答“什么必须保持连续、什么值得关心”，后者回答“候选变更如何验证和应用”。但二者都不能绕过 Kernel policy。

### 4.4 保留事件溯源方向

Event spine + deterministic projection 是正确方向。问题不是事件溯源本身，而是 durable event、ephemeral stream、UI notification、mailbox 和 legacy RPC event 的边界还不够简洁。

---

## 5. 当前最严重的架构问题

### 5.1 “唯一主链”仍不唯一

当前至少存在：

- `TurnService`
- `TurnCoordinator`
- `TurnPipeline`
- `DaemonTurnOrchestrator`
- `CognitiveSession`
- `AgentControlService + AgentRuntime`

`TurnService` 自己被标记为 `TurnCoordinator` 的 compatibility facade，但仍是 CLI 生产入口；`TurnPipeline` 是 daemon 的完整编排器；Native child 又直接使用 Cognitive Session。

后果：

- 新能力需要决定接入哪一层，容易只对 daemon 或 CLI 生效。
- budget、deadline、cancel、checkpoint、event 和 verifier 语义容易漂移。
- “测试通过某条路径”不能证明其他入口一致。

优化：建立唯一 `TurnEngine`，让 daemon、CLI、child agent 只提供不同 adapter/policy：

```rust
pub trait TurnEngine {
    async fn execute(
        &self,
        request: TurnRequest,
        context: TurnExecutionContext,
        events: &dyn TurnEventSink,
    ) -> Result<TurnExecution, TurnError>;
}
```

`TurnCoordinator` 负责 operation/session/cancel 的控制外壳；`CognitiveSession` 负责认知循环；pre/post contributors 通过有序列表注入。不要再让 daemon 拥有一套 1,000+ 行 pipeline、CLI 拥有另一套 facade。

### 5.2 存在“有名字、无生产权威”的抽象

代码证据：

- `fabric::RuntimeOps` 没有生产实现。
- `AletheonBodyRuntime`/`BodyRuntime` 主要留在 Corpus 自身，生产 capability 路径已经转向 `CorpusService` 和 governed invocation。
- `AletheonExecutive::step()` 只递增 iteration 并返回占位结果。
- `AletheonExecutive` 仍持有 compatibility runtime registry，却不是 Agent run 的权威 owner。
- 旧 `CognitCore` 的 Planner/Reasoner/WorldModel 公开面很丰富，但真实 Turn 主要通过 `LinearCognitiveSession`。
- Executive 与 Cognit 各自定义了一个 `CognitiveSessionFactory` 概念。

这正是“代理能力看起来很强，实际很弱”的架构原因之一：大量设计对象没有处在用户任务的关键路径上。

优化决策：

- 要么把抽象接入唯一生产主链并设验收指标。
- 要么标为 experimental feature。
- 没有调用者、没有迁移计划的兼容抽象应删除，不能永久公开。

### 5.3 Fabric 已不是纯 contract crate

Fabric 同时包含：

- shared types 和 ID。
- subsystem/include traits。
- event spine contract。
- IPC transport、mailbox、bus 实现。
- policy、permission、sandbox 类型。
- kernel debug/registry。
- PNG、Nix、DashMap、Tokio 等实现依赖。
- 大量根级 `pub use`。

这使所有领域 crate 都获得一个过大的隐式依赖面，也让“放不下的类型先扔 Fabric”成为默认选择。

优化顺序：

1. 先在现有 crate 内建立 `contract` 与 `infra` 可见性边界。
2. 新代码只能从 `fabric::contract::*` 或明确模块导入，停止根级继续重导出。
3. 删除未使用的 legacy envelope/communication primitives。
4. 边界稳定后再考虑拆出 `aletheon-contracts` 和 `runtime-infra`，不要一开始新建十几个 crate。

### 5.4 Executive 仍是集成 God Crate

Executive 依赖几乎所有领域 crate是 composition root 的正常特征；问题在于它同时拥有：

- daemon transport 与 JSON-RPC compatibility。
- Turn pipeline 和 session coordination。
- Agent control、goal、automation、plugins、Google/channel。
- memory/evolution projection。
- approval、workspace trust、checkpoint。
- Runtime adapters 和 sandbox client。
- 大量 bootstrap 具体对象与数据库路径。

当前 `bootstrap/request.rs` 仍有约 1,300 行，架构门禁甚至把 2,000 行设为上限。这个上限防止继续恶化，却不是合理完成标准。

优化：

```text
executive/
  application/       TurnEngine、AgentControl、Goal、Approval use cases
  ports/             application-owned ports
  adapters/          daemon、CLI、channel、storage、runtime providers
  composition/       唯一具体对象构造位置
  compatibility/     有删除期限的旧协议
```

Bootstrap 应输出少数 service bundle，而不是把几十个 `Arc<Mutex<_>>` 逐层复制。

### 5.5 Corpus 边界泄漏

Corpus 目前包括工具、MCP、OAuth、Google、Skill、Hook、ACIX、桌面 Driver、Platform、Sandbox、Subagent worktree 等。更关键的是：

- MCP config 依赖 Cognit 配置类型。
- MCP credential grant 依赖 Mnemosyne 类型。
- 隔离 Exec Server 依赖整个 Corpus。

合理目标：

```text
Corpus Core
  CapabilityCatalog
  CapabilityExecutor
  Tool schemas
  Evidence/Receipt

Providers
  workspace-tools
  shell/git/test
  mcp
  browser
  desktop
  hardware
```

配置由 Executive 解析后投影成 provider-owned config；凭据属于 Security/Credential Port，不属于 Memory；Exec Server 只依赖 sandbox/platform contract 和少数 provider protocol。

### 5.6 状态所有权仍然过于复杂

当前 Session 有 canonical store、event-sourced wrapper、SessionService、SessionGateway 和 legacy service；Event 有 durable spine、projection store、bus 和 UI stream；Memory 有多种 live object；Agent control 又有独立 run/settlement/mailbox store。

需要为每类事实写出唯一权威表：

| 事实 | 唯一写入权威 | 其他表示 |
|---|---|---|
| Turn/Item history | EventSpine 或 Canonical Session Store，二选一并固定 | read projection |
| Active operation | Kernel OperationTable | durable recovery record |
| Agent run lifecycle | AgentRunRepository + Kernel binding | UI projection |
| Shared task state | Agora transaction | prompt summary |
| Long-term memory | MemoryService | index/cache/GBrain replica |
| Self continuity | Dasein ledger | SelfField read projection |
| Mutation lifecycle | MetacogService | audit/event projection |

不能允许两个“权威”互相修复。恢复应从一个 durable authority 重建 projection。

### 5.7 硬安全与 Dasein 主体策略混合

Dasein/SelfField 的设计强调所有意图经过主体解释，这对于偏好、关切、叙事和需要人类确认的判断有价值；但不可变安全边界不能依赖可演化的主体状态。

必须明确单调权限原则：

```text
Kernel/Organization Policy 可以拒绝
Dasein 可以进一步拒绝、降级、要求 sandbox/approval
Dasein 永远不能扩大 Capability Grant 或覆盖 Kernel deny
LLM/Cognit 永远不能直接决定最终权限
```

建议把 SelfField 分为：

- `ConstitutionalPolicyPort`：确定性、版本化、不可由普通学习自动修改。
- `SelfInterpretationPort`：关切、叙事、偏好、建议和解释。

前者进入 Admission；后者进入 planning/context，并只能收紧决策。

### 5.8 多 Runtime 还不是一等架构

Native Cognit、Pi、goal worker/reviewer、provider worker 已经存在，但注册、事件、工具治理、workspace delta、checkpoint、receipt 和 verifier 没有全部统一为一个 Runtime contract。

结果是每接一个 Codex/Hermes/Grok，都可能复制 adapter 特判。

目标应是：

```text
RuntimeDescriptor
RuntimeSession
WorkOrder
RuntimeEvent stream
RuntimeControl: steer/cancel/resume/checkpoint
WorkspaceDelta
RuntimeReceipt
ToolGovernanceLevel
```

Executive Broker 只按 capability、health、policy、cost 和 task type 选 Runtime。

### 5.9 架构文档与门禁自身发生漂移

现有 `CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md` 中的依赖矩阵与当前 Cargo 文件不完全一致。例如当前 Corpus 直接依赖 Cognit 和 Mnemosyne，但旧矩阵没有体现；文档称 EventBus 双轨完成，而代码仍保留 CommunicationBus、CanonicalEventBus、EventSpine 和兼容事件投影等多种语义。

`architecture-check.sh` 是有价值的 shrink-only gate，但存在问题：

- 依赖分析内部直接调用 `cargo metadata`，未走仓库规定的 Cargo wrapper。
- 在 Cargo 不存在时诊断不够清晰。
- `corpus -> cognit/mnemosyne` 被列为 reviewed exception，不会推动边界收敛。
- 文件行数上限只能限制增长，不能证明职责合理。
- completion ledger 容易把“有 contract test”误写成“产品路径完全收敛”。

架构状态应由可机器验证的 `architecture-status.toml` 管理：owner、authority、production path、compatibility deadline、required acceptance tests。

---

## 6. 各模块优化决策

| 模块 | 决策 | 优化方向 |
|---|---|---|
| Kernel | 保留并强化 | 继续领域中立；成为生命周期/权限唯一权威 |
| Fabric | 保留名称，收缩 | contract/infra 分层，减少根级重导出，迁出实现 |
| Executive | 保留，重组 | 只做 application + composition；削减具体状态持有 |
| Cognit | 保留，收敛 | `CognitiveSession` 成为生产核心；旧 CognitCore 要么接入要么实验化 |
| Corpus | 保留 Core，拆 provider | 去掉 Cognit/Memory 反向依赖；Host/Hardware 分离 |
| Mnemosyne | 保留统一门面 | 生产只依赖 `MemoryService`；旧 backend 内部化/feature gate |
| Agora | 保留 | 只做事务化共享工作状态，不做通用事件总线或长期记忆 |
| Dasein | 保留但降耦 | 主体解释与硬安全分离；不直接执行 Host 命令 |
| Metacog | 保留为可选能力 | 只生成/验证/迁移候选，默认不在核心 Agent 成功链路阻塞 |
| Gateway | 保留 | 渠道中立，Executive 实现 use-case ports |
| Interact | 保留 | 只消费版本化 client protocol，不构造领域对象 |
| Exec Server | 保留进程边界 | 改依赖 Platform/Sandbox contract，不依赖整个 Corpus |
| Host Platform | 新建独立边界 | Linux/Windows/macOS 进程、文件、PTY、服务和 sandbox |
| Hardware Control | 新建独立边界 | Device/Provider/Broker/Lease/Safety，不进入 Host trait |

---

## 7. 推荐目标架构

```text
Clients: TUI / CLI / Telegram / API / Automation
                         │
                         ▼
                Executive Application
       ┌─────────────────┼──────────────────┐
       │                 │                  │
   TurnEngine       AgentControl        Goal/Workflow
       │                 │                  │
       └────────────── Operation ───────────┘
                         │
                         ▼
                    KernelRuntime
 Process / Operation / Admission / Budget / Lease / Space / Supervision
                         │
        ┌────────────────┼──────────────────┐
        ▼                ▼                  ▼
   Cognit Session   Runtime Broker   Capability Broker
                         │                  │
           Pi / Codex / Hermes        Corpus Core
                                      │
                         ┌────────────┼─────────────┐
                         ▼            ▼             ▼
                   Workspace      Host Platform   Hardware Broker

Domain services used through ports:
  Mnemosyne / Agora / Dasein / Metacog

Infrastructure:
  Durable EventSpine / Read Projections / Artifact Store / Credential Vault
```

### 7.1 两类事件通道，而不是一个总线统治全部

```text
DurableEventLog
  用于 lifecycle、session item、approval、settlement、audit
  可 replay、带 schema/version/idempotency

EphemeralStream
  用于 token delta、progress、presence、debug telemetry
  有 backpressure，可丢弃或重连，不作为事实权威
```

Mailbox 是 Agent 之间的有界消息语义，Request/Response 是外部协议语义，不应因为底层 transport 相同就混成一个万能 EventBus。

### 7.2 一个 Capability syscall

所有工具、Runtime 外部动作和 Hardware command 最终经过：

```text
resolve capability
  -> bind actor/workspace/operation
  -> policy + Dasein tightening
  -> budget/lease/sandbox admission
  -> provider execution
  -> evidence + usage + receipt
  -> settlement/revocation
```

禁止 Executive handler、Dasein、Plugin 或 Runtime adapter 自己直接执行 shell/process/filesystem 写操作。

### 7.3 Required 与 Optional 依赖分离

当前 `TurnServices` 有大量默认空实现，容易出现“编译成功但生产静默降级”。应改为：

```rust
pub struct RequiredTurnPorts {
    pub inference: Arc<dyn InferencePort>,
    pub capabilities: Arc<dyn CapabilityInvoker>,
    pub sessions: Arc<dyn SessionPort>,
    pub operations: Arc<dyn OperationPort>,
}

pub struct OptionalTurnFeatures {
    pub memory: Option<Arc<dyn MemoryRecallPort>>,
    pub agora: Option<Arc<dyn AgoraReadPort>>,
    pub self_interpretation: Option<Arc<dyn SelfInterpretationPort>>,
}
```

启动时输出 Feature Manifest；缺失 required port 直接失败，缺失 optional feature 明确报告而不是返回默认空对象。

---

## 8. 为什么当前 Agent 能力弱

### 8.1 真正执行的不是文档中的全部“认知系统”

真实 Turn 核心主要是 Linear Cognitive Session/ReAct loop。Planner、WorldModel、Critic、CognitCore、RuntimeOps、BodyRuntime 等很多概念没有共同构成一个生产决策图。

因此新增意识、反思或架构对象不会自动提升搜索、定位、修改、测试和恢复能力。

### 8.2 工具闭环比认知层更重要

编码 Agent 的最低生产闭环是：

```text
理解仓库指令
-> 全局/符号搜索
-> 精确读取
-> 计划修改
-> 带冲突检测编辑
-> 运行窄测试
-> 读取诊断
-> 修复
-> 独立 verifier
```

任何一步缺失都会让模型表现为“不会写代码”。Aletheon 当前在这些能力上仍有 cwd、搜索分页、结构化结果、LSP、artifact、verification 和 Runtime 接入等缺口。

### 8.3 控制平面复杂度没有转化为任务成功率

Agent run、Dasein、Agora、Memory、Metacog、事件投影都可能参与一个 turn，但缺少统一的任务级指标来证明每层带来增益。结果是启动、状态和恢复路径变复杂，编码成功率却不一定提高。

建议用 ablation profile 验证：

```text
Core profile: Kernel + TurnEngine + Corpus + Session + Pi
+Memory
+Agora
+Dasein interpretation
+Metacog reflection/evolution
```

只有能在任务成功率、恢复率或安全率上证明增益的层，才进入默认生产 profile。

---

## 9. 分阶段优化路线

### A0：冻结横向扩张，建立真实架构账本

时间：约 1 周。

- 建立 `architecture-status.toml`。
- 列出所有 production entry、compatibility facade、experimental module 和删除期限。
- 修正当前依赖矩阵与文档完成状态。
- 修复 architecture check 的 Cargo 探测和 fail-fast 诊断。
- 禁止新增 Fabric 根级重导出和 Executive 具体 domain state。

验收：每个公开 Runtime/Turn/Session 接口都有 owner、生产调用者和状态；没有调用者的接口必须标 experimental/deprecated。

### A1：唯一 Turn Engine

时间：约 2–3 周。

- 提取 `TurnEngine::execute`。
- 把 daemon 与 CLI 差异改为 `TurnPolicy + contributors + event sink`。
- `TurnCoordinator` 成为唯一 operation/session/cancel 外壳。
- 合并 Executive 与 Cognit 的 factory 概念。
- Native Agent 也进入同一 Turn Engine，或明确作为外部 Runtime 并返回标准 receipt。
- 为 daemon/CLI/child 建同输入的 semantic parity tests。

验收：工具授权、deadline、cancel、compaction、receipt 和 terminal settlement 只有一套实现。

### A2：Capability 与 Corpus 收敛

时间：约 2–3 周。

- Corpus Core 只保留 catalog/executor/schema/receipt。
- Workspace、MCP、browser、desktop 变成 provider 模块。
- MCP 配置类型从 Cognit 移出。
- Credential grant 从 Mnemosyne 移到 Credential Port。
- Dasein/Executive 的直接 `tokio::process::Command` 逐步走 Host Capability。
- Exec Server 改为最小 protocol + platform/sandbox contract。

验收：所有副作用都有 operation/permit/receipt；Corpus 不再依赖 Cognit 和 Mnemosyne。

### A3：状态权威与存储运营

时间：约 2–4 周。

- 确定 Session/Event 唯一 authority。
- 为 legacy session service 设置移除版本。
- 建立 `StorageManifest`、schema version、migration coordinator。
- 定义多数据库备份顺序、恢复点和 reconciliation。
- 将 critical control-plane store 合并到一个事务边界，或写明跨库补偿协议。
- 启动恢复测试覆盖 turn、agent run、lease、approval 和 checkpoint。

验收：kill -9 后可从 durable authority 重建；没有两个 store 互相宣称权威。

### A4：通用 Agent Runtime 与编码能力

时间：与 A2/A3 部分并行。

- 落地 Runtime API/Broker。
- Pi 成为默认 Coding Runtime。
- Native Cognit、Codex、Hermes、Grok 统一成 adapter。
- Workspace delta、runtime event、checkpoint、steering、cancel、receipt 统一。
- Workspace Tools V2、LSP、Git/test/diagnostics 和 verifier 接入。

验收：同一 WorkOrder 可切换 Runtime；Aletheon 不需要在 Executive 加 Runtime 特判。

### A5：部署 Profiles 与高级域隔离

时间：约 1–2 周。

```text
core            Kernel + TurnEngine + Corpus + Session
coding          core + Workspace Tools + Pi + Git/Test/LSP
personal        core + Mnemosyne + Dasein + Gateway
conscious       personal + Agora recurrent workspace
evolution       conscious + Metacog，默认人工批准
hardware-edge   core + Hardware Broker/provider
```

- Dasein constitutional minimum与可选 narrative/care 分开。
- Metacog 默认不允许直接生产 mutation。
- 每个 profile 输出 capability、storage 和 recovery manifest。

验收：Core/Coding profile 不因高级认知模块故障而不可用；启用高级模块有可测增益。

### A6：Host 与 Hardware 两条独立轨道

- Host：按 [Host Platform 多操作系统计划](aletheon-host-platform-plan.md) 实施 Linux→Windows→macOS。
- Hardware：按 [Hardware Control Platform 计划](aletheon-hardware-control-platform-plan.md) 实施 simulator→ROS 2 仿真→只读总线→HIL→真实执行器。

两者只通过 Kernel Capability 和受治理 Host 原语连接。

---

## 10. 建议 PR 顺序

### PR-A01：生产路径清单与架构门禁修复

- 添加架构状态清单。
- 更新实际 Cargo 依赖图。
- Gate 检测无 Cargo、过期 allowlist、新兼容入口。
- 不改变运行语义。

### PR-A02：删除/隔离无生产调用者接口

- 标记或 feature-gate `RuntimeOps`、旧 BodyRuntime、旧 CognitCore 路径。
- 给 `AletheonExecutive::step` 和 compatibility registry 设置替代与删除计划。
- 收缩 root re-export。

### PR-A03：TurnEngine contract

- 先引入 contract 和 parity harness。
- 适配现有 daemon/CLI，不立即删除旧 facade。

### PR-A04：Daemon 迁移到 TurnEngine

- 将 TurnPipeline 拆为 contributors。
- 保持流式协议兼容。

### PR-A05：CLI/Native Agent 迁移

- 删除 TurnService compatibility facade。
- 统一 deadline/cancel/settlement。

### PR-A06：Corpus 依赖倒置

- MCP-owned config。
- CredentialPort。
- 删除 `corpus -> cognit/mnemosyne`。

### PR-A07：Session/Event 权威收敛

- 固化 durable log 与 read projection。
- 删除 legacy 写路径。

### PR-A08：Runtime Broker + Pi production adapter

- 完成编码任务纵向切片与 verifier。

每个 PR 都要减少兼容面或 forbidden dependency；不要再以“大合并 PR”同时改协议、数据库和主链。

---

## 11. 生产指标与架构验收

### 11.1 任务指标

- Coding benchmark 首次成功率。
- 平均修复迭代数。
- 工具调用有效率和重复调用率。
- verifier 通过率与 false-success 率。
- cancel 后残留进程数。
- crash/resume 成功率。

### 11.2 架构指标

- 生产 Turn engine 实现数：目标 1。
- Capability execution syscall 数：目标 1。
- 每类 durable fact 的 authority 数：目标 1。
- `corpus -> cognit/mnemosyne`：目标 0。
- Fabric 新增根级 re-export：目标 0。
- 兼容 facade 有删除版本比例：目标 100%。
- Host 直接 process/fs 调用 allowlist：只减不增并最终归零。
- Required port 使用默认 no-op 的数量：目标 0。

### 11.3 运行可靠性

- 启动中任一 DB migration 失败必须阻止进入 ready。
- 每次 operation 有 start、terminal、usage、evidence 和 settlement。
- Event projection 可从零重建并校验 hash/count。
- 升级可回滚，旧 schema 有明确支持窗口。
- Production profile 的 feature manifest 可查询。

---

## 12. 不建议的优化方式

- 不推倒重写整个 Rust workspace。
- 不把每个领域都拆成独立微服务。
- 不先新建十几个 `*-api` crate 再寻找调用者。
- 不继续增加新的 Bus、Harness、Runtime facade 或 Memory store。
- 不因名称优雅而保留没有生产调用者的抽象。
- 不把所有 JSON 都替换成复杂泛型；只类型化安全、生命周期和跨边界协议。
- 不让 Dasein 或 Metacog 成为权限授予者。
- 不在 Agent 编码闭环不稳定时优先扩展更多“意识层”。
- 不以单元测试数量代替唯一主链、真机恢复和端到端任务成功率。

---

## 13. 最终目标结构

Aletheon 成熟后的核心不应由模块名称来描述，而应由五个不可绕过的系统语义来描述：

```text
1. 所有工作成为 Operation
2. 所有执行主体成为受监督 Process/Runtime Session
3. 所有副作用成为受 Permit 约束的 Capability Invocation
4. 所有结果形成 Evidence + Receipt + Settlement
5. 所有 durable fact 只有一个 Authority，可重放产生 Projection
```

如果这五条成立，Dasein、Agora、Mnemosyne、Metacog、Pi、Codex、Hardware Provider 都可以插拔或演化；如果这五条不成立，再多的模块只会增加多真相源。

最终建议：

> 保留 Aletheon 的宏内核和领域思想，但把未来两到三个版本的主线改成“架构收敛 + Coding Agent 纵向闭环”。先删除假能力和兼容双轨，再增加 Runtime、Host 和 Hardware 的真实能力。这样既不浪费现有架构投入，也能让用户实际感受到 Agent 变强。

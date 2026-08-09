# Executive 完全拆解：专项计划索引与重复权威总账

> 状态：Draft
>
> 基线：`dev@bd1ceac2965832f15cf45b811fd34e052e7e9a67`
>
> 上位计划：[`2026-08-08-agent-kernel-v2-complete-rearchitecture.md`](./2026-08-08-agent-kernel-v2-complete-rearchitecture.md)

## 1. 为什么需要这一组专项计划

上位计划已经确定最终边界：`fabric` 收缩后改名为 `contracts`，Runtime 成为 Agent/Session/Turn/Delegate 的唯一语义权威，Kernel 只做不可绕过的执行治理，Application/Gateway/TUI 位于外层，Executive 最终完全删除。V2 的 Kernel 物理形态已经固定为独立小型 crate；本轮只能收缩它的 public surface，不能在迁移中顺手并入 Runtime，未来变化必须等 V2 稳定后另开 ADR。

本组计划解决更具体的问题：最新 `dev` 中，同一语义在多个目录、多个 crate 和多个数据库里都有实现。若只按照 `executive/src/application`、`core`、`host`、`adapters` 横向搬目录，`RequestHandler::new`、`TurnPipeline`、`AgentControlService` 和 `SessionGateway` 会换一个名字继续成为 God Object。

这次采用纵向迁移：一次只收敛一个可运行能力的 owner、command、journal、adapter 和 presentation，并在新路径接管生产流量后立即删除对应旧路径。

本计划系列不关闭 Dasein/Self 或 Metacog，不删除 Gmail、Pi、GBrain、Robot VLA、Hardware，也不把 `contracts` 命名为 ABI。

## 2. 最新 `dev` 的可核对事实

### 2.1 Executive 的规模不是根因，但暴露了边界失效

- `crates/executive/src` 当前约 12.3 万行；
- `application/turn_pipeline.rs` 约 2527 行；
- `application/agent_control/mod.rs` 约 1672 行；
- `host/daemon/bootstrap/request.rs` 约 1591 行；
- `host/daemon/server.rs` 约 1575 行；
- `application/agent_control/settlement.rs` 约 1557 行；
- `application/session_service.rs` 约 1156 行；
- `core/session_gateway/gateway.rs` 约 681 行。

`RequestHandler::new` 有 16 个入参，并在一个构造函数中创建默认 Session、Self、Memory、Goal、Approval、Gmail、Agora、Kernel、Robot、Native/Pi runtime、AgentControl、Session/Turn service 和后台 worker。它不是 handler 构造函数，而是隐藏在 transport 下的 composition root。

### 2.2 依赖图证明 Executive 是总装箱

当前直接依赖大致为：

```text
executive -> fabric, gateway, kernel, agora, cognit, corpus,
             mnemosyne, runtime, dasein, metacog, hardware
interact  -> fabric, executive
aletheon  -> fabric, executive, interact
runtime   -> serde, serde_json, async-trait
```

`runtime` 目前主要只有 manifest/selector；真正的 Agent Runtime 语义仍在 Executive。与此同时 Executive 内部的依赖方向也反转：

- Application 直接依赖 `core::SessionGateway`、composition config、具体 adapters 和 host formatter；
- Core 直接依赖 Application、Host、Composition、具体 Session adapter；
- Adapter 反向依赖旧 core registry、composition config 和中央 compatibility migrations；
- Interact/TUI 直接依赖 Executive 的 daemon lifecycle 和 host launcher；
- ACP 直接 bootstrap `RuntimeCore`，再直接打开 Executive 的 canonical SessionStore。

### 2.3 Application 并不纯

`executive/src/application` 中至少 16 个源文件直接引用 SQLite，至少 24 个源文件直接访问文件系统；Approval apply 路径还直接启动 `git` 子进程。典型位置包括：

- `application/approval/repository.rs`；
- `application/goal/*`；
- `application/event_projection.rs`；
- `application/admin_service.rs`；
- `application/agent_control/settlement.rs`；
- `application/workspace_checkpoint.rs`；
- `application/workspace_trust.rs`；
- `application/approval/apply_coordinator.rs`。

因此“新建 application crate，把现有目录整体搬过去”明确不合格。

## 3. 重复实现分类规则

同名不自动等于重复，重复也不一定同名。本系列使用四类标签：

| 标签 | 含义 | 处理方式 |
|---|---|---|
| `AUTHORITY-DUP` | 两处都能分配 ID、推进状态或持久化同一事实 | 先选唯一 owner，切 writer，再删除旧 writer |
| `IMPLEMENTATION-DUP` | 同一个 port 有多份实际实现，但没有明确选择规则 | 保留一个 production adapter；其他转 fixture、兼容或删除 |
| `PROJECTION/CACHE` | 名称像 aggregate，但实际是展示、缓存或工作集 | 改名并禁止写 authority journal |
| `VOCABULARY-COLLISION` | 名字相同但语义不同 | 不盲删；重命名并收窄类型边界 |
| `PRESERVE-MOVE` | 是活能力，但位置和依赖方向错误 | 先冻结行为证据，后迁移，等价后删除旧位置 |

每个迁移 PR 必须在描述中声明它处理哪一类；不接受“cleanup”“refactor”作为删除双写路径的唯一解释。

## 4. 重复权威与实现总账

### 4.1 Agent / AgentRun / Runtime registry

| 当前实现 | 当前性质 | 目标 | 决定 |
|---|---|---|---|
| `application/agent/mod.rs::AgentRuntime` | 自带内存 Agent/Task map 的旧状态机；未发现生产构造者，不是“仅重导出” | 无 | public API/caller 核验后删除，不能迁成第二 Runtime |
| `application/orchestration::{Agent,AgentRegistry,...}` | 旧执行栈；WorkflowStore 仍有生产价值 | Runtime orchestration + 独立 workflow store | 删除旧执行者，保留并迁移 workflow 定义 |
| `application/agent_control::AgentControlService` | 当前最可信的 AgentRun reducer/repository owner，但本身过宽 | `runtime::AgentSupervisor` + 小组件 | 迁移不变量，不整块搬入 Runtime |
| `core::RuntimeRegistry` + `core::SubAgentRuntime` | Goal/Pi 仍在使用的兼容执行路径 | `runtime::DelegateBackendRegistry` | Pi/Goal 切流后删除 |
| `application::agent_control::AgentRuntimeRegistry` | 新 launcher/manifest registry | Runtime sealed catalog | 迁入并成为唯一 registry |
| `NativeCognitRuntime` | 混合 Cognit、profile、tool、metering、AgentRun | Cognit `CognitiveRun` adapter | 拆分后保留认知能力 |
| `PiRuntime` + `PiRpcRuntime` | 两条 live Pi 路径，共享 `pi-coder` 语义 | Pi `DelegateBackend` | resident 先切换；legacy 等等价后删 |
| `ProviderWorkerRuntime` | Provider 被伪装成通用 runtime | `InferenceAdapter` 实现 `InferencePort` | 从 delegate registry 移出 |

Agent ID 当前至少由 root daemon lifecycle、AgentControl child spawn 和 admission 分别生成。目标只有 Runtime 可以创建 Agent/AgentRun identity；Kernel 只分配并治理 Process/Operation generation。

### 4.2 Session / Thread / Context

| 当前实现 | 当前性质 | 目标 | 决定 |
|---|---|---|---|
| `core/session.rs::{Session,TuiSessionManager}` | 无生产构造者，却能 mint Session ID 和计算 permission/tool policy | 无 | 先删 |
| `host/daemon/session_manager.rs::SessionManager` | messages、turn count、rewrite、compaction 的可变工作集 | Runtime `ContextWorkingSet` | 改名、降权、可重建 |
| `application/session_service.rs::SessionService` | canonical append/query、active Turn 和独立 protocol journal 混合 | Runtime `SessionAuthority` + Gateway projection | 拆开迁移 |
| `compatibility/LegacySessionService` | canonical 与旧 mutable 双向同步，仍是 writer | 单向 Runtime facade | 禁止 mint/旧库写入后再删 |
| `adapters/session/store.rs::SessionStore` | 旧整份 JSON upsert | migration reader only | 切换后删除 |
| `CanonicalSessionStore` | normalized read projection | Runtime journal adapter/read model | 保留语义，迁出 Executive |
| `EventSourcedSessionStore` | writer 包装层，同时驱动 spine/projection | Runtime journal adapter | 收敛事务边界 |
| `SessionGateway` | JSON-RPC、debug、LLM、Memory、Self、host manager 混合 | Gateway query/projection handler | 拆掉业务依赖后删除旧实现 |

`ThreadId` 与 `SessionId` 当前大量通过字符串互转，并有独立 `ThreadAuthorityStore`。目标是 Session aggregate 内的 authority binding；transport thread/connection identity 不能再伪装为 Session identity。

### 4.3 Turn

当前一条请求会穿过 `TurnCoordinator`、`TurnPipelineLifecycle`、`TurnPipeline`、`DaemonTurnEngine`、`DaemonTurnOrchestrator`；CLI 还走 `TurnService/ExecSessionBuilder`，ACP 又有手工 composition。

`TurnId` 当前可由：

- `TurnCoordinator`；
- Native Cognit child path；
- lifecycle synthetic path；
- Session legacy projection；
- TUI optimistic overlay；
- CLI `ExecSessionBuilder`

分别生成。

目标是一个 `runtime::TurnAggregate` 和一个 durable reducer。TUI 使用 `UiOverlayId`；非 Turn 生命周期使用独立 occurrence/correlation ID；Cognit、Pi、Gmail、Robot 都只能接收 Runtime 已分配的 binding。

### 4.4 Capability / Approval

当前 Kernel invoker、Executive `governed_capability`、host `ProductionCapabilityService/TurnToolExecutor` 共同组成执行链；executor 在调用时注入，没有形成启动后 sealed 的 descriptor→executor binding。

Approval 又分为：

- Corpus socket approval；
- Executive 内存 `PendingApprovals`；
- durable `ApprovalRepository/ApprovalService`；
- 独立长期 grant cache。

目标分工：发起等待的 authority owner 先持久化 challenge 并分配 `DecisionRequestId`；Application 拥有 durable Approval 展示与 resolution aggregate；Kernel 验证并消费单次 execution grant；各 adapter 只注册 executor。grant 必须绑定 action digest、principal、scope、Agent/Process generation、policy revision、expiry、nonce 和 single-use state。

### 4.5 Event / Journal / Projection

以下不是一律删除，而是必须明确角色：

| 当前组件 | 正确角色 |
|---|---|
| `fabric::EventSpine` + SQLite implementation | durable history/journal mechanism |
| `CanonicalEventBus` / Kernel bus | 进程内通知 transport，不是恢复权威 |
| Agora broadcast/store | Turn-scoped active workspace 事实，不是 Session journal |
| `CanonicalSessionStore` | 可重建的 normalized read model |
| protocol events DB | Gateway streaming/cursor projection |
| `public-session` JSON projection | 先证明有消费者；无生产 reader 则删除 |

任何 projection failure 都不能反向决定 Turn terminal truth；任何 bus delivery 都不能代替 durable append。

### 4.6 Goal / Cognit plan state

Executive `ObjectiveStore/GoalCoordinator` 是持久业务 Goal；Cognit `GoalTracker` 是单 Turn 认知计划，却也叫 Goal、维护状态并直接写 YAML。

目标：Goal Draft/人类确认属于 Application；被采用后的实际执行进入普通 Runtime Turn/Agent stream，不建立第二套 Agent Runtime。Cognit 组件改名为 `PlanProgress` 或 `TaskObligationTracker`，只输出 proposal/evidence，不持久化业务 Goal。

### 4.7 Domain-rich duplicates

| 重复 | 唯一 owner |
|---|---|
| Dasein `SelfField`、`DaseinModule`、Metacog mutation lifecycle、Fabric rich Self traits | Dasein commit；Metacog 只 proposal/experiment/evaluation |
| Agora workspace 与 Executive conscious workspace/coordinator | Agora 持有 workspace；Runtime 驱动 cycle |
| Mnemosyne memory service/projection 与 Executive memory gateway/policy/maintenance/projection | Mnemosyne |
| Cognit provider factory 与 Executive inference bridge/harness/native runtime | Cognit port + provider adapter；Runtime 只编排 |
| Fabric embodiment permit 与 Hardware lease/safety + Executive authority/service | Hardware safety；Kernel generic grant；Robot task semantics |
| Metacog rubric 与 Executive coding rubric/adapter | Metacog；无生产调用的副本先删 |

`contracts` 最终只保留 ownerless ID、digest、correlation 和必要 schema primitive，不能继续承载这些 rich domain models。

### 4.8 Store / loader / type collisions

这些项目需要逐项处理，不能用全局重命名掩盖：

- 三个 `ArtifactStore`：Corpus tool output、Executive artifact adapter、Platform store；
- 两个 `AgentLoader`：baseline census 已证明 agent definition loader 只有自身单测、无生产 caller；inactive 实现由 foundation PR 删除，仍在生产 bootstrap 使用的 Markdown role profile loader 后续精确命名；
- 两个 Runtime registry；
- 两个 `AgentId`：UUID aggregate identity 与 legacy IPC `u64`；
- 两个 `SessionRecord`：Fabric canonical record 与 Executive legacy JSON row；
- 两个 `CheckpointStore`：workspace checkpoint port 与旧 core file store；
- Executive 与 Mnemosyne 各有 `MemoryProjection`、`CompactionLineage`；
- Executive 与 Metacog 各有 rubric 类型；
- Corpus/Fabric 各有 policy/approval 类型；
- Agora/Corpus 都有 TaskGraph/TaskNode/TaskStatus，但一个是协同 workspace，一个是工具局部任务；
- Cognit/Hardware 都有 `ProviderRegistry`，语义不同，必须改为 `InferenceProviderRegistry` 与 `DeviceProviderRegistry`。

处理原则是先确认语义，再决定 consolidate、rename 或 delete。`AgentLoader` 已完成 caller/input/output census，因此可以有证据地删除 inactive 实现；TaskGraph、ProviderRegistry 仍不得仅因同名直接删。

## 5. 专项计划与依赖图

本索引对应七份实施计划：

1. [`Runtime 权威收敛`](./2026-08-08-runtime-authority-consolidation.md)
2. [`Kernel Enforcement 收敛`](./2026-08-08-kernel-enforcement-consolidation.md)
3. [`领域权威与 Adapter 抽离`](./2026-08-08-domain-authority-and-adapter-extraction.md)
4. [`Application 与持久化抽离`](./2026-08-08-application-persistence-extraction.md)
5. [`Composition、Gateway 与 Presentation 抽离`](./2026-08-08-composition-gateway-presentation-extraction.md)
6. [`保留扩展迁移`](./2026-08-08-preserved-extensions-cutover.md)
7. [`Executive 退役与 Fitness Gates`](./2026-08-08-executive-retirement-and-fitness-gates.md)

逐文件处置账本：[`Executive source disposition ledger`](./2026-08-08-executive-source-disposition-ledger.md)。它是本索引的机器可核对附件，不是第七套架构。

依赖关系：

```text
Authority ledger + freeze gates
        |
        +--> Runtime IDs/catalog --> Agent --> Session --> Turn
        |                                  \        /
        |                                   Journal
        |
        +--> Kernel contracts/enforcement --> Linux/effect adapters
        |                 \                       /
        +--> Domain ports/owners --> Application ports/adapters
        |               \                     /
        |                +--> preserved extensions
        |
        +--> Gateway protocol/client split --> TUI/CLI/ACP cutover
                                                |
                                                v
                                  owner logic out + compatibility drain
                                                |
                                                v
                                    fabric thinning -> contracts rename
                                                |
                                                v
                                      empty Executive shell deletion
```

扩展 preservation facade 必须早建；最终扩展 cutover 依赖 Runtime、Domain、Application 的新 ports。Executive 删除和 `fabric -> contracts` 机械 rename 都是结果，不是第一步。

### 5.1 唯一编号 crosswalk 与实施 DAG

这是仓库内唯一规范的 AK2 crosswalk。`AK2-*` 只是上位 roadmap umbrella：它描述 release 能力，不拥有 writer，也不自动产生第二个实现 PR。实际分支、review owner 和 deletion evidence 必须使用下表的 `RA/K/D/APX/CGP/E/XRET` 编号；同一格有多个专项编号时，表示一个 umbrella 由多个窄 owner PR 交付，不表示把它们合成 Mega PR。只有 `E2-K6a`、`E4-K6b`、`E5-K6c`、`E6-K6d` 是明确的单个联合 PR。

| Roadmap umbrella | 实际 PR owner / 专项切片 | 直接前置 | writer / executor cutover | 旧路径删除门 |
|---|---|---|---|---|
| `AK2-00a/00b` | `RA-00 + K0 + D0 + APX-00 + CGP-00 + E0 + XRET-00` 各自的 census/freeze PR | baseline commit | 无；只固化 manifest、caller、ID、writer、topology 与非增长 gate | 每份 inventory 与源码双向一致后，后续 owner PR 才可开始 |
| `AK2-01` Gmail 保存舱 | `E0` Gmail manifest | `AK2-00a/00b` | 无；legacy Gmail 仍唯一 writer | `E2-K6a` 等价后由 `XRET-03/E7` 删除旧实现/alias |
| `AK2-02` Pi 保存舱 | `E0` Pi manifest | `AK2-00a/00b` | 无；legacy Pi 仍唯一 child owner | `E6-K6d -> RA-06` 后由 `XRET-03` 删除旧 runtime/seam |
| `AK2-03` GBrain 保存舱 | `E0` GBrain manifest | `AK2-00a/00b` | 无；legacy outbox/reconcile 仍唯一 writer | `E3` 等价后由 `XRET-03/E7` 删除旧实现/alias |
| `AK2-04` Hardware target API | `E0 + E1` Hardware seam | `AK2-00a/00b` | 无真实设备切流；legacy/simulation 保持现任 owner | `E4-K6b` 等价后由 `XRET-03/E7` 删除旧 conversion/fallback |
| `AK2-05` Robot VLA facade | `E0 + E1` Robot seam | `AK2-00a/00b` | 无；只建立 frozen facade | `E5-K6c` 等价后由 `XRET-03/E7` 删除旧 facade |
| `AK2-06` minimal Contracts / Fabric freeze | `D0 + D1` | `AK2-00a/00b` | 无 writer；只抽 ownerless primitives 并冻结新增 rich type | `D6` 清退 rich surface；rename 只能在 `AK2-25` |
| `AK2-07a` Cognit API | `D2` | `D0 + D1` | Cognit `InferencePort` 接管，provider adapter 保留 machine admission | `D6/XRET-02` 删除 Executive inference bridge |
| `AK2-07b` Dasein API/store | `D3` 的 Dasein slice | `D0 + D1` | Dasein 成为 Self commit/store 唯一 writer | `D6/XRET-02` 删除 Executive/Fabric Self writer |
| `AK2-07c` Metacog API/store | `D3` 的 Metacog slice | Dasein authority seam 可用 | Metacog 只写 proposal/experiment/evaluation store | `D6/XRET-02` 删除 host/Kernel mutation shortcut |
| `AK2-07d` Agora API | `D4` 的 Agora slice | `D0 + D1` | Agora workspace writer 接管 | `D6/XRET-02` 删除 Executive conscious workspace duplicate |
| `AK2-07e` Mnemosyne API | `D4` 的 Mnemosyne slice | `D0 + D1` | Mnemosyne local memory writer 接管；GBrain 仍 supplemental | `D6/XRET-02` 删除 Executive memory writer |
| `AK2-08` Kernel types/ports | `D1 + K1 + K2` | `D0 + K0`；`K1` 另等 `RA-01/APX-00`，`K2` 等 `K1 + Application verifier design` | Kernel 分配 Operation/descriptor identity；Contracts 不写状态 | `K7 + D6` 删除 Executive/Fabric Kernel surface |
| `AK2-09` durable Kernel slice | 严格串行 `K3 -> K4 -> K5`（`K1/K2` 已由 `AK2-08` 交付） | `K3` 等 `K2 + Contracts/Application verifier`；`K4` 等 `RA-04 + owner migration composition`；`K5` 等 `CGP-01`；扩展 manifest `E0/E1` 只阻塞 `K6` | `K4/K5` 依次切唯一 admit/invoke/receipt 与 Linux process writer | `K7/XRET-02` 删除 parallel settlement、spawn 与 registry |
| `AK2-10a` Agent/Session authority | 严格串行 `RA-01 -> RA-02 -> RA-03` | `RA-01 <- RA-00`；`RA-02 <- RA-01`；`RA-03 <- RA-02` | `RA-03` 切 Runtime Agent/Session ID 与 journal writer | `RA-03` 可删 dead Session authority；其余在 `RA-06/XRET-02` |
| `AK2-10b` Native Turn cutover | `RA-04`（Runtime journal `RA-02` 已由 `AK2-10a` 严格先行交付） | `RA-03` | `RA-04` 切唯一 Turn ID/reducer/terminal writer | `RA-06/XRET-02` 删除旧 pipeline/daemon authority |
| `AK2-11a` generic supervisor/registry | `RA-05` | `RA-04` | Runtime `AgentSupervisor` 与 sealed `DelegateBackendRegistry` 接管；不迁 Pi | legacy Pi 保持单向 seam，禁止在此删除 |
| `AK2-11b` Pi production cutover | 唯一联合 PR `E6-K6d` | `RA-05 + K6 + E0/E1` | Pi adapter 切 spawn/wait/cancel/recovery 与 Kernel process lease | installed smoke、active-child drain/reconcile 后进入 `RA-06/XRET-03` |
| `AK2-11c` legacy cleanup | `RA-06` | `E6-K6d` + matching `APX-05`/host caller-zero evidence | 无新 writer；验证 Runtime 单入口 | 只删除 Runtime-owned `SubAgentRuntime`、旧 registry/AgentControl facade；Executive tree/crate 留给 `XRET-04/05` |
| `AK2-12` Dasein authority | `D3` 的 Dasein owner PR | `D1` | OwnerManifest + Dasein verdict/commit writer | `D6/XRET-02` |
| `AK2-13` Metacog boundary | `D3` 的 Metacog owner PR | `AK2-12` authority chain | post-settlement proposal/experiment writer | `D6/XRET-02` |
| `AK2-14` Agora boundary | `D4` 的 Agora owner PR | `D1` | Turn-scoped workspace writer | `D6/XRET-02` |
| `AK2-15` Memory/GBrain boundary | `D4 + E3` | `D1 + E0` | Mnemosyne 写 canonical memory；GBrain 只写 supplemental outbox/ack | `XRET-03 -> E7 extension rows -> D6 root closeout` |
| `AK2-16` Corpus executor boundary | `D5`（复用已交付 `K2` seam） | `D1 + K2` stable | Corpus 拥有 catalog；Kernel sealed registry 拥有 invoke dispatch | `K7 + D6/XRET-02` |
| `AK2-17` minimal Application | `APX-01..APX-05` | `APX-01 <- RA-05`；`APX-02/03/04 <- APX-01`；`APX-05` 按文件族只等 authoritative seams：`RA-03/04/05`、`D2..D5`、`K4/K6`、对应 `E2..E6`，不等待其最终 deletion stage | `APX-02` 切 Approval aggregate/store；Goal Draft 由 `APX-03` 切 | `APX-05` 只删 Application-owned dead facade并产 caller-zero evidence；Runtime/domain/Kernel 旧实现由 `RA-06/D6/K7`，extension concrete/bootstrap/schema 由 matching `E2..E6+XRET-03`，extension-specific Fabric row/re-export 才由 `E7` 唯一删除 |
| `AK2-18a` typed Gateway | `CGP-00..CGP-03` | `APX-01 + RA-01` public commands | Gateway 只写 protocol projection/cursor，不写 Runtime authority | `CGP-08` 使旧 handlers/protocol alias caller-zero/deletion-ready；`XRET-04` 唯一物理删除 |
| `AK2-18b` daemon/socket/ACP | `CGP-01 + CGP-04 + CGP-05` | `CGP-02/03` | `CGP-04` 切 official user daemon/socket writer；ACP 只走 typed client | `CGP-08` 证明第二 composition/socket path inert；`XRET-04` 唯一物理删除 |
| `AK2-19a` TUI/CLI command-only | `CGP-06` | `CGP-02 + CGP-04` | 无 core writer；TUI 只分配 `UiOverlayId` | `CGP-07/08` 使 legacy Session/RPC seam caller-zero；`XRET-04` 删除 Executive remnant |
| `AK2-19b` Presentation split | `CGP-07` | `CGP-06` | 无 authority cutover；只拆 model/controller/renderer | `CGP-08` 产 deletion evidence；`XRET-04` 唯一删除 Executive host compatibility state |
| `AK2-20` Gmail cutover | 唯一联合 PR `E2-K6a` | `E0/E1 + K6 + APX-03` | Gmail cursor/outbox/send writer 与 Kernel outbound executor 一次切换 | `XRET-03 -> E7` |
| `AK2-21` Hardware cutover | 唯一联合 PR `E4-K6b` | `E0/E1 + K6` | Hardware lease/safety/command receipt 与 Kernel generic permit 对齐 | `XRET-03 -> E7` |
| `AK2-22` Robot VLA cutover | 唯一联合 PR `E5-K6c` | `E4-K6b + D2 + K6`；若 E0 发现 Mnemosyne episode installed data/writer 非零，另等 `D4` owner seam + `APX-04` SQLite migration/rollback readiness | Robot episode/dispatch writer 切换；Hardware veto 不被覆盖；零 caller/data 的 duplicate 走 exact DELETE 而不制造迁移 | `XRET-03 -> E7` |
| `AK2-23` remaining Linux adapters | `E1/E3 + K5 + CGP-04` 对应 owner PR | 相关 domain/Application ports | 每个 adapter 单独 drain 后切自己的 writer/executor；不得批量双写 | `XRET-03 + K7 + CGP-08 -> E7` |
| `AK2-24` pre-removal acceptance | supported `E2-K6a/E3/E4-K6b/E5-K6c/E6-K6d` + `XRET-03`，并消费 `RA-06/APX-05/K7/CGP-08` 对应 family 的 caller-zero evidence | 每个 extension row 只等待实际引用它的 Runtime/Application/Kernel/host family gate；禁止用全局互等制造环 | 无新 writer；核验 installed provenance、rollback/reconcile、canary | 对应 family 全绿后才进入 `E7 -> D6 -> AK2-25` |
| `AK2-25` Contracts rename | `E7` 先删 extension rows，`D6` 再做 non-extension/root closeout，随后开唯一机械 rename PR | `XRET-03 -> E7 -> D6`，rich type/re-export/alias writer 均为零 | 无行为或 schema writer 变化；只做 `fabric -> contracts` mechanical rename | source/manifest/lock/docs 的 `fabric` compatibility path hard zero，才可 `XRET-04` |
| `AK2-26` Executive empty-shell removal | `XRET-04`（复用已交付 `CGP-08` gate） | `K7 + RA-06 + D6 + APX-05 + CGP-08 + E7 + AK2-25` 各自 per-slice gate | 无 writer；只删 host/composition/compat remnants | `XRET-04` 验收后才可 `XRET-05` |
| `AK2-27` final delete/soak | `XRET-05` | `XRET-04` | 无 writer；机械删除 crate 后验证 installed binary | resolved graph/source hard zero + fault injection/soak |

规范 DAG 中，横向 owner PR 可并行，删除链不能改序：

```text
RA-00 -> RA-01 -> RA-02 -> RA-03 -> RA-04 -> RA-05 -> E6-K6d -> RA-06
K0 -> K1 -> K2 -> K3 -> K4 -> K5 -> K6
  K1 <- RA-01 + APX-00
  K2/K3 <- D1 contracts + APX verifier design
  K4 <- RA-04 + owner migration composition
  K5 <- CGP-01
  K6 <- E0/E1
  K6 -> E2-K6a/E4-K6b/E5-K6c/E6-K6d
  RA-06/APX-05/CGP-08 + matching extension caller-zero -> K7 per retired Kernel-glue family
D0 -> D1 -> D2/D3/D4
D1 + K2 -> D5
D2 + D3 + D4 + D5 -> D6
RA-05 -> APX-01
APX-00 -> APX-01 -> APX-02/APX-03/APX-04
APX-02 + APX-03 + APX-04 -> APX-05
RA-03/RA-04/RA-05 + D2/D3/D4/D5 + K4/K6 + matching E2..E6 cutover -> APX-05 per file family
APX-05 caller-zero evidence -> RA-06/D6/K7/E7 only for matching deletion families
CGP-00 -> CGP-01
CGP-00 -> CGP-02
CGP-01 + CGP-02 -> CGP-03 -> CGP-04
CGP-04 -> CGP-05 + CGP-06
CGP-06 -> CGP-07
CGP-05 + CGP-07 -> CGP-08
supported E2-K6a/E3/E4-K6b/E5-K6c/E6-K6d -> CGP-08 for matching host/extension wiring
E0 -> E1 -> E2-K6a/E3/E4-K6b/E6-K6d
D2 + E4-K6b -> E5-K6c
E0 finds Mnemosyne episode caller/data != 0: D4 + APX-04 migration/rollback -> E5-K6c episode-writer slice
E2-K6a/E3/E4-K6b/E5-K6c/E6-K6d + XRET-03
  + matching RA-06/APX-05/K7/CGP-08 caller-zero evidence -> E7 per extension row

APX-04 + D2 + K5 + CGP-04 + supported E2-K6a/E3/E4-K6b/E5-K6c/E6-K6d + plugin decision -> XRET-03
E7 per-family gates all green -> E7 extension rows -> D6 non-extension/root closeout -> AK2-25 mechanical Contracts rename
K7 + RA-06 + APX-05 + CGP-08 + D6 + E7 + AK2-25 -> XRET-04 -> XRET-05
```

`XRET` 是删除 gate/证据 owner，不接管被迁能力的 writer。尤其不得把 `XRET-04` 提前到 `AK2-25` 之前：先完成 concrete adapter drain（`XRET-03`），再由 E7 清 extension rows、D6 收 non-extension/shared root，之后机械 rename，随后才允许删除 host/composition/compatibility remnants，最后 `XRET-05` 机械删除 crate。

## 6. 计划 PR 与实现 PR 的关系

这些文档适合拆成多个 Draft plans PR 分别审阅，但实现时仍要保持以下规则：

| Draft plans PR | 建议分支 | 内容 | 逻辑前置 |
|---|---|---|---|
| `docs(plan): index Executive decomposition and file ledger` | `plan/executive-decomposition-index` | 本索引 + 逐文件处置账本 | 上位计划 |
| `docs(plan): consolidate Runtime authority` | `plan/runtime-authority-consolidation` | Runtime 专项 | 索引中的 owner 决议 |
| `docs(plan): consolidate Kernel enforcement` | `plan/kernel-enforcement-consolidation` | Kernel 专项、Linux effect/process adapters | 索引中的 owner 决议 |
| `docs(plan): extract domain authorities and adapters` | `plan/domain-authority-adapters` | 六领域 + Contracts thinning | 索引中的 owner 决议 |
| `docs(plan): separate Application from persistence and host I/O` | `plan/application-persistence-boundary` | Application 专项 | Runtime/Domain port 名称稳定 |
| `docs(plan): extract composition Gateway and presentation` | `plan/composition-gateway-presentation` | daemon/Gateway/TUI/ACP | Runtime/Application command surface 稳定 |
| `docs(plan): preserve and cut over supported extensions` | `plan/preserved-extensions-cutover` | Gmail/Pi/GBrain/Robot/Hardware | preservation E0 可先行；最终切换依赖 owner ports |
| `docs(plan): retire Executive with fitness gates` | `plan/executive-retirement-gates` | compatibility 与最终删除 | 前七条的 deletion gates；最后删除必须晚于 Contracts rename |

以上分支都应从最新 `dev` 独立创建 Draft PR，避免 plans PR 彼此堆叠造成审阅假依赖；正文用链接表达逻辑依赖。真正的实现 PR 再按各专项编号执行。

- 每个实现 PR 只切一个 authority writer 或一条 adapter 路径；
- 新旧路径只允许单向 facade、shadow read 或 backfill；禁止双执行、双 terminal、无界双写；
- 同一 PR 不同时迁 Gmail、Robot、Memory 和 Self；
- 删除旧路径必须引用具体 production caller 已切换的证据；
- compatibility seam 必须写明最后消费方和删除 PR；
- 迁移数据库允许双版本读取，不允许两个 owner 同时写同一 aggregate；
- 每个 PR 有独立回滚点，真实外部副作用依赖 idempotency/reconciliation，不能回滚数据库后盲重放。

## 7. 全系列共同硬约束

- 新 Agent/Session/Turn ID 只能由 Runtime assignment owner 产生；
- Kernel 在 V2 固定为独立小型 enforcement crate；只允许按 K0–K7 收缩 public surface，禁止迁移中物理并入 Runtime；
- 一个 production Turn reducer、一个 Session writer、一个 AgentRun writer；
- Kernel 不理解 prompt、Self、Robot、Provider、Session；
- Runtime 不依赖 SQLite、HTTP、Unix socket、systemd、TUI、Gmail、Robot、Hardware；
- Application 不构造 concrete adapter，不直接使用 SQLite/文件系统/子进程；
- Gateway 不持有 Kernel、Dasein、Mnemosyne concrete service；
- TUI/CLI/ACP 不 mint core ID，不写 repository，不推导 effective permission 或 runtime policy；
- Dasein 保持活跃并成为 Self mutation 的唯一 commit owner；
- Metacog 保持活跃，但只观察、评估、提出和实验，不占据 Turn terminal 热路径；
- Gmail、Pi、GBrain、Robot VLA、Hardware 都是 `PRESERVE-MOVE`；
- Robot/Hardware 真实执行器在 HIL/实机证据前继续声明 unsupported；
- `fabric` 只有在 rich types 全部归还 owner 后才改名为 `contracts`；名称明确不是 `abi`；
- Executive 不能长期保留为“协调层”；最后一个业务模块迁出后删除整个 crate。

## 8. 不采用的方案

- 不把 `executive` 整体改名为 `runtime`；
- 不把 `TurnPipeline` 或 `AgentControlService` 原样搬入 Runtime；
- 不在 `contracts` 中建立新的共享领域模型仓库；
- 不先删所有测试再失去 Gmail/Robot/Hardware 的唯一行为证据；
- 不把所有 SQLite 表并成一个万能 repository；
- 不用 Cargo feature 隐藏反向依赖；
- 不把 transient event bus 当作 journal；
- 不把 Pi、Provider、Native Cognit 都称为 `Runtime`；
- 不因模块当前复杂就关闭 Self 或 Metacog。

## 9. 本系列完成定义

- 每个重复实现都有 `authority/projection/cache/transport/compatibility/dead/preserve-move` 标签；
- 每个 authority 只有一个 owner、一个 ID assignment point、一个 writer；
- 每个 Executive production 模块都有目标 package 和删除 PR；
- 每个 compatibility seam 有最后消费者、deadline 和删除 gate；
- 七份专项计划的阶段/PR 顺序无循环依赖；
- Gmail/Pi/GBrain/Robot/Hardware 的迁移门早于相关旧代码删除；
- 最终 resolved dependency graph 中不存在 `executive`，也不存在 `interact -> runtime/kernel/domain adapter`；
- `contracts` public surface 通过预算检查且无 rich domain aggregate。

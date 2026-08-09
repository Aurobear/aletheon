# Runtime 权威收敛与重复实现退役计划

> 日期：2026-08-08
> 基线：`dev@bd1ceac2965832f15cf45b811fd34e052e7e9a67`
> 上位计划：`2026-08-08-agent-kernel-v2-complete-rearchitecture.md`
> 状态：可执行计划；本文不改生产代码
> 代号：`RA`（Runtime Authority）

## 1. 结论与边界

当前不是完全没有 Agent Core，而是 Agent Core 的语义分别长在 `executive` 的 Turn、Session、AgentControl、daemon bootstrap、兼容服务和 adapter 中；现有 `runtime` crate 反而只有 external runtime manifest/selector。只移动目录会把重复 constructor、状态机和 writer 原样复制进新 crate。

最终决定：

1. `runtime` 是 Agent、AgentRun、Session、Turn、Delegate 生命周期的唯一 owner；
2. 每个 aggregate 只有一个构造入口、一个状态机、一个 journal writer；
3. 新 `AgentId/AgentRunId/SessionId/TurnId` 只由 Runtime assignment path 产生；
4. Kernel 只治理 ExecutionProcess、Operation、Permit、Lease，不保存 Agent 语义；
5. Native Cognit 是单 Turn `CognitiveRun`，Pi 是受监督 `DelegateBackend`，两者都不是第二个 Runtime；
6. 迁移只允许单 writer；禁止同一 aggregate 双写、双执行或双 terminal；
7. Executive 中迁出的实现按纵向切片删除，最后不保留改名空壳；
8. 不搬运整套旧测试，只保留少量 Runtime 不变量和正式安装态 Native/Pi smoke。

本文负责 Session/Turn/Agent/Delegate、runtime registry、ID assignment、RuntimeJournal 及 Executive 对应重复实现的迁移退役；不负责 TUI/RPC 的完整物理拆分、领域内部重构、Kernel permit 细节或 `fabric -> contracts` 的全量机械迁移。相关计划必须服从本文 owner 和单 writer 约束。

## 2. 当前基线事实

- `crates/runtime/src/lib.rs` 只导出 manifest/selector，不拥有 Agent、Session、Turn 或 journal；
- `executive/application/turn_coordinator.rs` 会创建 Kernel Operation、分配 TurnId、隐式创建 Session、写 items 并 terminal settlement；
- `executive/application/turn_pipeline.rs` 同时编排 Self、Memory、Agora、Kernel、Cognit、Hook、工具和 post-turn；
- `executive/application/agent_control/` 已有完整 AgentRun 状态、registry、recovery、mailbox、settlement 和 SQLite repository；
- `executive/application/session_service.rs` 同时承担 lifecycle、active-turn lookup、canonical store 和独立 `protocol_events` journal；
- `host/daemon/session_manager.rs` 名为 SessionManager，实际是 process-local message/context working set；
- `compatibility/legacy_session_service.rs` 又维护 registry、workspace mapping、compaction 和 canonical projection；
- `core/session.rs` 仍公开另一套 Session、ContextState、TuiSessionManager，并自行计算 permission/tool policy；
- `core/runtime_registry.rs::RuntimeRegistry` 与 `agent_control::AgentRuntimeRegistry` 都能注册/选择子执行 runtime；
- `fabric::AgentRuntimeCapability` 与 `runtime::RuntimeCapability` 字段逐项重复并手工转换；
- `fabric::AgentId(Uuid)` 与 `fabric::ipc::AgentId = u64` 同名异义；
- TUI reducer、CLI one-shot、Native adapter 和多个 Executive workflow 都能直接 mint TurnId 或新 Session 字符串；
- generic EventSpine、CanonicalEventBus、Session append store、AgentRun repository、legacy Session DB 和 protocol event DB 并存，没有逐 stream 唯一 writer 清单。

## 3. 目标模型

```text
runtime/
├── command.rs               # typed create/submit/spawn/cancel commands
├── query.rs                 # authoritative snapshots/projection facts
├── ids.rs                   # Runtime-only assignment services
├── agent_supervisor.rs      # Agent/AgentRun/Delegate lifecycle
├── session_authority.rs     # Session aggregate/principal binding
├── turn_coordinator.rs      # 唯一 Turn state machine/terminal fence
├── runtime_router.rs        # Native/Delegate route decision
├── delegate_backend.rs      # Pi 等 external backend port
├── context_working_set.rs   # process-local、可重建 context
├── context_assembler.rs
├── recovery_coordinator.rs
├── journal.rs               # Agent/Session/Turn streams
├── projection.rs
└── ports.rs                 # Kernel/Cognit/Dasein/Memory 窄 port
```

| 语义事实 | 唯一 owner | durable writer | 外层只能做什么 |
|---|---|---|---|
| Agent identity/generation/profile binding | Runtime `AgentSupervisor` | `RuntimeJournal::AgentStream` | spawn/query；引用已有 ID |
| AgentRun/Delegate wait/cancel/terminal | Runtime `AgentSupervisor` | Agent stream | 观察 receipt，不能自行 terminal |
| Session create/fork/status/principal | Runtime `SessionAuthority` | Session stream | create/resume/fork/query command |
| Turn state/terminal/recovery | Runtime `TurnCoordinator` | Turn stream | submit/continue/cancel/query |
| Agent-to-execution binding | Runtime | Agent stream | Kernel 只返回 execution identity/receipt |
| Operation/Permit/Lease/accounting | Kernel | `ExecutionJournal` | Runtime 保存引用，不复制状态 |
| active context cache | Runtime `ContextWorkingSet` | 不 durable | 从 journal/projection 重建 |
| client cursor/reconnect view | Gateway projection | `ClientEventLog` 或可重建 projection | 不推进 Session/Turn |
| backend selection decision | Runtime `RuntimeRouter` | Agent/Turn stream decision fact | 外层只提交 requirement/hint |
| Pi OS/protocol execution | Pi adapter + Kernel process permit | Kernel receipt；Runtime 关联 | 不创建 Session/Turn/Self |

最终只允许三类 Session-shaped 数据：`SessionAggregate`（authority）、`SessionProjection/SessionView`（read side）、`ContextWorkingSet`（cache）。任何第四类必须登记 owner，否则不能合入。

## 4. 重复实现清单：KEEP / MIGRATE / DELETE

### 4.1 Session

| 当前位置 | 实际职责/问题 | 决定 | 最终处理 |
|---|---|---|---|
| `fabric/types/session.rs::{SessionRecord, SessionReadStore, SessionAppendStore}` | domain model/repository 位于共享层，任意调用者可构造 record | **MIGRATE** | aggregate/event/port 归 Runtime；最小 DTO/ID 归 `contracts`；旧 re-export 逐调用者删除 |
| `application/session_service.rs::SessionService` | lifecycle、active-turn、storage、protocol reconnect 混合 | **MIGRATE+DELETE** | lifecycle -> SessionAuthority；active index -> TurnCoordinator；protocol log -> Gateway；RA-06 删除类 |
| `turn_coordinator.rs` 隐式 Session create | caller thread ID 被直接当新 Session identity | **MIGRATE** | 只调用 SessionAuthority；unknown caller ID 不再隐式创建 |
| `host/daemon/session_manager.rs::SessionManager` | message/cache/token/compaction working set | **KEEP+RENAME** | -> `ContextWorkingSet`；无 durable writer/Session constructor |
| `compatibility/legacy_session_service.rs` | 第二 registry、create/switch/clear/compact path | **SEAM+DELETE** | 只持 Runtime command/query port；RA-06 删除 |
| `core/session.rs::{Session, ContextState, TuiSessionManager}` | UI、policy、context 冒充 core Session | **DELETE** | cache/policy/view 分别迁 owner；RA-03 前删除 |
| `adapters/session/store.rs::{SessionStore, SessionRecord}` | 整包 `messages_json` legacy writer | **READ SEAM+DELETE** | 只读 import/alias；cutover 后停写并最终删除 |
| `CanonicalSessionStore` | SQLite materialized model | **KEEP+RENAME** | projection store，禁止 originate event |
| `EventSourcedSessionStore` | 当前 canonical append 切片 | **MIGRATE** | 逻辑 -> `SqliteRuntimeJournal`；先复用物理 schema |
| `core/session_gateway/SessionGateway` | context、approval、snapshot、legacy manager 混合 | **SPLIT** | Gateway 留 command/query client；context 回 Runtime；approval 回 Application |
| TUI SessionPicker/SelectedSession | presentation state | **KEEP** | 明确叫 SessionView/SelectedSession，只吃 projection |

### 4.2 Turn

| 当前位置 | 实际职责/问题 | 决定 | 最终处理 |
|---|---|---|---|
| `turn_lifecycle.rs::TurnPipelineLifecycle` | 小型 Pre/Cognitive/Post state，与 durable Turn 不一致 | **MIGRATE+DELETE** | 有效 transition 合入 TurnAggregate，不保留并行 lifecycle |
| `turn_coordinator.rs::TurnCoordinator` | 最接近 canonical，但直接依赖 Kernel/旧 store并隐式建Session | **KEEP+MIGRATE** | -> `runtime::TurnCoordinator`，拆 Kernel port/SessionAuthority/Journal |
| `turn_pipeline.rs::TurnPipeline` | 巨型 orchestration，有第二套 rejection/terminal 语义 | **MIGRATE+DELETE** | context/gate/cognition/effect 变窄 port；terminal 只能回 coordinator |
| `application/daemon_turn/*`、`daemon_turn_engine.rs` | daemon 持有 Turn orchestration | **MIGRATE+DELETE** | Host 只翻译 command/event |
| `composition/turn_service.rs::TurnService` | facade 自己创建 in-memory coordinator/store | **SEAM+DELETE** | 只注入同一个 Runtime port，不得自组 authority |
| `composition/turn_coordinator.rs` | journal/projection/coordinator composition | **MIGRATE** | adapter 组装到唯一 daemon composition root |
| `turn_engine.rs`、`turn_runtime_ports.rs`、`harness_factory.rs` | cognition/provider/harness route，Runtime词汇重叠 | **SPLIT** | Native 单 Turn统一为 `CognitiveRun`/`InferencePort`，不拥有 durable Turn |
| `fabric::{TurnRecord, ItemRecord, TurnSettlement}` | transport/domain event/projection混合 | **MIGRATE** | Runtime event归 Runtime；Gateway DTO独立；旧schema只读兼容 |
| TUI reducer `TurnId::new()` | optimistic state伪造真实 ID | **DELETE** | 使用 `UiOverlayId`，收到 `TurnAccepted` 后绑定 TurnId |

Canonical Turn 状态固定为：

```text
Accepted -> Started -> ContextReady -> CognitionActive
-> WaitingForAuthority | WaitingForDelegate | Executing
-> Verifying -> Settling -> ReconciliationPending
-> Succeeded | Failed | Cancelled | TimedOut | Indeterminate
```

`ReconciliationPending` 非 terminal；每个 Turn 只能写一个 terminal。有效晚到副作用仅追加 `LateExecutionObserved`，不能改写 terminal。

### 4.3 Agent/AgentRun

| 当前位置 | 实际职责/问题 | 决定 | 最终处理 |
|---|---|---|---|
| `application/agent/mod.rs::AgentRuntime` | 内存 Agent/Task map，与 AgentControl并存 | **DELETE** | 无生产调用则 RA-03 直接删，不迁空壳 |
| `application/agent_control/AgentControlService` | 真正 spawn/send/wait/cancel/recovery/settlement core | **KEEP+MIGRATE** | 分模块 -> Runtime `AgentSupervisor` |
| `agent_control::AgentRunRepository` | AgentRun、mailbox、lease、runtime process混合 | **MIGRATE+SPLIT** | Agent事实 -> Agent stream；Kernel事实只保存引用 |
| `SqliteAgentRunRepository` | 重要历史表和恢复数据 | **KEEP ADAPTER** | 先由 RuntimeJournal适配原schema；切换后删旧trait写入口 |
| `fabric::AgentSpawnRequest` | caller携带root/parent/runtime与authority-like字段 | **MIGRATE** | `SpawnDelegateCommand` 不携带新child ID/generation |
| `fabric::{AgentHandle, AgentSnapshot, AgentRunStatus}` | domain state被外层当可构造 DTO | **SPLIT** | aggregate/event归Runtime；Gateway只出immutable projection |
| `core/sub_agent.rs::SubAgentRuntime` | external backend trait名似第二Runtime | **MIGRATE+RENAME** | -> `DelegateBackend`；Native不实现它 |
| `core/orchestrator.rs` compatibility registry | goal worker直接resolve backend | **SEAM+DELETE** | 改交 Runtime SpawnDelegate，不碰backend |
| 两个 `composition/**/AgentLoader` | baseline census 已证明 `composition/agents` 只有自身单测、无生产 caller；`composition/agent_loader` 被 daemon bootstrap 使用 | **DELETE DUPLICATE + RENAME SURVIVOR** | foundation PR 删除 inactive loader；生产 loader 在 RA-05 迁为 Runtime profile port，并精确命名 `MarkdownAgentProfileLoader`，不再使用裸 `AgentLoader` |
| `RuntimeCore/UserRuntime/SystemCoreRuntime` | 三个容器均声称Runtime | **SPLIT+RENAME** | 仅 `runtime::AgentRuntime` 表示 canonical Agent Runtime；system若保留只能是无 Agent 状态的 `InferenceBroker`/host，不使用 `Service` 或 `Runtime` 冒充 |

### 4.4 Registry 与 capability

| 当前实现 | 决定 | 最终形态 |
|---|---|---|
| `runtime::{RuntimeManifest, RuntimeSelector}` | **KEEP+收窄** | external部分 -> `DelegateBackendManifest/Selector`；通用requirement归RuntimeRouter |
| `core::RuntimeRegistry` | **SEAM+DELETE** | 单向委托唯一registry；goal caller清零后删 |
| `agent_control::AgentRuntimeRegistry` | **KEEP+MIGRATE** | -> Runtime `DelegateBackendRegistry`；package ownership留registration adapter |
| `fabric::AgentRuntimeCapability` | **DELETE DUPLICATE** | 与 Runtime capability统一为单一语义类型，删除手工conversion |
| `runtime::RuntimeCapability` | **KEEP** | 演化为 Runtime-owned `ExecutionCapabilityRequirement` |
| `NativeCognitRuntime` | **KEEP+RENAME** | `NativeCognitiveRun`，不进入 external registry |
| `PiRuntime/PiRpcRuntime` | **MIGRATE+RENAME** | E6 收敛为精确命名的 `PiDelegateBackend`；保留 Pi manifest/protocol/wait/recovery，删旧 `Runtime`/launcher 命名与旁路 spawn |
| plugin/package runtime loader | **KEEP ADAPTER** | 只能由composition root注册唯一registry，不能执行或持Agent状态 |

最终每个进程只有一个 `DelegateBackendRegistry`；duplicate ID、invalid manifest、unsupported governance fail closed。运行中的 AgentRun 固定 backend generation，package reload 不能偷换 binding。

### 4.5 ID assignment

| 当前身份/构造点 | 问题 | 唯一 owner与处理 |
|---|---|---|
| `fabric::AgentId(Uuid)::new/default` | 任意crate能创建真实Agent | Runtime `AgentIdSource`；production移除无语义Default；wrapper可进contracts |
| `fabric::ipc::AgentId=u64` | 同名异义 | 改 `IpcEndpointId` 或删除；不得转logical AgentId |
| `AgentSpawnRequest.root/parent_agent_id` | caller参与child identity构造 | parent仅引用；child ID/generation由AgentSupervisor分配 |
| `SessionId(pub String)` 与CLI/TUI/daemon构造 | unknown ID会隐式建Session | Runtime `SessionIdSource`；create无caller ID；旧字符串只作`LegacySessionAlias` |
| `TurnId::new/default` 与多处直接调用 | 一次动作可产生多个真Turn | Runtime `TurnIdSource`；TUI用UiOverlayId；adapter只接收已有TurnId |
| `TurnCoordinator` 当前mint | 主要正确点但物理在Executive | 迁Runtime，`TurnAccepted` durable后才返回 |
| `SessionService::fork`随机child | Application创建aggregate | fork command无child ID，SessionAuthority分配 |
| extension/host手造OperationId/ProcessId | 绕过Kernel | 生产只通过Kernel port取得；deterministic ID限fixture |
| correlation/request/event ID | 容易与authority混用 | 明确命名；只关联，永不授权/创建aggregate |

ID wrapper在`contracts`只负责编码关联。真正强制点是 create command不接caller ID、Runtime先append后receipt、resume/query验证principal/generation/scope，以及静态gate禁止Runtime外production mint。

### 4.6 Journal/store/event bus

| 当前实现 | 决定 | 最终处理 |
|---|---|---|
| `fabric::EventSpine` + `SqliteEventSpine` | **SEAM/MIGRATE** | 可暂作RuntimeJournal物理adapter；append不再全领域公开 |
| `CanonicalEventBus` | **KEEP NOTIFICATION ONLY** | 不能作为replay、terminal或权限证据 |
| `SessionAppendStore/EventSourcedSessionStore` | **MIGRATE** | -> Runtime Session/Turn streams；projection只读 |
| `CanonicalSessionStore` | **KEEP PROJECTION** | 不得originate Session/Turn event |
| `AgentRunRepository/SqliteAgentRunRepository` | **MIGRATE** | -> AgentStream adapter；原表可原位读，禁止第二writer |
| `SessionService.protocol_events` | **SPLIT** | -> Gateway ClientEventLog/可重建projection；不写Turn terminal |
| legacy `SessionStore/sessions.db` | **READ SEAM+DELETE** | importer/alias only；cutover后停写 |
| Agora commit/broadcast | **KEEP OUTSIDE** | workspace事实，不恢复/结算Session/Turn |

`RuntimeJournal` 至少隔离 `AgentStream(agent_id,generation)`、`SessionStream(session_id,revision)`、`TurnStream(turn_id,generation)`。跨stream用 saga + correlation + idempotency；不假装与 Kernel ExecutionJournal 存在全局事务。每个event带stream sequence、schema、causation、writer generation和payload digest。

## 5. Pi 能力保全

Pi 不是清理对象。切换前冻结并验证：

- runtime ID/alias、capability/profile/workspace/task encoding选择；
- spawn task、tool/workspace/budget/deadline/credential/sandbox attenuation；
- 已支持的 one-shot/resident/steering/follow-up；
- 未观察 authoritative terminal receipt 前绝不 success；
- cancel正确generation与process group，防PID复用误杀；
- daemon restart后reconcile，unknown不能伪装success；
- stdout/stderr/exit/timeout/artifact evidence有界可追踪；
- mediated与observed/opaque如实区分；opaque只声称process-level lease；
- key/token不进argv、prompt、event或普通日志；
- stale generation不推进当前AgentRun，有效晚到effect仍审计/accounting。

固定顺序：冻结legacy contract/installed smoke -> 旧path单向经Runtime facade调用Pi -> AgentSupervisor/Journal取得child writer -> Pi实现DelegateBackend并由Kernel治理process -> installed spawn/wait/cancel/restart等价 -> 才删旧registry/facade。

Native先切换不能成为删除Pi配置、protocol、recovery表或registry的理由；shadow不得真实spawn/send/cancel；Pi切换失败保持legacy-authoritative，不能以关闭能力通过架构验收。

## 6. 兼容 seam 与切换

| 模式 | 唯一writer/executor | V2 | 旧路径 |
|---|---|---|---|
| `legacy-authoritative` | Executive旧path | inventory/read-only compare | 唯一写入执行 |
| `v2-shadow` | Executive旧path | replay/compare；禁止append/spawn/effect | 唯一写入执行 |
| `v2-authoritative` | Runtime V2 | 唯一写入执行settlement | 单向facade调用Runtime |

模式可按 Native/Pi 切片，但每个具体 aggregate 在创建时固定 `authority_generation`，运行中不能换writer。

旧Session字符串兼容：已存在ID在维护窗口导入并保留；新CreateSession不接caller ID；旧协议的unknown字符串由Gateway标记`LegacySessionAlias`，Runtime原子记录alias mapping和SessionCreated，重复请求幂等、跨principal冲突fail closed。typed协议完成后删除alias import，普通SubmitTurn不再隐式建Session。

每个 facade 必须无repository/background worker、不得`new`自己的coordinator/registry，只持一个typed command/query port；仅转换schema，不计算effective permission、route或terminal，并标明删除PR。

每次stream切换必须记录 old/new schema、old/new writer symbol、read范围、import watermark、cutover generation、rollback binary范围和reconciliation owner。切换时maintenance -> drain/cancel -> watermark/import -> 启新binary -> 验唯一writer -> 恢复流量；禁止长期双写。

## 7. 实施 PR 序列

```text
RA-00 census + ownership gates
RA-01 Runtime contracts + owner-assigned IDs + command/query seams
RA-02 RuntimeJournal adapter + read-only replay shadow
RA-03 SessionAuthority + ContextWorkingSet + legacy seam
RA-04 canonical Native Turn cutover
RA-05 AgentSupervisor + unified backend selection
E6    Pi extension implements DelegateBackend and owns Pi cutover
RA-06 remove duplicate Executive authorities/seams after E6
```

主 writer 链严格依赖 `RA-00 -> 01 -> 02 -> 03 -> 04 -> 05 -> E6 -> RA-06`。RA-05 只建立 generic `DelegateBackend` seam/registry，不迁 Pi 文件；Pi adapter、协议、active child 与生产 caller 切换唯一由扩展计划 E6 执行。Host/Executive extraction 可从 RA-02 后并行；RA-06 除 Native 与 E6 Pi 安装态验收和 rollback drill 外，还必须按待删 file family 消费 `APX-05` 及其他已登记 host caller-zero evidence，不要求无关 family 互等。每个 PR 从最新 dev 建短分支，不维护跨阶段 Mega branch。

| PR | 主要动作 | 退出条件 | 回滚边界 |
|---|---|---|---|
| **RA-00** | 固化constructor/ID/writer/table/composition census；Session-like分类；重复symbol只减不增gate；Pi/Native基线；旧测试分类 | 每个stream有现任/目标writer；全部production mint点分类；Pi smoke可重复 | 纯inventory/gate，可回规则，无行为/数据变化 |
| **RA-01** | 最小commands/events/queries/ports；Runtime ID sources；Create/Spawn不收新ID；optional opaque client-correlation/LegacyAlias binding；统一 capability 语义。`UiOverlayId` 仅由 CGP-06/Interact local 定义，Runtime 不解析、不存储该类型 | caller不能声明新aggregate；Runtime外新增mint被拒；旧path仍唯一writer | 删除未启用API/seam，旧writer未变 |
| **RA-02** | 定义三类streams；先适配现有SQLite/EventSpine/agent_runs；bounded replay；只读projection shadow；分离notification | shadow零append/spawn/effect；replay差异typed；unknown schema fail closed | 关shadow即可，无逆迁移 |
| **RA-03** | 迁Session create/fork/principal；SessionManager->ContextWorkingSet；SessionService变facade；legacy store停写；protocol log拆Gateway；删TuiSessionManager | 一个Session constructor/writer；working set可重建；list/resume/fork/compact等价；client不mint | writer切前可回legacy；切后只回兼容新schema且仍走Runtime的binary |
| **RA-04** | 迁TurnCoordinator；合并lifecycle；拆TurnPipeline窄ports；Native只做CognitiveRun；TUI用UiOverlayId；post-settle outbox | 一个Turn状态机/ID/terminal writer；cancel/timeout/restart无false success；正式安装Native多轮通过 | 旧facade仍调Runtime，不重开旧writer；side effect只reconcile不重放 |
| **RA-05** | 迁AgentControl模块；AgentRun repo适配AgentStream；Runtime分配child/generation/run ID；建立唯一 generic router/`DelegateBackendRegistry`；caller改SpawnDelegate；删dead AgentRuntime和已证实等价的 loader/capability duplicate；语义不同的 loader 精确改名；保留单向 legacy Pi adapter | 一个spawn/send/wait/cancel/recovery入口；backend reload不换运行中binding；无 Pi 文件迁移或双执行 | Pi未切时保留单向legacy backend seam；Agent writer切后只回兼容Runtime binary |
| **E6（外部依赖）** | 由扩展计划迁 Pi adapter/协议，执行 active-child drain/reconcile、production caller/writer cutover 与 installed drill；Runtime PR 不重复修改这些文件 | wait前不success；cancel无孤儿；restart可reconcile；secret不泄漏；无双spawn/writer | 按 E6 maintenance/rollback 规则回兼容 adapter/config，绝不重开同 generation 旧 writer |
| **RA-06** | E6 验收且 matching `APX-05`/host caller-zero evidence 到位后，只删除 Runtime-owned 旧 AgentControl/registries/SubAgentRuntime、TurnPipeline/TurnService authority、SessionService/LegacySession/SessionStore writer、generic legacy backend seam、已证实等价的 loader/enum/mint/re-export/旧实现测试；不得删除 Executive host/composition/compat tree 或 crate | 静态证明一个constructor/state machine/writer/registry；full-linux Native/Pi与rollback drill通过；每个待删 seam 的直接/间接 caller 为零 | 至少一版稳定V2后做；只回兼容V2 journal的artifact，不恢复旧writer；空 Executive crate 唯一由 `XRET-05` 删除 |

每个PR固定提交：owner与非目标、before/after依赖和writer图、K/M/D符号清单、schema/writer manifest、feature删除PR、Native/Pi矩阵、rollback步骤、必要证据、删除与剩余债务。机械rename、writer切换、全协议重写不能塞进同一PR。

## 8. 验收与证据

### 8.1 静态 gate

最终必须成立：

```text
runtime !-> executive/interact/tui/gateway adapters
kernel !-> Agent/Session/Self semantics
gateway/tui !-> RuntimeJournal writer
Pi adapter !-> SessionAuthority/TurnJournal direct writer
one DelegateBackendRegistry construction
one RuntimeJournal writer per stream/generation
no production TurnId::new or AgentId::new outside runtime
no client-side SessionId mint for creation
no mirrored capability enum/conversion
```

同时扫描全部 SessionManager/Service、RuntimeRegistry、AgentRuntime、TurnCoordinator/Pipeline 定义和构造点；SQLite mutation writers；SessionCreated/TurnSettlement/AgentTerminal producers；blanket lint allow与oversized constructor增长。

### 8.2 少量核心不变量

- owner-assigned ID与append-before-receipt；
- sequence/generation fence/single terminal；
- cancel/deadline/drop/restart/replay；
- duplicate command幂等且不双执行；
- stale/late evidence不改current generation；
- Pi wait/recovery/process cleanup；
- secret redaction和sandbox unavailable fail closed。

旧测试只在对应实现删除时分类清理；不能在Pi preservation baseline前删掉其唯一行为证据。

### 8.3 正式安装态 smoke

影响writer、daemon、IPC、Pi或持久化的PR必须用正式部署binary和official user socket验证：Native普通Turn、同Session多轮、cancel后无污染、daemon restart/reconnect、Pi spawn->wait->terminal、Pi cancel/timeout/restart、permission/sandbox fail closed，以及release artifact、`/usr/bin/aletheon`、running daemon digest一致。临时daemon/mock socket只算诊断。

关键故障注入：start成功但terminal append失败；Kernel receipt晚到/过generation；Pi在ready前/执行中/terminal ack后崩溃；daemon在SessionCreated/TurnStarted/DelegateSpawned/TurnSettled边界重启；projection损坏；provider rejection/backpressure；opaque sandbox不可用。结果必须typed failed/cancelled/indeterminate/reconciliation pending，不能从日志字符串猜success。

## 9. 回滚协议

RA-00至RA-02未切writer，可直接关闭gate/shadow或回binary；shadow不得留下数据/副作用。

RA-03以后：maintenance停止新Turn/spawn -> drain/cancel active Native/Pi -> 保存watermark/writer generation/child清单 -> 回到能读当前schema的上一binary/config -> writer仍走Runtime单入口 -> 已执行未确认副作用只reconcile不重放 -> projection可重建但不得反向覆盖journal -> installed smoke和writer检查通过后恢复流量。旧binary不能读新event时保持maintenance并前向修复，不能启动双writer应急。

Pi回滚额外记录每个active child的AgentId/generation/process identity/start-time ticks；旧/新binary必须wait/reconcile或明确cancel，不能再次spawn。PID/identity不匹配fail closed；opaque effect无法确认时进入`ReconciliationPending/Indeterminate`。

## 10. 最终删除与完成定义

最终删除：

- `application::agent::AgentRuntime`；
- Executive-owned AgentControl入口、AgentRuntimeRegistry、core RuntimeRegistry、SubAgentRuntime；
- Executive-owned TurnCoordinator/TurnPipeline/TurnService/daemon Turn authority；
- SessionService的Runtime authority混合体、LegacySessionService、legacy SessionStore writer；
- core Session/ContextState/TuiSessionManager；
- Runtime外Agent/Session/Turn mint点；
- 已证明无生产 caller 的 inactive AgentLoader、已证实等价的 capability mirror/conversion、IPC 同名 AgentId；仍在生产使用的 Markdown profile loader 必须精确改名并迁给 Runtime profile owner；
- global EventSpine/EventBus authority式public append；
- 只服务已删实现的tests/re-exports/features。

保留但换边界：Native Cognit -> CognitiveRun；Pi -> DelegateBackend；Session/Turn历史 -> RuntimeJournal；AgentRun recovery/mailbox/settlement -> AgentStream；context/compaction -> ContextWorkingSet/ContextAssembler；client reconnect -> Gateway projection；plugin registration -> composition adapter；现有Native/Pi用户能力与数据。

完成不等于“新建Runtime API”，而是同时证明：Runtime唯一拥有Agent/AgentRun/Session/Turn；一个Turn状态机和terminal fence；一个Session/child constructor；一个registry；每stream一个writer；Runtime外不能正常mint核心identity；Kernel无Agent语义，TUI/Gateway无Runtime truth；Pi共享统一settlement/recovery且安装态等价；Executive对应实现和seam已实际删除；restart/cancel/timeout/late evidence无false success；rollback/reconcile已用正式安装态演练。

在这些条件全部成立前，不能宣称 Executive 已拆除，也不能通过关闭 Pi 或删除旧能力制造“架构完成”。

# Agent Kernel V2：DeepSeek 重构执行手册

> 状态：Implementation Guide
>
> 基线：`dev@bd1ceac2965832f15cf45b811fd34e052e7e9a67`
>
> 计划审阅检查点：PR [#192](https://github.com/Aurobear/aletheon/pull/192)
>
> 上位计划：[`2026-08-08-agent-kernel-v2-complete-rearchitecture.md`](./2026-08-08-agent-kernel-v2-complete-rearchitecture.md)
>
> 规范索引：[`2026-08-08-executive-decomposition-plan-series.md`](./2026-08-08-executive-decomposition-plan-series.md)
>
> 文件级任务包：[`2026-08-09-agent-kernel-v2-deepseek-implementation-plan.md`](./2026-08-09-agent-kernel-v2-deepseek-implementation-plan.md)

## 1. 目的与使用边界

本手册把已经审阅的架构决议转成可交给 DeepSeek 逐 PR 执行的工程协议。它不替代专项计划，也不授权一次性重写仓库。每次执行只领取一个实际 slice，先关闭证据，再实现，再切换，再清理。

DeepSeek 可以：

- 读取当前 `dev`、计划、账本、Cargo 依赖和已安装态证据；
- 在一个短分支中完成一个 bounded slice；
- 新增 owner-local type、port、adapter、migration、gate 和最小验证；
- 在 writer cutover 后删除该 slice 已证明 caller-zero 的旧实现；
- 发现计划与源码事实不一致时提交 plans-only 修正。

DeepSeek 不可以：

- 把整份 `executive`、`TurnPipeline`、`AgentControlService` 或 `RequestHandler` 搬到新 crate；
- 同时切换两个 authority writer，或在一个 PR 中迁多个无关扩展；
- 在不知道最后 caller、schema reader/writer、installed data 的情况下删除代码或表；
- 用 feature、re-export、service bag 或 compatibility facade 永久隐藏反向依赖；
- 为让测试通过而放宽 deny、伪造 receipt、吞掉 provider/terminal 错误；
- 把本手册中的候选路径当作源码事实，必须以领取 slice 时的最新 `dev` 重做 scoped census。

## 2. 事实源优先级

发生冲突时按以下顺序处理；低优先级不能覆盖高优先级：

1. 当前 checkout 中可复现的生产 caller、schema、配置和 installed-runtime 事实；
2. 上位计划中的 authority、依赖方向和硬安全不变量；
3. 专项计划的 slice、writer cutover、回滚和 deletion gate；
4. Executive/Fabric/Interact 逐文件账本；
5. 本执行手册的流程和模板；
6. 旧注释、旧测试、旧模块名和历史架构文档。

若第 1 项否定第 2 或第 3 项，不得自行“灵活实现”。先提交 plans-only PR，写出新证据、受影响 owner、DAG、兼容和回滚变化，计划通过后再改生产代码。

## 3. 不可妥协的目标边界

### 3.1 唯一权威

- Runtime 唯一构造并推进 `Session`、`Turn`、`AgentRun`、delegate binding；
- Kernel 唯一 admission、验证授权 evidence、调用受治理副作用并签发 receipt；
- Dasein 唯一提交 Self mutation；Metacog 只观察、评估、提出和实验；
- Application 拥有 Approval aggregate 与 canonical GoalDraft，不拥有第二套 Turn/Agent scheduler；
- Gateway 只做认证、协议适配、command/query/event projection；
- TUI、CLI、ACP 只持 presentation/local correlation，不 mint core identity；
- 每个 aggregate 同时只能有一个 production writer、一个 generation 和一个 recovery owner。

### 3.2 依赖方向

```text
Presentation -> Gateway protocol/client -> Application/Runtime ports
Application  -> owner ports
Runtime      -> Kernel/domain ports
Kernel       -> sealed descriptors + OS/effect adapter ports
Adapters     -> owner ports
aletheon     -> 唯一 composition root
```

明确禁止：

```text
Runtime -> SQLite/HTTP/Unix socket/systemd/TUI/Gmail/Robot/Hardware
Application -> raw SQLite/filesystem/process/concrete Pi or Gmail
Gateway -> concrete Kernel/Dasein/Mnemosyne/Executive service
Domain -> Gateway/TUI/Application workflow/Kernel concrete adapter
Kernel -> Session/prompt/Self/Provider/Robot business semantics
Interact -> Executive/Runtime repository/Kernel/domain concrete adapter
```

### 3.3 物理形态

- V2 保留独立、小型 `kernel` crate；本轮不并入 `runtime::execution`；
- `fabric` 先归还 rich type，再由唯一 `AK2-25` 机械改名为 `contracts`；
- `contracts` 不是 ABI，不存 rich aggregate、repository、service bag、业务 workflow；
- `aletheon` 是唯一 concrete composition root，extension 内不重建 composition root；
- Executive 最终由 `XRET-05` 删除，不能被改名成新的中央层。

### 3.4 保留与冻结

- Dasein/Self、Metacog 保持启用；
- Gmail/Google、GBrain、Hardware、Robot VLA、Pi 是保留迁移，不是顺手删除；
- 旧 skill/robot 应用扩张、多平台产品面和非核心 workflow 冻结；
- Robot/Hardware 真实执行在 HIL/实机证据前继续 fail closed/unsupported；
- optional Application package 必须有 production caller 或 installed-data 证据，否则先 `INVESTIGATE`，不在本轮重建。

## 4. 一个工作单元的状态机

每个实际 slice 使用以下状态，PR 描述必须标当前状态和下一状态：

```text
PLANNED
  -> EVIDENCE_OPEN
  -> EVIDENCE_CLOSED
  -> IMPLEMENTING
  -> SHADOW_OR_COMPAT
  -> WRITER_CUTOVER
  -> OLD_CALLER_ZERO
  -> CLEANUP_READY
  -> DONE
```

状态含义：

| 状态 | 必须满足的事实 |
|---|---|
| `PLANNED` | slice 在 canonical crosswalk 中有唯一 owner 和直接前置 |
| `EVIDENCE_OPEN` | caller/schema/config/installed census 尚有未知项 |
| `EVIDENCE_CLOSED` | 未知项为零，或每项都有明确 `INVESTIGATE` blocker 和后续 owner |
| `IMPLEMENTING` | 只增加新 owner path/port/adapter/gate，不改变 authoritative writer |
| `SHADOW_OR_COMPAT` | 只读 shadow 或单向 facade 已启用，无双执行/双 terminal/双写 |
| `WRITER_CUTOVER` | generation/watermark 固化，新 writer 接管，旧 writer 被机械拒绝 |
| `OLD_CALLER_ZERO` | production、installed、feature/config、migration reader caller 均已核零 |
| `CLEANUP_READY` | rollback 不再要求恢复旧 writer，只需兼容新事实的上一 binary |
| `DONE` | 旧 symbol/file/schema writer 删除，gate 和账本同步，安装态验收完成 |

禁止从 `EVIDENCE_OPEN` 直接进入 `IMPLEMENTING`，也禁止在同一提交中先切 writer 再补 census。

## 5. 每次领取任务前的上下文装载

DeepSeek 每次只装载与当前 slice 直接相关的上下文，避免被 12 万行 Executive 带回目录搬家思维。

### 5.1 必读内容

1. `AGENTS.md`；
2. 上位计划 §1.1、§6、§7、§10、§13、§23、§25；
3. 专项索引中的 canonical crosswalk 和 DAG；
4. 当前 slice 所属专项的现状、K/M/D 表、PR 行、验收和回滚；
5. Executive/Fabric/Interact 账本中与本 slice 精确匹配的行；
6. 当前源码的 declaration、production caller、composition、schema、config 和 tests；
7. 直接前置 PR 的最终报告，不只读 diff 标题。

### 5.2 上下文回执

开始修改前必须在工作记录中输出：

```text
Slice:
Baseline commit:
Direct prerequisites:
Current authoritative writer:
Target owner/writer:
IDs minted here:
Production callers:
Installed/config callers:
Tables/files/wire schemas:
External side effects:
Compatibility seam:
Deletion owner:
Unknowns/blockers:
Expected files:
Out-of-scope files:
```

只写“已阅读计划”不算回执。每个字段都要给 symbol/path、计数或明确 `none with command evidence`。

## 6. Phase 0 证据产物

所有实现 wave 前先完成相应 census。Census PR 只允许清单、静态 gate、无行为 rename 或删除已证明完全无 caller 的幽灵权威。

| Slice | 必交证据 | 关闭条件 |
|---|---|---|
| `RA-00` | Session/Turn/Agent constructor、ID mint、writer、journal、registry、backend caller | 每个 mint/writer 唯一分类；legacy/Pi caller 有计数 |
| `K0` | admission/invoke/process/operation/time/resource/effect path | 每个真实副作用有当前执行者、授权输入、receipt/recovery 路径 |
| `D0` | Fabric public file/symbol、domain owner、codec、reader/writer、re-export | 174/174 文件与 public surface 可机械核对；B2/B4 unknown 关闭后才进 D1 |
| `APX-00` | Application use case、SQLite/FS/process、optional package caller | basic/optional/dead 分类完成；I/O 有目标 adapter |
| `CGP-00` | route/protocol/socket/composition/TUI authority/ACP framing | Interact 56/56、Gateway 文件、Executive routes 全覆盖 |
| `E0` | Gmail/GBrain/Hardware/Robot/Pi manifest、installed config/data/behavior | 每项有 preserve/delete/investigate 决议和 equivalence oracle |
| `XRET-00` | Executive public surface、compat seam、最后 caller、deadline | 每个 source-level COMPAT 与 seam ledger 一一对应 |

证据命令优先使用 `rg`、`rg --files`、`cargo metadata` 的窄查询和 schema/config 搜索。测试中的引用必须与生产引用分开计数；public re-export 不是生产 caller。

## 7. 标准实现 PR 生命周期

### 7.1 PR-A：定义 owner seam，不切流量

允许：

- owner-local command/event/query/value type；
- 窄 port；
- typed error category；
- dependency/size/symbol-zero gate；
- adapter skeleton 和显式 composition handle。

不允许：

- 新 writer 与旧 writer 同时写；
- adapter 自行 mint canonical ID；
- port 接收 `ExecutiveConfig`、`HandlerPorts`、`TurnServices` 等 service bag；
- owner crate 为复用旧代码而反向依赖 Executive。

### 7.2 PR-B：适配旧路径与只读 shadow

- legacy facade 必须单向调用新 owner；
- shadow 只能 read/replay/compare，不 append、不 spawn、不执行 effect；
- 差异必须是 typed mismatch，不能只写日志后继续；
- 每个 compat branch 有计数器、最后 caller、deadline 和 deletion PR；
- 数据 backfill 可重复、可中断、可从 watermark 恢复。

### 7.3 PR-C：writer/executor cutover

切换前冻结：

- active aggregate/child/effect 数；
- old/new generation；
- journal/schema watermark；
- pending approval、outbox、receipt、reconciliation 集合；
- installed binary/config/socket identity。

切换动作必须原子表达：

1. maintenance 或 bounded drain；
2. 等待 terminal 或明确 cancel；
3. 固化 generation/watermark；
4. 新 writer/executor 接管；
5. 旧入口返回 typed retired/wrong-generation error；
6. 重放只读验证和 installed smoke；
7. 流量恢复。

不能通过两个 owner 都写、之后再“以新库为准”完成迁移。

### 7.4 PR-D：caller drain 与物理删除

- `rg`、Cargo graph、route/config、installed-data/migration reader 全部为零；
- compatibility counter 在约定观测窗内为零；
- 旧 type/file/re-export/test support 删除；
- inventory、architecture gate、账本和文档同步；
- 删除范围只属于本 slice 的唯一 deletion owner；
- Executive 空壳只能由 `XRET-05` 删除。

## 8. Canonical 实现波次

以下是调度波次，不是授权把一整波塞进同一 PR。一个实际 PR 仍只做一条 writer/adapter 路径。

### Wave A：证据与幽灵权威清理

- `RA-00`、`K0`、`D0`、`APX-00`、`CGP-00`、`E0`、`XRET-00`；
- 删除已证实零 workspace caller 的 `TuiSessionManager`、旧 `AgentRuntime`、第二 `AgentLoader`；
- 建立 monotonic dependency/symbol/authority gates；
- 不切 canonical writer，不改外部行为。

### Wave B：最小 contracts、owner ports 与 composition skeleton

- `RA-01`、`D1`、`K1/K2`、`APX-01`、`CGP-01/02`、`E1`；
- owner-local ID source、commands/events/queries；
- 小型 Kernel descriptor/effect port；
- Gateway protocol/client 与 `aletheon` composition skeleton；
- 禁止提前机械 rename Fabric。

### Wave C：Runtime 与领域 authority

- `RA-02/03/04/05` 严格按 journal -> Session -> Turn -> Agent 顺序；
- `D2/D3/D4/D5` 按 Cognit、Self/Metacog、workspace/memory、capability ownership 拆分；
- `K3/K4/K5` 建立 evidence verifier、单一 invoke/receipt/recovery 和 Linux/execd adapter；
- `APX-02/03/04` 拆 Approval、GoalDraft、SQLite/FS/process；
- writer 切换必须是独立 PR。

### Wave D：保留能力联合切换

- Gmail `E2-K6a`；
- GBrain `E3`；
- Hardware `E4-K6b`；
- Robot `E5-K6c`；
- Pi `E6-K6d`；
- 每项使用 E0 manifest 的 installed equivalence；
- K6 只提供 enforcement seam，E-series 是唯一 extension writer/executor cutover owner。

### Wave E：Presentation/host 与 caller drain

- `CGP-03/04/05/06/07/08`；
- `APX-05`、`RA-06`、`K7`、`XRET-01/02/03` 按对应 file family caller-zero；
- ACP 保留外层 ACP framing/correlation，GatewayClient 只拥有内层 Gateway wire；
- TUI 只剩 model/controller/renderer/GatewayClient。

### Wave F：rich surface、rename 与 Executive 退休

```text
matching extension caller-zero -> E7 extension Fabric rows
E7 -> D6 non-extension rows + shared root closeout
D6 -> AK2-25 one mechanical fabric -> contracts rename
AK2-25 + K7 + RA-06 + APX-05 + CGP-08 -> XRET-04
XRET-04 -> XRET-05 delete empty Executive crate
```

`D6` 与 `E7` 不并行改同一 Fabric root；`AK2-25` 不混类型移动或行为变化；`CGP-08` 不抢 `XRET-04` 的物理删除所有权。

## 9. Slice 快速执行表

此表只用于选任务。详细动作、验收和回滚仍以对应专项为准。

| Slice | 唯一交付 | 主要前置 | 明确不得做 |
|---|---|---|---|
| `RA-00` | constructor/ID/writer census | baseline | 改 writer |
| `RA-01` | Runtime commands/events/queries/ID owner | RA-00/D1 | 定义 UiOverlayId |
| `RA-02` | durable RuntimeJournal + read-only shadow | RA-01 | 把 notification bus 当 journal |
| `RA-03` | SessionAuthority + ContextWorkingSet | RA-02 | 保留第二 Session writer |
| `RA-04` | canonical Turn reducer/terminal | RA-03 | 搬 TurnPipeline God Object |
| `RA-05` | AgentSupervisor + DelegateBackend registry | RA-04 | 迁 Pi concrete 文件 |
| `RA-06` | 删除 Runtime-owned Executive authorities | E6 + matching caller-zero | 删除 crate/host tree |
| `K0` | effect/process/operation census | baseline | 推测业务语义 |
| `K1` | Operation authority/durable seam | K0/D1 | 理解 Session |
| `K2` | sealed descriptors/CapabilityExecutor | K1 | 保存 rich Tool schema |
| `K3` | authorization evidence verifier | K2 | 重新做 Approval workflow |
| `K4` | single admit/invoke/receipt/recovery | K3 + Runtime binding | 双执行 |
| `K5` | Linux/execd process adapter | K4 + CGP-01 | 在 Kernel core 解析业务 config |
| `K6` | extension enforcement seams/gates | K5 + E0/E1 | 代替 E-series 切扩展 |
| `K7` | caller-zero cleanup/surface contraction | matching RA/APX/CGP/E | 并入 Runtime |
| `D0` | Fabric symbol/codec/caller ledger | baseline | 预判未知 owner |
| `D1` | ownerless primitives/contracts seed | D0 unknown=0 | 搬 rich types |
| `D2` | Cognit/InferencePort/provider adapter split | D1 | 第二 Runtime workflow |
| `D3` | Dasein/Metacog authority split | D1 | 关闭 Self/Metacog |
| `D4` | Agora/Mnemosyne authority split | D1 | 让 Memory 写 core authority |
| `D5` | Corpus catalog/executor split | D1 + K2 | 让 Kernel理解 tool 名 |
| `D6` | non-extension Fabric/domain cleanup | D2-D5 + E7 | 抢 E7 extension rows |
| `APX-00` | basic/optional/I/O census | baseline | 迁旧 Application 整包 |
| `APX-01` | minimal Application + ports | APX-00 + Runtime seams | concrete adapter |
| `APX-02` | Approval aggregate/store port | APX-01 | Kernel human workflow |
| `APX-03` | GoalDraft/external stimulus | APX-01 | autonomous worker/scheduler |
| `APX-04` | SQLite/FS/process/host adapters | APX-01 | I/O 留 Application core |
| `APX-05` | Application caller drain | APX-02/03/04 + matching owners | 删除其他 owner 文件 |
| `CGP-00` | route/composition/Interact census | baseline | 改协议行为 |
| `CGP-01` | unique aletheon composition skeleton | CGP-00 | service bag |
| `CGP-02` | Gateway protocol/client | CGP-00 | ACP 外层 framing |
| `CGP-03` | typed route handlers | CGP-01/02 | 业务 aggregate 进 Gateway |
| `CGP-04` | official daemon/socket cutover | CGP-03 | 隐式 migrate |
| `CGP-05` | ACP typed-client cutover | CGP-04 | ACP mint Session |
| `CGP-06` | TUI/CLI command-only cutover | CGP-04 | 本地推导 effective policy |
| `CGP-07` | split TUI App/presentation compat | CGP-06 | Runtime state 进 reducer |
| `CGP-08` | host/composition deletion-ready evidence | CGP-05/07 + matching E | 删除整个旧 tree |
| `E0` | preservation/equivalence manifests | baseline | 假定 caller/data 为零 |
| `E1` | extension ports/registration descriptors | E0 + owner ports | extension composition root |
| `E2-K6a` | Gmail/Google writer/effect cutover | E1/APX/K6 | Gmail 写 canonical GoalDraft |
| `E3` | GBrain supplemental-memory adapter | E1/D4 | 反向依赖 Application |
| `E4-K6b` | Hardware core/bridge cutover | E1/K6 | 无证据开放真实执行 |
| `E5-K6c` | Robot VLA episode/evaluation cutover | D2/E4/K6 | Metacog 进入 hot-path |
| `E6-K6d` | Pi DelegateBackend cutover | RA-05/E1/K6 | raw process bypass Kernel |
| `E7` | extension-specific Fabric rows/re-exports cleanup | matching caller-zero/XRET-03 | 删 Executive concrete bootstrap |
| `XRET-00` | seam ledger/freeze | baseline | 删除行为 |
| `XRET-01` | external Executive facade caller-zero | owner ports ready | 新 facade |
| `XRET-02` | application/core business remnants cleanup | owner cutovers | 抢 RA/D/APX deletion |
| `XRET-03` | concrete adapter/bootstrap drain | APX/D/K/CGP/E cutovers | 删 Fabric rows |
| `XRET-04` | host/composition/compat remnants delete | AK2-25 + all matching gates | 提前于 rename |
| `XRET-05` | empty Executive crate mechanical delete | XRET-04 | 夹带行为改动 |

## 10. 数据、journal 与 schema 迁移协议

每个涉及 SQLite、JSON、filesystem state、wire schema 的 slice 必须列：

- authoritative table/file/stream；
- current writer 与 generation；
- production、installed 和 migration readers；
- key/ID/ordering/terminal/idempotency 不变量；
- backfill watermark 与校验摘要；
- 新旧 binary 的 read compatibility；
- downgrade/forward-fix 条件；
- schema owner 和最终删除 PR。

迁移规则：

1. schema 创建和版本迁移只进显式 `aletheon migrate`，`open()` 不得隐式建表；
2. 可以双版本读，不可以两个 authority 同时写同一 aggregate；
3. notification/projection/cache 可重建，不能反向覆盖 journal；
4. unknown event/version 必须 fail closed，不能忽略后继续推进 terminal；
5. writer cutover 后，回滚 binary 必须能读新事实且仍调用唯一 writer；
6. 无兼容 binary 时保持 maintenance 并前向修复，不能恢复旧双写；
7. 删除表前要证明 installed data、backup/restore、migration reader 和旧 binary 均不再需要。

## 11. 外部副作用与设备安全

副作用包括进程、文件变更、git apply、网络发送、Gmail send、Pi child、Hardware/Robot action。

每条副作用必须经过：

```text
owner command
  -> Kernel sealed descriptor
  -> verified authority evidence
  -> CapabilityExecutor/ProcessController
  -> durable receipt
  -> owner settlement/reconciliation
```

要求：

- `OperationId`、generation、idempotency key 在执行前持久化；
- timeout/cancel/crash 后先 reconcile receipt，不盲重放；
- `success` 只有 authoritative terminal/receipt 能给出；
- Application 不直接 spawn，Pi/Robot/Hardware adapter 不绕 Kernel；
- 无 human authority resolver 时，高风险动作返回 typed `AuthorityUnavailable`，可取消且有 deadline；
- 静态预授权的只读路径可继续，高风险路径 fail closed；
- Robot evaluation 使用 fresh observation 和独立 Robot-owned validator；
- Hardware safety receipt/veto 不得由 Robot/Cognit/Metacog 覆盖。

## 12. 兼容 seam 协议

一个临时 seam 必须同时具备：

| 字段 | 要求 |
|---|---|
| source path/symbol | 精确到 public symbol |
| direction | 只允许 old -> new 或 new reader -> old data |
| last callers | 生产/installed/test 分开计数 |
| observability | 命中计数、错误类别、generation |
| deadline | 日期或 canonical deletion PR |
| rollback role | 说明是否仍被 rollback binary 需要 |
| zero gate | 静态 + installed observation window |
| deletion owner | 唯一 RA/D/APX/CGP/E/XRET slice |

以下不是合法 seam：双向同步、两个 registry 同时可选、旧路径 fallback-on-any-error、永不到期的 re-export、无计数的 alias、在 adapter 中重新 mint ID。

## 13. 命名与代码规范

- 创建权用 `create_session`、`start_turn`、`spawn_agent_run`；只有 owner 可以暴露该动词；
- 客户端用 `request_*`、`submit_*`、`project_*`，不伪装 authority；
- `Service` 只用于 Application facade；领域/Runtime 端口使用职责名或 `*Port`；
- child/Pi 使用 `DelegateBackend`，LLM 使用 `InferencePort`/adapter，受治理副作用使用 `CapabilityExecutor`；
- Native 认知循环使用 `CognitiveRun`，禁止裸 `Executor`；
- 禁止新增泛化 `Core`、`Manager`、`HandlerPorts`、`RuntimeCore`、`TurnServices`、`DomainPorts`、`*Group` getter bag；
- 新 production 普通文件目标 `<=500` 行、`mod.rs <=200` 行、普通函数 `<=60` 行、状态机 transition `<=100` 行、constructor 显式依赖 `<=5`；超过时必须拆分或登记有期限的 Architecture Exception；
- 不新增 crate 级 blanket allow 掩盖 `too_many_arguments`、`module_inception`、`new_without_default`；
- 内部错误链：`AdapterError -> owning Domain/Kernel/Runtime typed error`；公开链：`OwnerError -> ApplicationError -> GatewayErrorCode -> Presentation`；
- 禁止按错误字符串识别 policy denial、terminal、provider failure；
- `UiOverlayId` 只在 Interact 本地，不能序列化为 canonical ID。

## 14. 验证策略：少而关键

本重构不迁整套旧测试，也不以覆盖率作为完成标准。每个 slice 只保留能证明边界或恢复的最小证据。

### 14.1 每个 PR 都运行

- `git diff --check`；
- 受影响 crate 的 `bash scripts/cargo-agent.sh check -p <crate>`；
- `bash scripts/cargo-agent.sh fmt --all -- --check`；
- architecture fitness gate；
- 新增/退休 symbol、dependency、caller 的机械计数；
- 与 slice 对应的最小 invariant/replay/recovery smoke。

### 14.2 writer cutover PR 额外运行

- crash/restart/replay；
- timeout/cancel/late terminal；
- wrong generation/duplicate delivery；
- idempotency/reconciliation；
- migration backfill/rollback drill；
- 官方安装态二进制、socket、daemon 和真实请求验收。

### 14.3 保留能力额外运行

- Gmail/GBrain/Pi 使用 installed config 的无破坏 smoke；
- Robot/Hardware 在 simulator/replay 证明 contract，真实执行仍 gated；
- Native/Pi 必须分别验收，不能用其中一条代替另一条；
- provider error 必须渲染为失败，不能用 prompt 返回掩盖；
- async child 必须 wait 到 authoritative terminal。

旧测试分类只允许：`KEEP-EVIDENCE`、`REPLACE`、`DELETE-LATER`。不得默认把 Executive 的大测试目录整体搬到新 owner。

## 15. 交给 DeepSeek 的标准任务模板

### 15.1 Evidence-only 任务

```text
执行 <SLICE> 的 evidence-only PR。
基线为最新 dev，先读 AGENTS.md、总计划、索引、所属专项和相关账本。
只产出 caller/constructor/writer/schema/config/installed census 与 monotonic gates；
不得改变 production behavior、writer 或 schema。
对每个 unknown 标出 owner、阻塞、关闭命令和后续 canonical slice。
最终按本手册 §17 报告，不要执行下一个 slice。
```

### 15.2 Owner/port 提取任务

```text
执行 <SLICE> 的 owner/port extraction PR。
先提交上下文回执，证明 direct prerequisites 已满足。
只新增一个 owner-local command/event/query/port 和必要 adapter skeleton；
旧 writer 继续唯一 authoritative，新路径不得产生副作用。
禁止 service bag、concrete adapter 反向依赖和 canonical ID 外部 mint。
通过 scoped check、architecture gate 后停止，不切 writer。
```

### 15.3 Writer cutover 任务

```text
执行 <SLICE> 的 writer/executor cutover PR。
给出当前/目标 writer、generation、watermark、active work、pending receipt、
compat counter、installed config 和可读新 schema 的 rollback binary。
采用 maintenance/drain -> generation freeze -> single cutover -> old path reject
-> replay/installed verification -> resume 的顺序。
任何一步不满足即回到 maintenance，不允许启用双写或双执行。
```

### 15.4 Deletion 任务

```text
执行 <SLICE> 的 caller drain/deletion PR。
先证明 production、installed、config/feature、wire/schema reader、migration 和
compat counter 均为零；列出 exact symbol/files。
只删除属于该 slice 的旧实现、facade、re-export 和旧测试支持；
同步 inventory、fitness gate、source ledger 和 deadline。
不得删除其他 owner 的 tree、Fabric extension rows 或 Executive crate 空壳。
```

## 16. 必须停止并升级计划的条件

出现以下任一情况，DeepSeek 必须停止修改生产代码：

- 同一 aggregate 找到新的 production writer 或 ID mint 点；
- 当前文件职责跨两个 owner，但账本写 `MOVE/MERGE` 而不是 `SPLIT/INVESTIGATE`；
- installed data/config/caller 与 E0/D0/APX-00 census 不一致；
- 计划中的两个 slice 都声称拥有同一 writer、文件或 deletion；
- 迁移需要双写、无界 shadow、fallback-on-any-error 或旧 schema 反向覆盖新 journal；
- 回滚只能恢复旧 authority writer；
- Hardware/Robot 真实安全语义缺少 receipt/freshness/veto 证据；
- 需要 Application/Runtime/Kernel 依赖 concrete extension 才能复用代码；
- 需要在 `contracts` 新增 rich aggregate/service/repository；
- architecture gate 只能通过删除或放宽既有禁止规则；
- scoped compile 因代码错误失败；依赖/环境失败则记录，不伪称验证通过；
- PR 超出一个 authority writer 或一个 adapter family，且无法安全拆分。

升级报告必须包含：新证据、受影响计划段落、owner/DAG/compat/rollback 变化和最小 plans patch。

## 17. 每个 PR 的完成报告

```markdown
## Slice
- ID / state transition / baseline / head

## Authority change
- old owner/writer/generation
- new owner/writer/generation
- ID assignment point

## Evidence
- production callers before/after
- installed/config callers before/after
- tables/files/wire readers/writers
- compatibility counter/deadline

## Change scope
- files added/changed/deleted
- explicitly out of scope

## Safety
- side effects/receipts/reconciliation
- migration/watermark
- rollback binary and procedure

## Validation
- exact commands and results
- architecture/symbol/dependency counts
- installed acceptance or why this PR cannot yet claim it

## Remaining blockers
- next canonical slice only; do not silently continue
```

如果报告不能回答“谁现在能写、谁现在能 mint、崩溃后谁恢复”，该 PR 不得标记完成。

## 18. 建议的首批 DeepSeek 任务

PR #192 合并后，不要直接开始 `TurnPipeline` 大拆分。建议顺序：

1. 将现有 architecture ratchet 固化为 foundation checkpoint，验证没有误删 external public consumer；
2. 执行 `RA-00`，关闭剩余 ID/writer/registry/Agent loader 证据；
3. 并行只做 evidence 的 `K0`、`APX-00`、`CGP-00`、`E0`；
4. 完成 Fabric overlay 的 B2/B4 unknown，只有 D0 真正关闭后开始 `D1`；
5. 先建立 `RA-01/D1/K1/K2/CGP-01` 的窄 contracts/ports；
6. 再按 `RA-02 -> RA-03 -> RA-04 -> RA-05` 逐 writer 推进；
7. Pi 只能在 RA-05 generic registry 稳定后由 `E6-K6d` 联合切换；
8. Executive 删除严格留到对应 owner caller-zero、Fabric rename 和 XRET gates 之后。

每个任务结束即停止，由人审阅完成报告和 diff 后再发下一个任务。不要给 DeepSeek 一条“按全部 plans 完成重构”的长任务。

## 19. 最终完成定义

当且仅当以下全部成立，Agent Kernel V2 架构重构才完成：

- Runtime 是 Session/Turn/AgentRun/delegate identity、journal、reducer、terminal、recovery 的唯一 owner；
- Kernel 是 admission/evidence verification/effect execution/receipt/recovery 的不可绕过小型独立层；
- Dasein、Metacog、Cognit、Agora、Mnemosyne、Corpus 各自只有窄 owner surface；
- Application 可关闭且只留下基本 use case；Gateway/TUI/ACP 不构造核心状态；
- Gmail/GBrain/Hardware/Robot/Pi 的保留清单、数据和安装态行为完成等价切换；
- `contracts` 只含 ownerless primitives/wire-safe references，且从未命名为 ABI；
- Executive、双 writer、双 registry、service bag、旧 compatibility seam、旧 Fabric rich rows 为零；
- repository dependency graph 满足方向，God Object/文件规模/lint gate 不回退；
- crash/replay/cancel/timeout/idempotency/reconciliation 和正式安装态请求均有证据；
- 每个删除动作都能追溯到唯一 owner、caller-zero 证据和 canonical deletion PR。

本手册的价值不是让执行者“更快搬完代码”，而是让每一步都能回答：当前事实在哪里、谁有权改变它、谁实际执行、失败后如何恢复、旧路径何时可以永久删除。

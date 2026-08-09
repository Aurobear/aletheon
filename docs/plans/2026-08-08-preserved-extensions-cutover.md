# 保留扩展能力迁出与切换计划

> 状态：Draft，供独立 plans PR 评审
>
> 基线：`dev@bd1ceac2965832f15cf45b811fd34e052e7e9a67`
>
> 上位计划：`2026-08-08-agent-kernel-v2-complete-rearchitecture.md`
>
> 前置阶段：D1 Contracts primitives、对应领域窄 port、RA-05 `DelegateBackend` seam 与 Kernel process/effect port；不要求先完成整份 Domain/Runtime/Executive 退役计划
>
> 本文只定义能力保全、迁移、切换与回滚，不在 plans PR 中修改运行时代码。

## 1. 目标

Gmail/Google、GBrain、Robot VLA、Hardware 和 Pi 不是这次重构的删除对象。它们当前的问题是实现横跨 Executive、Fabric、Cognit、Corpus、Mnemosyne、Hardware 和 daemon bootstrap，导致核心 Agent 无法独立构造，扩展本身也没有明确 owner。

本计划执行 `preserve-but-move`：

- 保留已有且能够用证据证明的能力；
- 不把测试名称或未验证代码当成“已经支持”；
- 先建立行为、数据和失败语义保存舱，再迁移实现；
- 迁移期间始终只有一个外部副作用 dispatcher 和一个状态 writer；
- 核心 Runtime 不依赖任何具体扩展，扩展可缺省、可降级、可拔除；
- 最终删除 Executive/Fabric 中的扩展实现和兼容入口。

Robot skill marketplace、新 robot 型号、新平台适配继续冻结。冻结的是扩张，不是现有 Robot VLA、Hardware simulation/bridge、安全链和验收证据。

## 2. 不可违反的边界

### 2.1 Contracts 不是扩展仓库

极小共享 crate 最终名称是 `contracts`，明确不叫 `abi`。以下类型禁止进入 `contracts`：

- Gmail message、cursor、history、OAuth/account、send/report 类型；
- Google Calendar/Drive delta 和 provider DTO；
- GBrain page、spool、remote ack/reconcile 类型；
- Robot proposal、failure、episode、evaluation、VLA state；
- device、lease、safety、watchdog、bridge command/receipt；
- Pi manifest、RPC record、child process/protocol state。

跨域绑定只使用极小 primitives，例如 `AgentId`、`SessionId`、`TurnId`、`ActionBindingId`、`ActionDigest`、`CorrelationId`、`IdempotencyKey`。ID 只做关联，不代表授权或成功。

### 2.2 Core 必须在没有扩展时成立

目标依赖：

```text
runtime     -> core domains only
application -> runtime
gateway     -> application

gmail       -> gateway/application + kernel ports
robot-vla   -> cognit/runtime extension ports + hardware
hardware    -> contracts + kernel executor/receipt ports
pi          -> runtime DelegateBackend + kernel process/lease ports
gbrain      -> mnemosyne supplemental port + kernel resource port

aletheon composition root -> selects and wires installed adapters
```

禁止 Runtime import Gmail、Google SDK、Robot、Hardware、Pi、GBrain、gRPC、OAuth 或扩展配置。禁止扩展创建第二个 Agent、Session、Turn、Self 或 Memory authority。

### 2.3 三类 I/O 不混用

| I/O 类别 | 本计划中的实例 | 路径 |
|---|---|---|
| Core service I/O | Gmail inbound polling/sync、GBrain supplemental reconcile、provider worker heartbeat | 专属 port、lease、budget、outbox/reconcile；不伪装成普通 tool |
| Agent-requested effect | Gmail send/reply、Robot command、由 Pi 启动的 delegated process | Kernel descriptor → permit/lease → dispatch → finalized receipt |
| Owner persistence/control | extension cursor/store、Hardware safety store、migration、Bridge 本地 safe-stop | 各 owner repository/control port；不经过普通 CapabilityBroker |

外部 effect 的“请求被接受”“执行完成”“目标达成”是不同事实，必须使用不同 receipt，不能由模型文本代替。

## 3. 当前实现与目标 owner

| 能力 | 当前分布 | 目标 owner |
|---|---|---|
| Gmail ingress/classify/goal/report/send policy | `executive/src/adapters/channel/gmail/*`、handler、Application Goal/Approval | `extensions/gmail`；公共交互经 Gateway/Application |
| Google OAuth/API/sync | `corpus/src/tools/google/*`、`executive/src/adapters/google/*`、`adapters/external/google_use_cases.rs`、daemon bootstrap | Google concrete adapter；Gmail-specific state 仍归 Gmail extension |
| GBrain | `mnemosyne/src/backends/supplemental/*` 与 `executive/src/adapters/gbrain/*` | `adapters/gbrain` 实现 Mnemosyne 的 supplemental port |
| Robot cognition/VLA | `cognit/src/harness/robot/*`、Executive robot harness/perception/audit/promotion/composition；另对 `mnemosyne/src/embodied_episode.rs` 与 `metacog/src/evaluation/outcome.rs` 做 caller/data/equivalence census | `extensions/robot-vla`；后两项先 INVESTIGATE，禁止无证据整迁 |
| Robot/embodiment rich types | `fabric/src/types/embodiment.rs`、episode/report/failure/audit/world-state 等 | Hardware 或 Robot VLA，按事实 owner 拆分 |
| Device safety/bridge/simulation | `crates/hardware` 加 Executive embodiment service/bootstrap | Hardware core + gRPC/bridge adapter；composition root 组装 |
| Pi delegate | `executive/src/adapters/runtime/pi.rs`、`pi_protocol.rs`、`pi_rpc.rs` 与 RequestHandler 构造 | `adapters/pi` 实现 Runtime `DelegateBackend` |

Executive 中的旧模块不能整体改名成 extension。每条路径必须拆出 domain state、concrete transport、Application use case、Kernel effect 和 composition glue。

## 4. 迁移前保存舱

E0 PR 必须为每个扩展生成一份 versioned preservation manifest，至少记录：

- 当前配置键、默认值、环境变量、secret source 与 redaction 规则；
- public command/RPC/tool surface；
- 输入输出 schema、error code 和 degraded behavior；
- database path、table、schema version、writer process、migration owner；
- worker lease、cursor、dedupe、retry/backoff、idempotency 和 reconciliation；
- 外部 side effect 的 dispatch boundary 与 terminal receipt；
- cancel/restart/reconnect/recovery 语义；
- 已验证支持等级、未验证等级和明确不支持项；
- 最小可重复 smoke 场景及所需真实环境；
- 回滚 binary/schema 兼容范围。

Robot manifest 还必须逐一登记两项容易漏掉的重复实现：`mnemosyne/src/embodied_episode.rs` 自建的 `episode_attempts` SQLite schema 及 `expected_outcome_json/verification_json` reader/writer，以及 `metacog/src/evaluation/outcome.rs` 的 public deterministic outcome verifier。当前静态 census 只证明两者存在并有测试引用，未证明 production constructor/caller；E0 必须补 installed config/data-file/caller 与行为差异证据。caller/data 均为零则给出 exact deletion PR；存在安装态数据或唯一行为才进入 E5 migration/equivalence，不能因目录名推定 owner。

现有测试用于发现预期场景，不自动成为真实支持证据：

| 扩展 | 可复用的发现入口 | 保存舱必须补充的真实证据 |
|---|---|---|
| Gmail/Google | `corpus/tests/google_read_only.rs`、`gmail_history_sync.rs`、`google_delta_sync.rs`；Executive Gmail/Google tests | installed runtime 下真实 account 的只读 sync/cursor/restart；只有确实启用 write 时才验证 send/reconcile |
| GBrain | Mnemosyne `gbrain_backend_contract`、`gbrain_spool`、`gbrain_reconciliation`；Executive bootstrap/worker tests | remote unavailable/recover、spool replay、ack 去重、本地权威不被覆盖 |
| Robot | Cognit robot harness tests；Executive robot session/policy/bridge/audit tests；Mnemosyne episode schema/Metacog outcome verifier 只作发现入口 | Simulation 可重复链；Bridge handshake/lease/cancel；另给出 duplicate store/verifier 的 installed caller、数据文件、schema 与行为等价结论；HIL 只有真实证据才声明 |
| Hardware | Hardware emergency-stop/deployment-gate/gRPC tests；Executive hardware simulation | simulator deterministic smoke；Bridge timeout/lease/watchdog/safe-stop receipt |
| Pi | Executive `pi_runtime`、`pi_rpc_runtime`、`pi_real_contract` | 真实 child 的 spawn/wait/terminal/reconnect/cancel，key 不进 argv，未终态不成功 |

这不是要求保存所有旧测试。迁移完成后只保留能证明产品能力、数据兼容和边界不可绕过的窄场景。

## 5. Gmail 与 Google

### 5.1 目标拆分

`extensions/gmail` 拥有：

- inbound message normalization、source identity、trust boundary；
- Gmail cursor/history/dedupe/quarantine；
- classifier、ingest policy、Goal Draft、report/send policy；
- Gmail-specific command/event/error 与 store port；
- send/reply descriptor、idempotency key、reconcile 与 receipt mapping。

Google concrete adapter 拥有：

- OAuth、credential vault、account resolution；
- HTTP client 与 Gmail/Calendar/Drive provider DTO；
- provider pagination/delta/history protocol；
- remote retry/backoff、rate limit 与 provider receipt。

Gateway/Application 拥有：

- `IngestExternalStimulus`、`CreateGoalDraft`、Approval presentation/resolution；
- authenticated public RPC DTO/error；
- channel subscription/presentation。

Kernel 拥有 outbound send/reply 的 execution permit、dispatch fence 与 finalized capability receipt。Gmail inbound read/sync 使用专属有限期 sync lease，不通过通用 Tool executor。

### 5.2 当前代码迁移

- `executive/src/adapters/channel/gmail/*` → Gmail extension，去掉对 RequestHandler/daemon 类型依赖；
- `executive/src/adapters/google/{store,sync_manager,event_dispatcher}.rs` → 按状态 owner 分到 Gmail store 或 Google adapter；
- `executive/src/adapters/external/google_use_cases.rs` → Gateway/Application port adapter；
- `corpus/src/tools/google/*` → rich descriptor 留 Corpus，Google HTTP/OAuth 实现进 adapter；
- `host/daemon/bootstrap/google.rs`、`integrations.rs`、`channels.rs` → 唯一 `aletheon` composition root；
- `rpc_google.rs` → Gateway handler，不包含 OAuth、SQL 或 sync manager 构造。

### 5.3 数据与安全

- 邮件正文/附件始终标记 ExternalUntrusted，不能进入 system/owner instruction；
- credential/refresh token 不进入 Runtime journal、Contracts、模型上下文或普通日志；
- cursor、dedupe、quarantine、message metadata 只有 Gmail store 一个 writer；
- Goal Draft 与 activated Goal 是不同 aggregate，邮件不能自行激活；
- read-only 保持默认，write descriptor 显式启用；
- send/reply 必须绑定 account、recipient set、content digest、approval evidence、idempotency key；
- dispatch 后结果不明进入 reconciliation，禁止直接重试造成重复发送。

### 5.4 切换门与回滚

- 先以新 adapter shadow-read 同一 remote page，并比较 normalized envelope/cursor；
- shadow 阶段不重复发送、不推进第二个 cursor；
- 暂停旧 worker，确认 lease 失效，再把唯一 writer 切到新 store/service；
- additive schema migration 后同时保留旧 binary 可读范围；
- 如需回滚，先停新 worker、完成 in-flight reconcile，再恢复旧 writer；
- 未能证明 send terminal state 时回滚仍保留 `ReconciliationPending`，不能把它变成 success/failed 猜测。

## 6. GBrain

### 6.1 目标边界

Mnemosyne 定义 `SupplementalMemoryPort`、local memory provenance 与 merge/admission 规则；`adapters/gbrain` 只实现 remote page、outbox/spool、ack 和 reconciliation。

GBrain 永远不是第二个 Memory Authority：

- local accepted `MemoryRecord` 是 Agent 使用的正式记忆；
- remote result 只能作为有来源、可过期、可拒绝的 supplemental evidence；
- remote 不得覆盖 Runtime terminal、Dasein Self revision、Kernel/Hardware receipt；
- unavailable 时本地写入继续，明确标记 supplemental degraded；
- reconcile 成功不能回写或改写已 settled Turn。

### 6.2 当前代码迁移

- `mnemosyne/src/backends/supplemental/{backend,config,migrations,page,reconcile,spool}.rs` 中的通用 port/model 留 Mnemosyne，GBrain wire/store/reconcile 实现迁 adapter；
- `executive/src/adapters/gbrain/{bootstrap,mcp_adapter,worker}.rs` 迁入 GBrain adapter/composition，不再依赖 Executive；
- `mnemosyne::CompositeService` 只组合 local + supplemental port，不认识 MCP/HTTP/config path；
- daemon bootstrap 只负责读取 validated config、提供 secrets、启动 supervised worker。

### 6.3 切换门与回滚

- 复制或 version-migrate spool 前先记录 row count、digest、ack position；
- 只允许一个 outbox writer 和一个 reconcile lease holder；
- 新 adapter 先 shadow-read remote，不 ack、不删除旧 spool；
- 切换后验证 crash/restart 不丢 pending item、重复 ack 幂等、remote unavailable 可恢复；
- 回滚停止新 worker后恢复旧 lease，保留新格式 spool 的兼容 reader；
- schema 不兼容时 fail closed 并保留 backup，禁止 startup 隐式重建空库。

## 7. Hardware

### 7.1 Hardware 是独立安全域

Hardware 保留为核心外的独立 device/safety domain，不降格为普通 Tool，也不并入 Robot VLA。它拥有：

- device identity/manifest/state revision；
- typed command validation；
- control lease、command sequence、deadline/nonce；
- Hardware SafetyPermit；
- watchdog、heartbeat、emergency-stop/safe-stop；
- observation freshness；
- command accepted、execution、safe-stop receipt。

`crates/hardware` 当前直接依赖 Fabric 且携带 `tonic`。目标是 Hardware core 只依赖 Contracts 和所需 Kernel port；`grpc/provider` 迁到独立 bridge adapter。纯 deterministic simulator 可以保留为 Hardware-owned test/reference adapter，但不能让 production core 默认拉入 gRPC。

Fabric 中 device/lease/safety/embodiment 类型回 Hardware；Robot proposal/episode/evaluation 类型不进入 Hardware。

### 7.2 三重 gate

物理动作必须同时满足：

```text
Dasein/Owner semantic verdict
+ Kernel ExecutionPermit
+ Hardware SafetyPermit
= Bridge 可执行
```

三者绑定同一个 `ActionBindingId + ActionDigest`。Hardware permit 还绑定 `DeviceId + DeviceStateRevision + lease + deadline + nonce`，在 Bridge 执行边界再次验证。Hardware veto 永远优先，Kernel permit 不能覆盖它。

本地 safe-stop/emergency stop 不等待 Runtime、Dasein 或 Kernel；上游断连、heartbeat 丢失、lease 过期必须在 Bridge 本地进入安全状态。

### 7.3 支持等级与切换门

| 等级 | 本次承诺 |
|---|---|
| Simulation | 保留，作为默认 deterministic safety smoke |
| Bridge | 保留现有 gRPC bridge handshake/lease/cancel/watchdog 路径 |
| Lab/HIL | 仅有真实设备、部署 gate、receipt evidence 时声明 |
| Production actuator | 当前明确 `UNSUPPORTED` |

先将 gRPC adapter 接到不改变行为的新 Hardware port，再运行 Simulation/Bridge shadow observation；命令不能 dual-dispatch。切换唯一 dispatcher 前必须完成 lease handoff，旧 bridge session 被显式 revoke。回滚同样先 safe-stop、settle/标记所有 in-flight command，再恢复旧 adapter，不能让两个 controller 同时持 lease。

## 8. Robot VLA

### 8.1 目标 owner

`extensions/robot-vla` 拥有：

- Robot-specific cognition strategy/profile；
- typed proposal、proposal validator、perception cache/policy；
- attempt/episode/failure/audit/evaluation；
- independent outcome validator；
- Robot configuration 中不属于 device safety 的部分。

迁移来源：

- `cognit/src/harness/robot/*`；
- Executive `robot_harness_composition.rs`、`robot_perception.rs`、`robot_audit.rs`、`robot_episode_promotion.rs`；
- Executive `application/deterministic_outcome_verifier.rs` 与 episode SQLite sink：生产 caller/writer census 后提取 Robot-owned validator/store adapter；
- `mnemosyne/src/embodied_episode.rs`：先 INVESTIGATE installed constructor 与既有 DB/data；若为零则删除重复 repository，若存在数据则把 `episode_attempts` 及 JSON reader/writer 版本化迁到 Robot-owned store adapter，并由 D4 清掉 Mnemosyne owner 越界；
- `metacog/src/evaluation/outcome.rs`：先 census public/test-only caller 与规则差异；只把经证明确实唯一的 deterministic validator 语义合并进 Robot validator，否则删除重复实现，Metacog 核心只消费 post-settlement evidence；
- Executive embodiment workflow 中 Robot-specific orchestration；
- Fabric `robot_*`、episode/report、proposal/evaluation 类型。

Executive `host/daemon/bootstrap/robot.rs` 只留下应迁往唯一 composition root 的 wiring；Hardware-owned类型和规则不复制进 Robot extension。

### 8.2 目标控制链

```text
Runtime canonical Turn
-> Robot CognitiveRun strategy
-> typed Robot ActionProposal
-> deterministic proposal validation
-> Dasein/Owner review
-> Kernel permit
-> Hardware safety permit
-> bounded Bridge execution
-> CommandAcceptedReceipt
-> HardwareExecutionReceipt
-> fresh observation
-> independent RobotEvaluationReceipt
-> episode/goal settlement
```

VLA 不能直接调用 driver，不能验证自己提出的动作，不能用 command accepted 代替 execution complete，也不能用 execution complete 代替 goal achieved。cancel 必须传播到 Hardware safe-stop。

### 8.3 切换门与回滚

- 先迁纯 proposal/validator，并对同一 recorded observation 做 shadow evaluation；
- shadow 路径不能向 Hardware dispatch；
- episode store 采用单写，迁移时保留 attempt/receipt/action digest；
- `expected_outcome_json`、`verification_json` 与 `episode_attempts` 必须有 schema/version/checksum、last reader/writer、备份、双读窗口和回滚证据；D4 只迁 Mnemosyne consumer，不保留第二 Robot episode writer；
- Executive 与 Metacog deterministic verifier 先做相同 observation 的差异 corpus；E5 只保留一个 Robot-owned independent validator，零 production caller 的重复实现直接删除；
- General Turn 在 Robot extension 未安装、Bridge 离线或 capability disabled 时仍可用；
- Robot Turn 在 Hardware safety/bridge 不可用时 fail closed；
- 切换后至少证明 Simulation 的 proposal→permit→receipt→evaluation 完整链和 Bridge 的 cancel/safe-stop；
- 回滚前 safe-stop、关闭新 dispatcher、完成/隔离 in-flight episode，再恢复旧路径；
- 新技能、新设备、新平台 PR 在本计划完成前仍拒绝进入。

## 9. Pi

### 9.1 目标边界

`adapters/pi` 实现 Runtime-owned `DelegateBackend`。Pi 是受监督 external delegate，不是第二个 Runtime、Session、Self 或 Kernel。

必须保留：

- manifest/capability selection；
- prompt/follow-up/steer/abort/get-state protocol；
- spawn、wait、terminal receipt；
- reconnect/recovery；
- workspace isolation；
- process-group cancellation；
- mediated 与 delegated-sandbox 两种可见治理等级；
- key 不进入 argv；
- child 未到 authoritative terminal snapshot 时不能报告成功。

### 9.2 当前重复与迁移

当前 `pi.rs` 实现 `SubAgentRuntime`，`pi_rpc.rs` 又实现 `AgentRuntimeLauncher`，两者都包含准备、选择、启动或协议职责；`pi_rpc.rs` 还直接 `spawn`、设置 `kill_on_drop` 和向 process group 发信号。目标只保留一个 Pi adapter 状态机：

- `pi_protocol.rs` → Pi-owned codec/schema；
- manifest/selection → Pi adapter 对 `DelegateBackend` capability 的实现；
- OS spawn、process group、sandbox、deadline、kill → Kernel `ProcessController`/lease；
- protocol event → Runtime child evidence/terminal receipt；
- RequestHandler 中 Pi 构造与 registry registration → 唯一 composition root；
- E6 在 caller 迁完后删除/改名 Pi-owned `PiRuntime/PiRpcRuntime` 与其 legacy launcher implementation，目标只剩 `PiDelegateBackend`；generic `SubAgentRuntime/SubAgentExecutionContext` 继续作为只读单向 facade，并唯一由 RA-06 在所有直接/间接 caller 清零后删除。

Pi 的 delegated sandbox 是进程级治理，不得虚假宣称 Pi 内部每个 opaque effect 都逐项受 Kernel 拦截；实际治理等级必须出现在 projection/receipt 中。

### 9.3 切换门与回滚

- resident `PiDelegateBackend` 完成 installed equivalence 前，Pi-owned legacy `PiRuntime/PiRpcRuntime` 与 generic `SubAgentRuntime` 只能组成单向 compatibility path；不得提前删除，也不得与 resident 路径同时执行同一个 delegate。E6 切换后删除 Pi-owned legacy implementation，generic facade 留给 RA-06；
- 新旧协议 codec 可用 recorded stream 做 shadow parse，但只能一个进程 owner；
- child spawn 必须来自 Kernel-issued process lease，Pi adapter 不直接创建未治理进程；
- cancel 后验证整个 process group 终止并得到 terminal/cancel receipt；
- daemon/adapter 重启后，要么重连受支持 child，要么明确 settle orphan/indeterminate；
- 回滚先拒绝新 delegate、settle/cancel active child，再切 launcher；
- 旧 binary 必须理解新 terminal/provenance projection，或回滚前完成 drain。

## 10. 跨扩展切换协议

每个扩展都按同一顺序：

```text
1. freeze new surface
2. capture preservation manifest
3. define owner port and rich types in owner
4. add concrete adapter without changing production writer/dispatcher
5. migrate schema additively
6. shadow-read / shadow-evaluate only
7. stop old worker and fence old lease/epoch
8. switch the single writer/dispatcher
9. observe installed runtime and external receipts
10. remove old caller/fallback/re-export in a later PR
```

临时 feature/config flag 只用于一次切换与紧急回退，不能作为永久分层手段。切换稳定后删除旧 flag 和旧 fallback。

任何外部副作用禁止 shadow-dispatch。若 effect 结果不确定，必须进入带 owner、deadline、next-action 的 `ReconciliationPending`，到期后 terminal `Indeterminate`，不能猜测成功或安全重试。

## 11. PR 序列与依赖

### E0：Preservation manifests

- 五类扩展的行为、schema、secret、worker、receipt、degraded/rollback 清单；
- 固化当前真实支持等级；
- 冻结新 Robot skill/平台和新的 Executive/Fabric 扩展 surface。

### E1：Owner ports 与 composition seam

- 在 Runtime/Mnemosyne/Kernel/Hardware/Application owner 定义窄 port；
- 建立 extension registration input；
- 核心 binary 在零扩展配置下可构造；
- 不把 rich types 放进 Contracts。

### E2：Gmail/Google 迁出

- Gmail domain/store 与 Google concrete adapter 分离；
- OAuth/HTTP/SQL/worker 从 Executive/Corpus bootstrap 迁出；
- read-only sync 先切，write/send 在独立后续 PR 切；
- 删除 Executive Gmail/Google production callers。

### E3：GBrain 迁出

- D4 只提供 Mnemosyne supplemental-memory port；本阶段是 GBrain 文件、worker、lease、spool writer 与生产 caller 切换的唯一 owner；
- Mnemosyne supplemental port 稳定；
- spool/reconcile/worker 迁 adapter；
- 单写/lease 切换与 degraded/recovery smoke；
- 删除 Executive GBrain bootstrap/adapters。

### E4：Hardware core/bridge 分离

- Fabric device/safety 类型回 Hardware；
- gRPC bridge 从 Hardware core 默认依赖面迁出；
- 三重 gate binding 与 lease handoff；
- Simulation、Bridge 切换证据。

### E5：Robot VLA 迁出

- D2 只提供 `CognitiveRun` port，E4 只提供 Hardware port；本阶段是 Robot harness/perception/episode/evaluation 文件迁移和行为切换的唯一 owner；
- 依赖 Cognit `CognitiveRun` 与 E4 Hardware port；
- Robot harness/perception/episode/evaluation 从 Cognit/Executive/Fabric 收敛；同时结案 `mnemosyne/src/embodied_episode.rs`（DELETE duplicate 或 B4 数据迁移后二选一）与 `metacog/src/evaluation/outcome.rs`（MERGE unique semantics 或 caller-zero DELETE 二选一），不得留下第二 store/verifier；
- 条件依赖固定：E0 若证明 Mnemosyne episode caller/data 为零，E5 只执行 exact DELETE；若非零，则 `D4` owner seam 与 `APX-04` SQLite migration/rollback readiness 必须先于 E5-K6c 的 Robot episode writer cutover。Metacog verifier 的 E0 结论只决定 MERGE unique semantics 或 DELETE，不另建 writer/dependency stage；
- General 无扩展路径、Simulation、Bridge cancel/safe-stop 验收；
- 删除 Executive Robot/embodiment workflow。

### E6：Pi 迁出

- RA-05 只建立 generic `DelegateBackend`/registry 与 legacy seam；本阶段是 Pi adapter、协议、active-child drain/reconcile 和 writer/caller 切换的唯一 owner；Runtime 最终清壳阶段只在 E6 验收后删除 generic legacy seam；
- 单一 `DelegateBackend`；
- Kernel process lease/controller 接管 spawn/cancel；
- 协议、terminal、reconnect/recovery 切换；
- installed equivalence、active child drain/reconcile 和 Pi-owned caller 切换完成后，E6 才删除/改名 legacy `PiRuntime/PiRpcRuntime`、legacy launcher implementation 与 Pi-specific Executive facade；generic `SubAgentRuntime/SubAgentExecutionContext` 不在 E6 删除，只产 caller-zero evidence并交给 RA-06。删除前保持单向兼容，禁止双执行。

### E7：兼容面退役

前置不是“扩展 cutover 完成”这一项笼统条件。每条将删除的 extension row 都必须同时满足：本扩展 authoritative cutover 与 installed equivalence 已通过、`XRET-03` 已完成相应 concrete adapter drain，并且仍引用该 row 的 Runtime/Application/Kernel/host family 已分别交付对应 `RA-06/APX-05/K7/CGP-08` caller-zero evidence。只等待实际引用该 row 的 family，禁止把无关 family 串成全局互锁。

- 唯一删除 Gmail/GBrain/Hardware/Robot/Pi 等 extension-specific Fabric rich types/re-export；不得同时清理非扩展 domain/runtime/kernel/application rows；
- 从 Fabric root re-export 中只移除 extension-owned entries并产出 root diff/evidence；完成后把根级最终收口权交给 D6，E7 不做 crate rename；
- Executive concrete extension modules、bootstrap/config/fallback 与旧 schema writer 必须已由 matching E2..E6 owner cutover 和 `XRET-03` 唯一删除；E7 只验证这些路径为零，不重复拥有物理删除；
- 只删除逐行登记在 Fabric extension surface 内的 post-cutover 单向 flags/parser/re-export；其他 parser 或 persistence writer 仍归其 concrete owner/XRET gate；
- dependency gate 证明 Runtime/Application core 不依赖具体扩展。

E2、E3、E4、E6 在 E0/E1 后可并行；E5 依赖 Cognit extraction 与 E4；E7 在上述逐 row caller-zero gate 全绿后最后执行。每个 E 编号是独立实现 PR 系列，不合成 Mega PR。

## 12. 验收矩阵

| 条件 | Gmail/Google | GBrain | Hardware | Robot VLA | Pi |
|---|---|---|---|---|---|
| 无扩展时 core 可启动 | 必须 | 必须 | 必须 | 必须 | 必须 |
| 单 writer/dispatcher | cursor/store/send | spool/ack | command/lease | episode/dispatch | child owner |
| degraded/offline 明确 | sync unavailable | supplemental unavailable | safety fail closed | General 可用，Robot fail closed | delegate unavailable |
| restart/recovery | cursor/dedupe/reconcile | spool replay | lease/watchdog | episode/in-flight | reconnect/orphan |
| cancel/safe-stop | send pre-dispatch cancel | worker stop | local safe-stop | Hardware safe-stop | process group cancel |
| 权威 terminal evidence | provider + Kernel receipt | remote ack 仅 supplemental | Hardware execution receipt | independent evaluation receipt | child terminal snapshot |
| secret 不泄露 | OAuth/token | GBrain credential | bridge credential | inherited minimum only | key 不进 argv |

实现 PR 使用最窄的结构检查、contract smoke 和真实 installed runtime 场景；所有 Cargo 命令经 `bash scripts/cargo-agent.sh ...`。Plans-only PR 不运行部署验收。

对 Gmail send、真实 Bridge/HIL、真实 Pi 和 GBrain remote 的支持声明，必须基于安装后运行证据；mock、临时 daemon 或直接 provider 调用只能作为诊断。

## 13. 回滚总则

- 回滚前先停止 admission，不再接收新 effect；
- fence 新 worker/lease/epoch，drain、cancel 或明确标记所有 in-flight；
- 保存 dispatch journal、idempotency key、receipt 与 reconciliation 状态；
- 只恢复一个旧 writer/dispatcher；
- 使用明确兼容当前 schema 的旧 binary；
- 任何 physical action 回滚先 safe-stop；
- 任何 mail/send 结果不明保留 reconcile，禁止重复发送；
- 任何 GBrain remote 差异不能覆盖 local authority；
- 任何 Pi orphan 不能被当作成功；
- 回滚不恢复已经被 architecture gate 禁止的第二个 Agent/Session/Memory authority。

## 14. 完成定义

本计划完成时：

- Gmail/Google、GBrain、Robot VLA、Hardware、Pi 的已验证能力各有 preservation manifest 与目标 owner；
- 它们不再由 Executive 构造或实现；
- 扩展 rich types 不在 Fabric/Contracts；
- 核心 Runtime 在全部扩展未安装时仍可编译、构造和运行；
- 每个扩展只有一个 store writer、worker lease holder 和 effect dispatcher；
- Gmail 外发、Robot/Hardware command、Pi process 都经过对应治理和 terminal receipt；
- GBrain 仍只是 Mnemosyne supplemental adapter；
- Hardware 仍保有独立最终安全否决权；
- Robot 扩张仍冻结，但现有 VLA、Simulation/Bridge 和安全链没有因重构丢失；
- 临时兼容、旧 Executive caller、Fabric re-export 和切换 flag 全部删除；
- 回滚路径已用真实 schema/worker/receipt 证据验证，而不是只写在文档里。

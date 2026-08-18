# Grok 审核：Aletheon `wiring` 所有权迁移计划

状态：**REVISE — 被审计划不可按原文实施；本文已按二次核查修订；未授权 Rust 代码修改**

日期：2026-08-16

二次核查：2026-08-17

审核对象：`docs/plans/2026-08-16-aletheon-wiring-ownership-migration.md`

核查基线：

- HEAD：`85704ed3f641419f86dd08c5be8d26c52c64c19b`
- 被审计划 SHA-256：`9e78bc2064eebd650ab9911e0831939f90b63dd65c9af13696e56ed0de361369`
- 除本文外 dirty path：35；path-list SHA-256：`c701342c50e86d1d1a7e01613989a445143efc8b2280eb8f92fd62deeacd82bb`
- 除本文外 tracked diff SHA-256：`f35d49b1e6dd57e50cd7b14a5d9e2017f87438d442a404079ae9aec9632bee12`

该基线包含未提交和未跟踪文件，不能只从 HEAD 复现。本文凡写“当前工作区”均指上述
二次核查快照；后续 Codex 审核必须重新读取 manifest、被审计划和 dirty path，不能把
本文的依赖判断当成长期事实。

审核格式：计划 §10

用途：交给 Codex / 计划作者修订。本文是审核意见，不是实施授权。

---

## 0. 给后续分析者的阅读顺序

1. 先读被审计划全文，尤其 §2 原则、§3 owner matrix、§4 依赖、M2/M5/M6、§9。
2. 再读本文 §1 总裁决和 §2 blocking。这五条不闭合，不得进入实施。
3. §3 是对计划 §10 十问的逐项回答，每条都带 `path:line`。
4. 修订计划时不要另起一套架构叙事；在原计划上改依赖规则、owner matrix、Turn 切法和完成门禁。

本文所有代码主张均在审核当日对照过仓库。修订前请重新打开被引文件，不要只信本文记忆。

---

## 1. 总裁决（§10 格式）

```text
Verdict: REVISE

Blocking findings:
1. [high] §2 A4 vs §4.1 — A4 允许 application 依赖 domain crate，§4.1 写成只许
   contracts+runtime。不能把 domain crate 按直接依赖列入白名单：HEAD 上已经存在
   application→mnemosyne→platform→application；当前工作区又存在
   application→dasein→platform→application，并传递影响 cognit/metacog。
   §4.1 必须按完整传递图区分“当前允许”和“完成前置解耦后才允许”。
2. [high] §3.2 / M6 — 把 TurnPipeline 整包迁入 application::turn 会形成
   Executive 2.0；但把步骤散到 cognit/runtime/ports 又会让用例顺序无人负责。
   application 必须拥有 provider-neutral TurnService 的用例顺序；daemon_react
   必须拆成 cognit session/ReAct 与外部副作用 adapter，不能放进 composition。
3. [high] §3.3 — owner matrix 漏 governed_review、workspace_trust、core_rpc、
   doctor、exec、user_runtime、exec_session、embodiment、evolution、readiness、
   cognitive_runtime。governed_review 持久化不能给 aletheon host；core_rpc 必须
   明确为 adapters-inference 的 Unix transport，host 只负责生命周期。未补全前
   不得写 M9 删除 wiring。
4. [high] M2.1 / M3.1 — Goal artifact FS（goal/mod.rs:65，
   goal/verification.rs:157-215）和 GoalWorker 的 gateway/platform 依赖未切。
   worker 整文件进 application 会成环；工件 I/O 必须 port 化。
5. [high] §9 — 完成定义不能阻止 application/composition/adapters-sqlite
   成为新 god module。补依赖、authority、副作用和行为硬门；名称与 LOC 只能作为
   review trigger。runtime::orchestration 必须经符号/行为审计后退役，不能默认并入 Agora。

Non-blocking findings:
1. [med] §3.4 adapters-runtime 改名并移走 worktree；Google SQLite 必须单 owner。
2. [med] 已存在 runtime::ids vs contracts 两套 SessionId/TurnId 表示，canonical
   identity、mint 权限与转换契约未闭合；
   application goal_* vs wiring goal 双模块；Approval 多入口。
3. [med] admin_service 不是单一 feature，禁止整包迁成 application::admin。
4. [med] A8 敏感度（MemorySensitivityV1）和 transient approval cache
   未落入 M2/M4/M10。
5. [med] M1 方向对；cognitive_role_workflow/{stages,state_machine}.rs
   确为幽灵文件（父模块已是 agora re-export）。
6. [low] M0 账本/gate 先做是对的；hotspot-budgets.tsv 仍把 turn_pipeline
   标成 aletheon-turn。
7. [low] 基线 LOC 与当前工作区一致，但 dirty 基线不能只从 HEAD 复现；冻结值必须
   同时记录统计命令和 diff/path-list digest。

Dependency graph verdict:
  目标图只有在每个 packet 都按完整传递依赖图验算时才可证明无环。当前 application
  可继续依赖 contracts/runtime；kernel 是本次列举中当前可证明无回边的 domain。
  mnemosyne 经 platform 回到 application；当前 dirty worktree 中 dasein 也经 platform
  回到 application，并传递影响 cognit/metacog。agora/corpus/platform/gateway 与
  GoalWorker 的具体类型仍只能通过 consumer-owned port 隔离。

Authority uniqueness verdict:
  Session/Turn/Agent 归 runtime、Memory 归 mnemosyne、Capability 归 kernel
  成立。未成立的是：canonical ID/转换契约、多 Approval 入口、三条编排器、
  Google 双路径，以及 matrix 外模块在 M9 的默认归属。两套 ID 表示本身不自动等于
  双 authority，但当前双向构造说明边界没有闭合。GenerationFence 已在 runtime
  settlement_engine，wiring 未使用，M5 必须单路径切过去。

Functional modularity verdict:
  §3.5 的 command/query/service/port 清单可作为审查清单。TurnPipeline、
  admin_service、GoalWorker 按当前切法不满足内聚。Turn 仍需由 application 的小型
  provider-neutral service 拥有用例顺序；composition 只构造对象。功能切片应先拆
  闭包，再搬目录。

Operational acceptance verdict:
  M10 的 digest / restart / doctor / 真实 LLM / 真实 GBrain / 三分计数
  足以做安装态验收。不足以覆盖 M5/M6：缺 principal_context fail-closed、
  durable-write failure、sensitivity、transient approval、robot target、
  以及 include_str 测试升级清单。
```

---

## 2. Blocking 详述与要求修正

### B1. 依赖规则自相矛盾，按原文会成环

计划 A4 图把 `kernel/domain crates` 画成可以指向 `application`。§4.1 表格却写：

> `application` | `contracts`、`runtime`；必要算法库，不依赖 adapter、HTTP、SQLite

当前 `crates/application/Cargo.toml:13-14` 确实只有 `contracts` + `runtime`。但 `TurnPipeline` 不是这个依赖闭包能装下的：

```81:111:crates/aletheon/src/wiring/application/turn_pipeline.rs
pub struct TurnPipeline {
    pub agora: Option<Arc<dyn AgoraService>>,
    pub kernel: Arc<KernelRuntime>,
    pub context_assembler: Arc<...ContextAssembler>,
    pub cognitive_sessions: Arc<dyn ...CognitiveSessionFactory>,
    pub memory_gateway: Arc<...MemoryGatewayService>,
}
```

同时已经存在 **反向或传递回边**，不能只看目标 crate 是否直接依赖 `application`：

| 候选依赖 | 回到 `application` 的路径 | 二次核查证据 | 当前裁决 |
|---|---|---|---|
| `kernel` | 未发现 | `crates/kernel/Cargo.toml:9-12` | 当前可允许，但只暴露稳定 domain API |
| `mnemosyne` | `mnemosyne -> platform -> application` | `crates/mnemosyne/Cargo.toml:15`；`crates/platform/Cargo.toml:9` | 当前禁止 |
| `dasein` | `dasein -> platform -> application` | 当前 dirty `crates/dasein/Cargo.toml:13`；`crates/platform/Cargo.toml:9` | 当前工作区禁止 |
| `cognit` | `cognit -> dasein -> platform -> application` | `crates/cognit/Cargo.toml:10`；同上 | 当前工作区禁止 |
| `metacog` | `metacog -> dasein/cognit -> ... -> application` | `crates/metacog/Cargo.toml:10-11`；同上 | 当前工作区禁止 |
| `agora` | `agora -> application` | `crates/agora/Cargo.toml:11` | 禁止 |
| `corpus` | `corpus -> application` | `crates/corpus/Cargo.toml:12` | 禁止 |
| `platform` | `platform -> application` | `crates/platform/Cargo.toml:9` | 禁止 |

`gateway` 今天只依赖 `contracts`（`crates/gateway/Cargo.toml:17`）。计划 §4.1 又允许 gateway 依赖 application-facing port。若 `application` 再引用 `gateway::ports::GoalProgress`，会变成第三条环。

`goal/worker.rs:9-11` 已经同时用了：

- `gateway::ports::{GoalProgress, GoalProgressKind}`
- `kernel::chronos::SystemClock`
- `platform::storage_quota::StorageQuota`

**spec-vs-code 冲突：**

| 审核稿原主张 | 当前代码现实 | 一致？ |
|---|---|---|
| `kernel/cognit/dasein/mnemosyne/metacog` 均“当前无环” | `mnemosyne` 在 HEAD 上已经通过 `platform` 回到 `application`；dirty worktree 中 `dasein/cognit/metacog` 也有传递回边 | 否 |
| domain crate 不直接依赖 application 即可进入白名单 | Cargo 环取决于完整传递闭包，不是直接边 | 否 |
| GoalProgress 可在迁移后继续由 application 引用 | gateway 若转向 application port，会与该反向引用成环 | 否 |

**要求修正：** 把 §4.1 改成带前置条件的显式表，并在每个 packet 重新生成完整
传递依赖图。不能把“当前无环”写成跨 milestone 的永久事实。

当前直接允许：

- `contracts`
- `runtime`
- `kernel`（仅限当前 manifest 已验证的稳定 domain API）
- 优先复用已在 contracts 的类型：`OperationRequest`、`LlmProvider`、`GoalId`、`MemorySensitivityV1`

当前禁止直接依赖：

- `agora`、`corpus`、`platform`、`gateway`
- `mnemosyne`，以及当前 dirty worktree 中的 `dasein`、`cognit`、`metacog`
- 任何 `adapters-*`
- `aletheon`

`dasein/cognit/mnemosyne/metacog` 只能在先消除全部传递回边、再通过 architecture gate
验证后转入“允许”；不能由本计划预先承诺。Agora / Corpus / FS / channel 必须表现为
application 侧 port，由具体 adapter 实现，`aletheon::composition` 只注入实现。Goal
进度映射留在 transport/composition adapter，不要把 `GoalProgress` 拉进 application。

### B2. TurnPipeline 整包进入 `application` = Executive 2.0

计划禁止新建 `orchestration` crate，这是对的。但 M6 把整个 pipeline 树写进 `application::turn`，只是换了个家。

`TurnPipeline` 当前是混合了用例顺序、transport 和具体 domain/adapter 的跨域编排器，
因此不能作为一个文件整体迁移。它直接依赖：

- `agora::contract::*`（`turn_pipeline.rs:17-19`）
- `cognit::CanonicalTurnEventSink`（`:20`）
- `corpus::hook::*`（`:21`）
- `dasein::{Intent, IntentSource}`（`:22`）
- `gateway::protocol::legacy_progress::ClientEvent`（`:23`）
- `kernel::{operation::OperationScope, KernelRuntime}`（`:24-25`）

旧 hardening 计划把 pipeline 派给 `runtime`（`docs/plans/2026-08-15-architecture-hardening.md:99`）也不对。Runtime 不应拥有跨 Cognit/Corpus 的 use-case orchestration。

`daemon_react.rs:13-29` 同时依赖 host-owned `CognitiveRuntimeConfig`、cognit stream 和
application session input；`daemon_react.rs:201-209` 还直接调用 Corpus 工件物化。
因此它也不能整文件归 cognit，更不能进入 composition。M6 来源表列了它，目标树却没有
按责任拆分后的 owner。

原计划 §3.5 已规定 `service.rs` 负责“用例顺序”，composition 只选择实现和生命周期
（`docs/plans/2026-08-16-aletheon-wiring-ownership-migration.md:343-367`）。所以修订不能
从“禁止整包进入 application”跳成“application 不拥有 pipeline 顺序”；否则实际顺序
只能重新泄漏到 cognit、runtime 或 composition。

**要求修正：** 禁止新 orchestration crate，也禁止整包搬迁；保留一个窄的、
provider-neutral 的 application Turn 用例协调器：

```text
application::turn
  command / outcome / service / ports
  唯一入口、授权、provider-neutral 用例顺序、cancel/deadline 关联、terminal mapping

cognit
  cognitive session / ReAct / cognitive stream；不持有 host config 或 Corpus 副作用

runtime::turn_*
  canonical ID / reducer / writer / recovery / generation fence

adapter implementations
  agora/dasein workspace/verdict
  corpus tool/hook/artifact
  mnemosyne recall/projection
  daemon notification transport

aletheon::composition
  只构造和注入，不出现 Goal/Turn/Agent 状态分支，也不实现 daemon_react 逻辑
```

现有 `daemon_react` 必须拆成 cognit-owned session/ReAct 核心与 adapter-owned config、
tool/artifact/event 投影；不能在目标矩阵里写“cognit 或 composition”。

`TurnEngineContext.notification_sender: Option<mpsc::Sender<String>>`（`turn_engine.rs:78-79`）是 transport，不得进入 `application` 公共类型。

### B3. Owner matrix 不完整，M9 删除 wiring 会无人认领

计划 §1.1 的 wiring 总量是准的（审核当日实测）：

| 路径 | 文件 | LOC |
|---|---:|---:|
| `crates/aletheon/src/wiring/` | 215 | 78148 |
| `wiring/application/` | 69 | 29329 |
| `wiring/daemon/` | 65 | 22724 |
| `wiring/adapters/` | 45 | 16846 |
| `wiring/composition/` | 12 | 1781 |
| `crates/application/src/` | 30 | 6315 |

§3.3 矩阵覆盖了 Session / Turn / Agent / Goal / Approval / Verification / Memory / Inference / Google / Daemon RPC / Extensions。未覆盖、删除 `wiring` 时会掉进 composition 的模块：

| 路径 | LOC | 建议在修订稿中补的 owner |
|---|---:|---|
| `wiring/governed_review/` | 1085 | `application::governed_review` use case + consumer-owned repository port；具体文件持久化进 platform 或独立 filesystem adapter；composition 只注入 |
| `wiring/workspace_trust.rs` | 943 | `application::workspace_trust` 决策/port；platform workspace-evidence adapter 负责受限 FS/Git metadata；host 只传 workspace root/client mode |
| `wiring/core_rpc/` | 929 | `adapters-inference::core_rpc` 拥有 client/protocol/server transport；`aletheon::host::core` 只负责 socket 路径、配置和生命周期 |
| `wiring/readiness.rs` | 684 | aletheon host |
| `wiring/doctor.rs` | 568 | `aletheon::host::doctor` 收集安装态事实；CLI 层只渲染结果 |
| `wiring/exec.rs` | 566 | `aletheon::host::exec` |
| `wiring/user_runtime.rs` | 566 | `aletheon::host::user_daemon`（M8 已点名，矩阵没有） |
| `wiring/exec_session.rs` | 527 | `aletheon::host::exec` 的 TurnService adapter；不持有 Turn policy |
| `wiring/embodiment/` | 426 | hardware/platform adapter 拥有设备集成；composition 只管理生命周期 |
| `wiring/evolution_coordinator.rs` | 385 | metacog 拥有 mutation/evolution policy；cognit reflection 与 lineage FS 经 typed ports/adapters；现文件拆完删除 |
| `wiring/approval_service.rs` | 228 | `application::approval` use case + repository/apply ports；`adapters-sqlite` 只实现持久化 |
| `wiring/cognitive_runtime.rs` | 175 | 不整包保留：session/ReAct 状态进 cognit，evolution 进 metacog/ports，host config 由 composition 注入；现文件删除 |
| `wiring/mode_router.rs` | 92 | `aletheon::host` 入口模式选择；不得进入 Turn policy |
| `wiring/domain.rs` | 96 | `aletheon::composition::services` 的 crate-private handle aggregate；按 feature builder 拆小，不拥有 policy |
| `wiring/extension.rs` | 14 | `aletheon::host::cli::extension` 的只读 Corpus inspector adapter |

`wiring/daemon/bootstrap/` 单独 11296 行。M8 说它变成 composition builders，可以，但必须算进 composition 预算，不能在删 `wiring` 后假装这些行消失了。

这里必须避免两个新的边界错误：

1. `GovernedReviewStore` 直接管理目录、权限和 authoritative JSON
   （`wiring/governed_review/store.rs:1-12,65-120`）。原计划 §4.2 又禁止
   `aletheon` 定义具体 repository implementation，因此不能写成 “aletheon host persistence”。
2. `core_rpc/client.rs:72-108` 实现 cognit 的 `InferencePort`，`server.rs:59-88`
   是 Unix transport，正好属于计划已经批准的 `adapters-inference` 边界；host 只监督
   server 生命周期，不能再用“host 或 composition”保留两个候选 owner。

**要求修正：** 扩写 §3.3，使 `wiring/` 每个顶层文件/目录都有唯一 target owner，
并将 policy/port、adapter implementation、host lifecycle 分列。表中出现“或”、
“暂留 composition”或未入表时，不得标 M9 complete。

### B4. Goal 工件文件系统和 GoalWorker 未按副作用切开

M2.1 只写 SQLite。代码里 Goal 还有文件权威：

- `wiring/application/goal/mod.rs:65`：`std::fs::create_dir_all(&artifact_dir)`
- `wiring/application/goal/verification.rs:157-215`：失败回滚删除、读工件、hash mismatch fail-closed、atomic rename（`:432`）

这与 M2.2 workspace checkpoint 是同类 fail-closed 文件语义，不能留在 application，也不能在抽 SQL 时丢掉。

M3.1 把 `worker.rs` 放进 `application/src/goal/worker.rs`。该文件导入 gateway / kernel / platform（见 B1）。整文件迁移会迫使 `application` 依赖禁止 crate。

另外，`crates/application/src/` 已经有 `goal_attempt.rs`、`goal_draft.rs`、`goal_frame.rs`、`goal_projection.rs`、`goal_retry.rs`。M2.1 再开 `application/src/goal/` 会变成第三套 Goal 表面（再加上 `contracts::goal`）。

**要求修正：**

- 不需要独立 Goal domain crate。`contracts::goal::{GoalId, GoalState, GoalSnapshot}` 已是值类型；transition/retry 规则放 application；SQL 放 `adapters-sqlite::goal`。
- 工件 I/O 通过 application-owned `GoalArtifactStore` port；platform/filesystem adapter
  实现 fail-closed read/write/hash/rename/remove，metadata 走 `adapters-sqlite::goal`。
- `GoalWorker` 不整文件保留：application service 拥有 transition/retry/单次 advance
  顺序；host scheduler 只提供 interval/cancel 触发；gateway progress adapter 映射输出；
  platform quota adapter 实现 admission port；composition 只注入这些实现。
- 合并现有 `application/src/goal_*.rs`，不要平行模块。

### B5. §9 挡不住新的 god module

§9 能抓住“删了 `wiring`、application 里没有 rusqlite”。抓不住：

1. `application` 涨到约 30K 并持有 `KernelRuntime` / `AgoraService` / `MemoryGateway`
2. `aletheon::composition` 吞下 B3 未分配模块
3. `adapters-sqlite` 或改名失败的 `adapters-runtime` 变成基础设施 god crate
4. `runtime::orchestration` 复活成第二条 Turn 编排

`runtime/src/orchestration.rs` 已发布 `EvidenceDrivenController`（`runtime/src/lib.rs:165-168`），仓内无生产调用方。不能在 runtime 里再长编排。

原计划 A7 已写明“行数只是预警，不是 owner 判据”
（`docs/plans/2026-08-16-aletheon-wiring-ownership-migration.md:206-208`）。因此 LOC
和命名可以触发复核，但不能替代依赖、authority 和行为证明；禁止几个名字也挡不住
换名后的 god module。

**要求修正：** §9 增加语义硬门：

- `application` 生产源禁止：`KernelRuntime`、`AgoraService`、`rusqlite`、`reqwest`、`Command::new`、`std::process`、`tokio::process`
- 每个 packet 用完整 Cargo 传递图证明无环，不能只 grep 直接依赖
- authority census 证明每个 durable fact 只有一个 reducer/write owner，adapter 不 mint authoritative ID
- composition 禁止 Goal/Turn/Agent 状态转换；只允许构造、配置和生命周期管理
- adapter effect census 证明 SQLite/FS/HTTP/process 副作用只在指定 adapter/host 边界
- `adapters-sqlite` 只许 persistence，不许 policy
- `runtime::orchestration` 先做消费者和行为审计；若确无生产消费者则删除，只有证明其语义属于 Agora 后才允许合并，不能按名称搬家
- source-string gate 与 behavior parity 分开计数；删除路径不等于功能等价

以下保留为 review trigger，不单独证明 COMPLETE：

- composition 单文件建议 ≤1000 LOC；application 按 feature 设预算
- 新建名为 `orchestration` / `core` / `misc` / `common` 的 crate 必须阻止并要求 owner 复核

---

## 3. §10 十问逐项回答

### Q1. `application` 只依赖 `contracts + runtime` 是否足够？ — REVISE

不够承载当前 Turn/Goal/Agent 闭包，也**不能**为此去依赖 agora / corpus / platform / gateway。

应增加的是 consumer-owned port，不是 concrete crate。除当前已验证无回边的 `kernel`
外，任何 domain Cargo 依赖都必须先满足 B1 的传递图前置条件，不能在计划里提前白名单。

`TurnConfigPort` 今天返回 `crate::config::CognitiveRuntimeConfig`（`turn_runtime_ports.rs:101-103`）。计划 M4 已要求改成 application-owned `TurnRuntimeSettings`。这是对的，但必须同时改 `turn_coordinator.rs:5` 对 `BackpressureConfig` / `GrokHardeningConfig` 的直接引用。

### Q2. TurnPipeline 进 `application::turn` 还是新 crate？ — REVISE

禁止新 crate。新 `orchestration` / `turn-pipeline` crate 就是 A6 要防的 Executive 2.0。

也禁止整包搬进 `application`。但 application 必须保留 provider-neutral TurnService，
拥有授权后的用例顺序；不能把顺序分散到 ports 或 composition。正确切法见 B2。

### Q3. Runtime 是否只保留 reducer/writer/authority？ — APPROVE

同意。Session/Turn/Agent 终态只在 `runtime`。

`runtime/src/session_service.rs:42-47` 把 `rusqlite::Connection` 和 authority 放在同一对象；M2.0 抽 `SessionProtocolEventStore` 是对的。约束也是对的：`protocol_events` schema / sequence / dedupe / approval reconnect / cursor 不变，禁止双写。

补一条：审计并退役 `runtime::orchestration`，不要把它做成“轻量编排层”。只有在
行为/authority 证据证明其语义属于 Agora 时才允许吸收，不能因为文件名相似就搬迁。

### Q4. §3.4 四个 adapter crate？ — REVISE

| crate | 裁决 | 理由 |
|---|---|---|
| `adapters-inference` | 批准 | 实测 3831 LOC；独立 HTTP/stream/backpressure 边界 |
| `adapters-gbrain` | 批准 | MCP + attestation + `ExternalReference` 降权 |
| `adapters-google` | 有条件批准 | OAuth/channel 独立；SQLite 必须单 owner，不能写“google 或 sqlite” |
| `adapters-runtime` | 拒绝此名 | 与 authority crate `runtime` 撞名，违反 A6；且混入 worktree recovery |

`adapters-runtime` 应改名为 `adapters-agent-backend`（Pi / native Cognit / provider worker）。worktree recovery 进 `platform` 或 `corpus`。

`wiring/core_rpc/{client,protocol,server}.rs` 也进入 `adapters-inference::core_rpc`：client
实现 cognit `InferencePort`，protocol 是该 Unix transport 的私有 wire schema，server
实现 peer policy/frame handling。`aletheon::host::core` 只绑定配置、socket 路径和生命周期。

Session 合入已有 `adapters-sqlite::session`：批准。生产 store 已在那边；`wiring/adapters/session/canonical_store.rs:8-10` 只是 re-export。

`<200` 行类型桥接留 `aletheon::composition`：批准。禁止临时做成 `adapters-all`。

另：`corpus/src/tools/mod.rs:5` 已有 `pub mod google`。M7.3 必须写明：

- Corpus Google = 受治理工具执行
- `adapters-google` = OAuth / sync / projection

否则 Google 双 owner。

`adapters-sqlite` 已依赖 gateway / application / cognit / metacog / mnemosyne / platform（`crates/adapters/sqlite/Cargo.toml:10-18`）。它可以继续收 Goal/Admin/Session SQL，但不能收业务 policy。

依赖图在遵守「adapter → consumer port，不反向」时无环。

### Q5. Goal policy 放 Application 是否 owner 不清？ — APPROVE（附条件）

不需要独立 Goal crate。条件见 B4。

Agent run 仍由 runtime 裁决。`RuntimeGoalAttemptExecutor` 作为 runtime adapter 放到 `adapters-agent-backend`（或现 `adapters-runtime` 的继任名），实现 application 的 `GoalAttemptPort`：同意。

### Q6. Gateway vs Unix server？ — APPROVE

切分正确。

- `gateway/src/lib.rs:1-12`：typed protocol / channel routing，不持有 persistence
- `daemon/protocol.rs`：连接协商，用的是 `contracts::protocol::client`
- Unix listener / systemd 留 `aletheon::host`

约束：

- gateway 只能依赖 application **port**，不能依赖 `GoalWorker` / `TurnPipeline` / `AdminService`
- `legacy_session` 必须有 sunset ledger
- `wiring/core_rpc/`（929 行）是第二条 Unix 协议，§3.3 / M7.1 / M8 必须明确迁到
  `adapters-inference::core_rpc`，host 只保留生命周期接线

### Q7. M5/M6 是否漏恢复/安全边？ — REVISE

原则 A8 写了 identity / cancel / deadline / permit / idempotency / generation fence / terminal receipt / restart recovery / 敏感度。packet 正文没有钉死下列已存在的边：

| 边 | 代码 | 计划 |
|---|---|---|
| `require_principal_context` fail-closed | `turn_engine.rs:88-99` | 未写 |
| `notification_sender` 是 transport | `turn_engine.rs:78-79` | M6 说入口只差 adapter，目标类型仍会把 daemon channel 带进 application |
| `evaluate_cancel` + `MonoDeadline` | `turn_coordinator.rs:6-14` | 只写“保持同一关联”，无符号、无测试名 |
| terminal durable-write failure | `turn_coordinator.rs:34-40`，`runtime::durable_write` | M6 第 4 条有语义，无验证命令 |
| host config 泄漏 | `turn_coordinator.rs:5`；`turn_runtime_ports.rs:101-103` | M4 改了 TurnConfigPort，M6 没改 coordinator |
| `GenerationFence` | `runtime/src/settlement_engine.rs:19,67,189` | `wiring/application` 零引用；M5 必须强制走 runtime engine |
| `compaction_v2` 门控 turn recovery | `runtime/src/turn_recovery.rs:3-9` | 未写 feature flag 恢复是否仍生效 |
| 敏感度 | `contracts/src/protocol/memory.rs:39-59` `MemorySensitivityV1` | 只出现在 A8；M4 / M7 / M10 全无 |
| transient approval cache | `admin_service.rs:295-308` | M2.1 当普通 admin SQL，未保留 principal/thread/tool/expiry |
| `daemon_react` | 见 B2 | M6 来源有、目标无 |
| robot target binding | M6 第 10 条 | M10 行为矩阵无 robot 回归 |

M5 已经写对的部分应保留：

- 禁止复制 `runtime::AgentRunId` / `AgentLifecycleEvent`
- 禁止永久保留 `CompatibilityRuntimeCatalog`（`agent_control/mod.rs:211`）
- 禁止 application 直接选择 Pi/native backend
- 禁止 adapter mint authoritative Agent ID
- 禁止 recovery 同时调用旧/新 settlement

缺的是：`AgentHostAdapter` 必须使用 `runtime::SettlementEngine` 的 fence，不得再写一套；并在验证里断言 wiring 侧零 `GenerationFence` 复本。

### Q8. 是否有 feature 被多 owner 同时裁决？ — REVISE

| 对象 | 现状 | 问题 |
|---|---|---|
| Session/Turn ID | `runtime/src/ids.rs:15-21` **和** `contracts::{SessionId,TurnId}`；coordinator 双向构造 | canonical identity、mint 权限和转换契约未闭合；两套表示本身不自动证明双 authority |
| Approval | `application::approval` + `adapters_sqlite::approval_repository` + `wiring/approval_service.rs` + `admin_service` transient cache | 至少两套 settlement / grant |
| Goal | `application/src/goal_*.rs` + `wiring/application/goal/` + `contracts::goal` | M2.1 再开 `application/src/goal/` 会变成第三套 |
| Agent settlement | runtime engine 已权威；`agent_control/settlement.rs` 现为 re-export + 投影 | 可以，但 M5 必须删除本地 policy |
| Google | `wiring/adapters/google` + `corpus/src/tools/google` | M7.3 未切工具 vs sync |
| 编排 | `TurnPipeline` + `runtime::orchestration` + `agora::cognitive_role_workflow` | 三条控制器 |
| Admin | `admin_service.rs` 混 profile / model / hook / approval / skill / extension / rollback | 不是一个 feature，不能整包迁成 `application::admin` |
| 未入矩阵 | 见 B3 | M9 默认归属会变成 composition god |

`admin_service.rs:295-308` 的 `ScopedApprovalCache` 自行打开 SQLite，建 `transient_session_grants`。这是独立 durable fact，必须有单一 authority，不能藏在“admin 模块”里。

Session/Turn ID 的修订项应是明确且可验证的契约，而不是只写“删除双 ID”：

1. `runtime` 是否是唯一 canonical ID mint authority；
2. `contracts` ID 是 wire representation、domain value，还是也可 mint；
3. `turn_coordinator.rs:636-672,1246-1256` 的 String/UUID 双向构造收敛到一个
   显式、可失败、可审计的转换边界；
4. repository/event/receipt 各自使用哪一种表示；
5. caller/thread alias 不得构造 canonical Session/Turn ID。

### Q9. 哪些测试必须先升级为行为 parity？ — REVISE

这些是 `include_str!` / 源码字符串门禁。移动文件后会假绿或假红，不能当作行为覆盖。
其中仍有价值的结构约束可以改写后保留，但必须单列为 architecture/source gate，不能
计入行为 parity：

| 测试 | 当前锚定 |
|---|---|
| `crates/aletheon/tests/agora_bound_permit.rs` | `turn_pipeline.rs` |
| `crates/aletheon/tests/approval_service.rs` | `rpc_approval.rs` |
| `crates/aletheon/tests/admin_service.rs` | `rpc_admin.rs` |
| `crates/aletheon/tests/agent_admission.rs` | `agent_control/{spawning,mod}.rs` |
| `crates/aletheon/tests/context_assembler.rs` | `turn_pipeline.rs`、`daemon_turn/mod.rs` |
| `crates/aletheon/tests/turn_use_case_ports.rs` | pipeline / coordinator / post_turn |
| `crates/aletheon/tests/session_use_case_port.rs` | `rpc_session.rs` |
| `crates/aletheon/tests/goal_service.rs` | `rpc_goal.rs` |
| `crates/aletheon/tests/governed_review_rpc.rs` | review RPC |
| `crates/aletheon/tests/memory_bifurcation_guard.rs` | cognitive_runtime / handler / domain / request |
| `crates/aletheon/tests/core_user_boundary.rs` | core/user runtime |
| `crates/aletheon/tests/kernel_clock_composition.rs` | core/user/bootstrap |
| `wiring/application/harness_factory.rs:32-46` | 自检 `pub use` 字符串（M1 点名禁止的那类） |

迁移前必须保住、且计划里已点名的行为测试：

- `session_event_recovery`
- `session_protocol_reconnect`
- `goal_restart_recovery`
- `goal_lifecycle`
- `approval_goal_flow`
- `agent_recovery`
- `agent_control_service`
- `agent_control_spawn`
- `agent_memory_isolation`
- `subagent_production_baseline`
- `turn_engine_parity`
- `turn_coordinator_lifecycle`
- `turn_pipeline_order`
- `daemon_streaming_turn_e2e`
- `principal_turn_isolation`
- `google_sync_recovery`

`turn_service_equivalence` 在 cutover 后应改成**唯一** TurnService 的 daemon/exec adapter parity，不要再证明两个 orchestrator 等价。

M1/M6/M9 里的 `rg` / `test ! -e` 可作为删除门禁，不能替代上表行为测试；也不要为了
“升级”而机械删除仍有效的 source architecture guard。

### Q10. §9 能否防止 Executive 2.0？ — REVISE

不能。见 B5。

---

## 4. 同意保留、不要在修订时删掉的部分

这些是计划里已经正确的约束，修订稿应原样保留：

1. 先定 authority 再移动，不允许机械搬目录。
2. 一个 durable fact 一个 authority；禁止双写、双 reducer、feature flag 并行两套。
3. 不新建 `orchestration` god crate；不长期兼容 re-export。
4. daemon 与 `aletheon exec` 共用同一 Turn use case。
5. Runtime 只做 Session/Turn/Agent authority，不做跨域编排。
6. 不需要独立 Goal domain crate。
7. Gateway 拥有 typed protocol，host 拥有 socket/systemd。
8. Session adapter 合入已有 `adapters-sqlite`，不新建 session crate。
9. M0 先冻结账本和 gate，再移动生产代码。
10. M1 删除 re-export：`session_service` / `governed_capability` / `harness_factory` / `memory_gateway` / `capability_benchmark` / `cognitive_role_workflow` 目前确实是兼容层。
11. `cognitive_role_workflow/{stages,state_machine}.rs` 确为幽灵文件：父模块已是 `pub use agora::cognitive_role_workflow`，agora 自己有同名 `mod stages` / `mod state_machine`。
12. M5 不得整包搬入 runtime。
13. M10 系统安装态：`sudo bash scripts/aletheon.sh deploy`、三方 SHA-256、restart counter、真实 LLM、真实 GBrain、三分计数分开报告。
14. §0.4：Grok 通过设计 ≠ 授权改代码。

---

## 5. 修订清单（给 Codex 的最小闭环）

按这个顺序改计划，不要先写 Rust：

1. **重写 §4.1**
   当前允许 `application -> {contracts, runtime, kernel?}`。
   当前禁止 `application -> {agora, corpus, platform, gateway, mnemosyne,
   adapters-*, aletheon}`；当前 dirty worktree 同时禁止 `dasein/cognit/metacog`。
   `dasein/cognit/mnemosyne/metacog` 只有在先消除传递回边并由 architecture gate
   重新证明无环后才可放行。每个 packet 都检查完整传递图，不写永久静态白名单。

2. **重写 M6 目标树**
   `application::turn` 保留 command / service / outcome / ports，并由 provider-neutral
   TurnService 拥有授权后的用例顺序。
   Cognit 拥有 session/ReAct/stream；runtime 拥有 ID/reducer/writer/recovery/fence；
   外部 domain 和 transport 由 adapter 实现 ports。
   `daemon_react` 拆成 Cognit 核心与 config/tool/artifact/event adapters，不得进入 composition。
   `TurnEngineContext` 去掉 `notification_sender`。

3. **补全 §3.3**
   覆盖 B3 表中每一个顶层 wiring 路径。
   `governed_review` = application port/use case + filesystem adapter，不能是 host persistence。
   `core_rpc` = `adapters-inference::core_rpc` transport；host 只管配置/socket/lifecycle。
   `embodiment`、`evolution` 等必须有唯一 owner；表中不能保留“或”。

4. **改 M2.1 / M3.1**
   Goal artifact I/O 成 application-owned port，由 platform/filesystem adapter 实现。
   `GoalWorker` 拆成 application advance/retry service、host interval/cancel scheduler、
   gateway progress adapter 与 platform quota adapter；composition 只注入。
   合并已有 `application/src/goal_*.rs`。

5. **改 §3.4**
   拒绝 `adapters-runtime` 这个名字。
   Google SQLite 选定唯一 owner。
   写明 Corpus Google 工具 vs adapter sync。

6. **改 §9**
   加上 B5 的传递依赖、authority census、effect census、composition policy 和行为门。
   LOC/命名降为 review trigger。`runtime::orchestration` 先审计消费者与行为；无消费者
   则删除，只有证据证明语义属于 Agora 时才允许合并。

7. **改 M5/M6/M10 验证**
   补 Q7 表中的符号和测试。
   把 Q9 的 `include_str!` 测试分成 source architecture gate 与 behavior parity；
   前者可以保留但不能充当后者。
   补 `MemorySensitivityV1` 与 transient approval cache。

8. **补 canonical identity 契约**
   明确 Runtime 是否唯一 mint Session/Turn ID，区分 contracts wire/domain 表示，集中
   `turn_coordinator` 中的 String/UUID 转换，并验证 alias 不能成为 authority。

9. **冻结可复现基线**
   记录 HEAD、被审计划 digest、dirty path list、tracked diff digest、LOC 命令和依赖图
   命令。dirty 基线变化后必须重新核查本文 path:line。

10. **不要做的事**
   - 不要新建 Goal crate
   - 不要新建 orchestration crate
   - 不要把 TurnPipeline 派给 runtime
   - 不要把 provider-neutral Turn 用例顺序派给 composition
   - 不要把未分配模块塞进 composition 然后宣称收敛
   - 不要把本文当成实施 packet

修订后再走一遍计划 §10。在 blocking 全部闭合前，审核结论保持 **REVISE**。

---

## 6. 审核范围外、不要借机改的东西

- 不改提示词、模型路由、工具策略
- 不改数据库 schema 或 wire protocol
- 不改机器人控制 / 安全状态机
- 不把当前未提交的 `verification_command.rs` / `worktree.rs` / `context_memory.rs` 当成已完成迁移；它们只是可用 seam，必须随 owner 走
- 不运行 workspace-wide cargo；本文是文档审核，不是验证报告

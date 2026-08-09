# Executive 退休、Compatibility Seam 与架构 Fitness Gates 计划

> 状态：Draft，只有迁移与删除设计，不包含生产代码变更
> 基线：`dev@bd1ceac2965832f15cf45b811fd34e052e7e9a67`
> 上位计划：`2026-08-08-agent-kernel-v2-complete-rearchitecture.md`
> 前置：删除按文件族分别受 `K7`、`RA-06`、`D6`、`APX-05`、`CGP-08`、`E7` 约束，不把整套专项计划串成一个全局大前置；Fabric root 的最终硬顺序固定为 `XRET-03 -> E7 extension rows -> D6 non-extension/root closeout -> AK2-25 Contracts rename -> XRET-04 -> XRET-05`

## 1. 最终决定

`executive` 不会改名成 `core`、`runtime`、`orchestrator` 或 `coordinator`；它必须从 active workspace 和生产依赖图中删除。

删除不是最后一次把 12 万行代码一次性移动。每个 owner 的迁移 PR 在新路径通过等价 smoke 后，立即删除自己负责的 Executive 业务模块。最终退休 PR 只能删除：

- 无状态、无 I/O、无 worker 的 deprecated re-export；
- 已无生产 caller 的 one-way compatibility adapter；
- 空的 module/Cargo shell、旧 examples/test imports 和 inventory 条目。

若最终 PR 仍在移动 Turn 状态机、SQLite repository、RequestHandler 或 extension 实现，说明前面的 owner cutover 没有完成，禁止删除 crate。

## 2. 为什么现有 Executive 不能继续做“协调层”

`executive/Cargo.toml` 直接聚合 Fabric、Kernel、Agora、Cognit、Corpus、Mnemosyne、Runtime、Dasein、Metacog、Hardware；源码又同时包含 `application`、`adapters`、`composition`、`core`、`host`、`compatibility`、`extensions` 和 `tools`。

根 `lib.rs` 进一步：

- crate-level blanket allow 多项 Clippy 规则；
- 重导出 config、`AletheonExecutive`、Kernel admission、Application facade；
- 用名为 `runtime` 的 module 重导出 Application/adapter；
- 用 `testing` 暴露 concrete SQLite/Gmail/GBrain/Pi adapter；
- 为 exec/CLI 再重导出 Fabric/Kernel 大量类型。

这使任何调用者都能通过 Executive 走捷径，新的 owner 即使建立也无法成为唯一入口。因此“保留一个薄 Executive 协调器”仍会永久保存错误依赖方向。

## 3. Executive 目录删除映射

| 当前文件族 | 目标 owner | 迁移后动作 |
|---|---|---|
| `application/turn_*`、`daemon_turn/**`、`pre_turn.rs`、`post_turn.rs`、`settlement.rs`、`context_assembler.rs` | Runtime | canonical Turn cutover PR 内删除；legacy event mapping 暂移 Gateway seam |
| `application/agent_control/**`、`application/agent/**`、`core/sub_agent.rs`、`core/runtime_registry.rs` | Runtime `AgentSupervisor`/`DelegateBackendRegistry` | spawn/wait/recovery 等价后删除；旧 `AgentRuntime` 不保留 |
| `application/session_service.rs`、`adapters/session/**` | Runtime + runtime-sqlite adapter | 单 writer cutover 后删除；projection adapter迁 owner |
| `core/session.rs`、`core/session_gateway/**`、`host/daemon/session_manager.rs` | Runtime query/`ContextWorkingSet`/Gateway | `TuiSessionManager` 删除；不得整块改名 |
| `application/approval*`、`goal draft`、generic stimulus/request facade | Application | 纯用例和 port 提取后删除 Executive 原文件 |
| `application/goal/**` 中 attempt/worker/verification/budget | Runtime/可选 workflow extension | 按真实 caller 迁移或删除，不进入 basic Application |
| `application/conscious*`、`conscious_action.rs`、`dasein_workspace_adapter.rs` | Dasein/Metacog/Agora + Runtime settlement hooks | 领域 proposal/verdict/workspace API 生效后删除 bridge |
| `application/memory_*`、`hook_lifecycle/session_distiller.rs` | Mnemosyne + GBrain adapter | local/GBrain preservation 通过后删除 |
| `application/governed_capability.rs`、`turn_runtime_ports.rs` 中 execution policy | Kernel + Runtime | descriptor/permit/invoke/receipt 路径 cutover 后删除 |
| `application/embodiment_*`、`robot_*`、`world_state.rs` | Hardware + Robot VLA extension | simulation/bridge/VLA 等价后删除；保留 legacy type conversion 直至双方切换 |
| `application/inference_port.rs`、`harness_factory.rs`、`adapters/runtime/native_cognit.rs` | Cognit `InferencePort/CognitiveRun` + provider adapter | provider/Native cutover 后删除 |
| `adapters/runtime/pi*`、`process_supervisor.rs`、`extensions/runtime/**` | Pi adapter + Kernel process/lease adapter | delegate wait/cancel/recovery 等价后删除 |
| `adapters/channel/gmail/**`、`adapters/google/**`、`adapters/external/**` | Gmail extension/Gateway channel adapters | inbound/outbound/approval/reconcile 各自切换后删除 |
| `adapters/gbrain/**` | GBrain supplemental adapter | outbox/reconcile smoke 后删除 |
| `adapters/events/**`、`adapters/evaluation/**`、`adapters/episode/**` | 对应 owner 的 SQLite/projection adapter | table owner 拆分后删除 |
| `adapters/plugin/**`、Application extension modules | Corpus/extension-admin + FS adapter | package lifecycle cutover后删除 |
| `composition/config/**` | 各 owner typed config + `aletheon` preflight composition | 不复制一个全局 `AppConfig`；迁完即删除 |
| `composition/**`、`host/**`、`core/runtime_core.rs` | `aletheon` composition、Gateway、linux/provider adapters | official topology 切换后删除 |
| `core/system_core_runtime.rs`、`host/core_rpc/**` | machine `InferenceBroker` + Gateway/internal protocol adapter | 改正语义后删除旧类型/path |
| `tools/self_observe.rs` | Dasein query + Corpus tool adapter | tool 经 typed port 后删除 |
| `compatibility/**` | 无长期 owner | 达到各 seam deletion gate 后全部删除 |
| `lib.rs` public facade | 无 | production caller 为零后删除 crate |

## 4. Compatibility seam 账本

Compatibility 只允许“旧输入/路径 -> 新 owner”，不允许“双向同步”或“双 writer”。每条 seam 必须登记 owner、旧 caller、调用/读计数、新入口、删除 PR 和最晚版本。

| 当前 seam | 临时 owner | 已知旧 caller | 基线调用/读计数 | 新入口 | 删除 PR / 最晚门 | 允许的临时形态与删除条件 |
|---|---|---|---|---|---|---|
| `crates/executive/src/compatibility/legacy_session_service.rs`（source-level `COMPAT`） | Runtime compat | TUI/CLI/ACP、legacy RPC handler | `TBD@XRET-00 (B0)` | Runtime typed Session command/query | `RA-06 / XRET-04` | 只翻译 command/query；不得持 store/workspace map/mint ID；连续 canary 为零后删 |
| `SessionService::ensure_legacy_projection` | migration composition | bootstrap/legacy projection reader | `TBD@XRET-00 (B0)` | `aletheon migrate` + Runtime projection rebuild | `APX-04 / XRET-04` | 仅一次性 backfill，不在 serving path 自动写；manifest/checksum 完成后删 |
| `host/daemon/server.rs` legacy handshake/JSON aliases | Gateway compat | 已发布 TUI/CLI/ACP/旧 binary | `TBD@XRET-00 (B0)` | versioned Gateway protocol/client | `CGP-08 / XRET-04` | `LegacyJsonRpcAdapter` 只转 typed command；supported clients 验证且 alias 计数为零后删 |
| `application/command_dispatcher.rs` V0 prompt-result adapter | Gateway/Application compat | TUI/CLI/ACP | `TBD@XRET-00 (B0)` | typed Application result | `CGP-06 / XRET-04` | 只做 schema 纯转换，不判断 terminal/policy；所有客户端切换后删 |
| `turn_pipeline.rs` legacy TUI wire projection | Gateway projector compat | TUI reducer | `TBD@XRET-00 (B0)` | versioned Runtime projection event | `CGP-05 / RA-06` | 只读 Runtime journal/event，不持 authority；新 reducer 验收后删 |
| `crates/executive/src/core/sub_agent.rs`（source-level `COMPAT`）：`SubAgentRuntime`、`SubAgentExecutionContext` | Runtime delegate compat | AgentControl、Pi、provider worker；legacy `RuntimeRegistry` 间接服务 Goal attempt coordinator/worker | `B0@bd1ceac`：2 个 declared symbols；17 处直接 production reference，分布于 4 个 source files；另有 2 个间接 Goal consumer files；动态调用计数在 `XRET-00` 落表 | Runtime `DelegateBackend` + owner-assigned delegate execution binding | `E6-K6d + matching APX-05/host caller-zero -> RA-06`；硬 deadline：`XRET-03` 前 | 只允许把旧 task/context 单向翻译为 Runtime command；不得 mint Agent/Process/Operation/Session ID、持 registry/state 或自行判 terminal；Pi installed smoke、active-child drain/reconcile 和所有直接/间接 caller 归零后删除 |
| Pi compatibility runtime/launcher | Runtime compat + `adapters/pi` | Goal worker、AgentControl/bootstrap launcher | `TBD@XRET-00 (B0)` | Runtime `DelegateBackend` | `E6-K6d / RA-06` | 旧 runtime ID 仅单向 route；spawn/wait/cancel/recovery 切换且 active child reconcile 后删 |
| Goal legacy CRUD/status/resume | optional Application goal compat | RPC Goal、Gmail/Google、worker | `TBD@XRET-00 (B0)` | canonical GoalDraft command 或 Runtime work command | `APX-03 / XRET-04` | 不另写 legacy table；caller/reader census 与 shared DB split 完成后删 |
| `compatibility/persistence_migrations.rs` | `aletheon migrate` compat | daemon/bootstrap migration path | `TBD@XRET-00 (B0)` | owner-specific migration bundles | `APX-04 / XRET-04` | 旧 manifest 只编排 checksum；每张表 owner/writer 定案后删 |
| Executive crate/path alias 与根级 runtime/testing re-export（SPLIT/MERGE 内 sub-seam） | 各 owner compat | examples、integration/external imports | `TBD@XRET-00 (B0)` | owner public API | `XRET-01` caller cut / `XRET-04` physical delete | 不重导 concrete store；public symbol census 为零后删；AK2-25 不拥有此 alias |
| duplicate Agent profile loaders | Runtime profile compat | host bootstrap/config/profile callers | `TBD@RA-00 (B0)` | 经 census 精确命名的 loader(s) | `RA-05 / RA-06` | 先证明语义；不等价则分别命名，不假设唯一 Markdown loader；caller 切换后删旧 facade |
| Fabric path alias（Fabric SPLIT/COMPAT sub-seam） | owner-specific compat | 全 workspace、migration reader、supported external API | `TBD@D0/XRET-00 (B0)` | owner API + `contracts` primitives | `E7 -> D6 -> AK2-25`；硬 deadline：`XRET-04` 前 | 只允许标注 deadline 的单向 re-export；mechanical rename 和 source gate 归零后删；不包含任何 `executive::` alias |

Seam 禁止事项：

- [`Executive source disposition ledger`](./2026-08-08-executive-source-disposition-ledger.md) 中每个 source-level `COMPAT` path 必须以完全相同的 path 在本表标为 `source-level COMPAT` 并恰好出现一次，反向亦然。`SPLIT/MERGE` 文件中的局部 compatibility 只能标为 `sub-seam`，不参与这项 `COMPAT` 行数基数，但仍必须登记 exact source path/symbol、last caller、deadline 与唯一 deletion PR；CI 对 source-level missing、duplicate、wildcard-only 或把 sub-seam 冒充整文件 `COMPAT` 一律 hard fail；
- 不能有自己的 DB connection、cache、registry、worker、retry loop 或 `Uuid::new_v4`；
- 不能根据错误字符串、runtime 名或旧字段猜 policy；
- 不能把新 event 回写旧 authority 再让旧 path 驱动新 owner；
- 不能以 feature flag 永久存在；deadline 到期前未删除必须 Architecture Review；
- 未观测到 caller 不是直接删除理由，必须同时证明生产配置、installed binary、socket clients 和 migration reader 均不需要。

## 5. 分 PR 退休顺序

`XRET` 编号是删除 gate，不是新的业务 owner。每个 owner 在等价 smoke 后立即删除自己范围内的 Executive 实现；退休计划只核验其 deletion evidence。下表是逐文件族硬前置，不要求无关 owner 相互等待：

| 待删 Executive 文件族 | 必须先完成的 owner gate | 对应 XRET gate |
|---|---|---|
| Agent/Session/Turn/Delegate authority、旧 registry/seam | `RA-06`（其自身硬依赖 `E6-K6d` 与 matching `APX-05`/host caller-zero evidence） | `XRET-02`；concrete Pi remnant 计入 `XRET-03` |
| governed capability、process/permit/settlement glue | `K7` | `XRET-02` |
| Dasein/Metacog/Agora/Mnemosyne/Cognit/Corpus rich surface | `D6` | `XRET-02` 后的 alias/rich-surface hard zero |
| Application、Approval/Goal 与 persistence/host I/O 混合模块 | `APX-05` | `XRET-02` |
| Gateway、daemon、composition、TUI/CLI/ACP host 实现 | `CGP-08` caller/writer/bind/spawn hard zero 与 exact deletion inventory | `XRET-04` 唯一物理删除 owner |
| Gmail/Pi/GBrain/Robot/Hardware concrete implementations | 对应 `E2-K6a/E3/E4-K6b/E5-K6c/E6-K6d` installed equivalence | `XRET-03` |
| SQLite/filesystem persistence implementations 与 shared migration writer | `APX-04` owner-specific adapter/migration cutover | `XRET-03` 对应 storage family |
| Provider/inference broker 与 local RPC concrete adapter | `D2` InferencePort/provider boundary、`CGP-04` host wiring | `XRET-03` 对应 provider family |
| Linux/execd/process/workspace concrete adapter | `K5` ProcessController/Linux cutover、`CGP-04` 唯一 wiring | `XRET-03` 对应 Linux family |
| plugin/extension-admin concrete path | `C0/E0` caller、installed config 与 preservation decision 已定案；未保留者有删除证据 | `XRET-03` 对应 plugin family |
| Executive extension bootstrap/config/fallback/schema writer | matching `E2-K6a/E3/E4-K6b/E5-K6c/E6-K6d` owner cutover | `XRET-03` 唯一 concrete deletion owner |
| extension-specific Fabric rich row/re-export | matching extension cutover + `XRET-03` drain + per-family caller-zero evidence | `E7` 唯一 Fabric extension-surface deletion owner；`XRET-04` 前 hard zero |

不可交换的最终删除链只有一条：`XRET-03 -> E7 extension rows -> D6 non-extension/root closeout -> AK2-25 机械 fabric -> contracts rename -> XRET-04 -> XRET-05`。E7 与 D6 的准备工作可并行，但两个 PR 不得并行写 Fabric root；root write 权按此顺序交接。`K7/RA-06/APX-05/CGP-08` 各自可在依赖满足后并行完成，但都必须在 `XRET-04` 前为绿。不得先删 Executive host/compatibility 再 rename，也不得在最终删除 PR 搬业务实现。

`CGP-08` 只证明旧 host/composition tree 已 inert 且 deletion-ready，不物理删除 RequestHandler、HandlerPorts、launcher、`executive/src/host/**` 或 `composition/**`。`AK2-25` 完成后，`XRET-04` 是这些 host/composition/compatibility remnants 的唯一物理删除 PR；两阶段不得各删一遍同一文件族。

### XRET-00：冻结 Executive surface 与 seam ledger

- 记录 Executive 所有 production files、public symbols、Cargo callers、wire/schema surfaces；
- `executive-layers.tsv` 每项增加 target owner、迁移 PR 和 deletion gate；
- 从 source disposition ledger 生成 `COMPAT path -> seam row` cardinality 清单，要求双向恰好一条；
- 禁止新增 Executive module/public re-export/concrete adapter；
- 为每条 legacy route/read 增加可观测计数，不记录 secret/payload。

验收：inventory 与 source 完全一致；所有 `COMPAT` exact path 双向 cardinality 为 1；`core/sub_agent.rs` 的静态 symbol/caller 基线和不含 payload 的动态调用计数均已落表；无行为变化。回滚只删除 gate/ledger。

### XRET-01：切断外部 crate 对 Executive public facade 的依赖

- `interact` 改用 Gateway；`aletheon` 改用新 composition；
- examples 改用 Runtime/Application public API；
- integration tests 移到 owner crate，删除只验证旧 re-export 的测试；
- `executive/src/lib.rs` 不再重导出 Kernel/Fabric/concrete adapters。

验收：Executive 之外的 production manifest/source 零 `executive` 依赖；Executive 内部迁移尚可继续。回滚单个 facade import，不恢复外层业务逻辑。

### XRET-02：按 owner 删除 Executive application/core 业务模块

这是多个小 PR，不是一个 commit：Runtime、Dasein/Metacog/Agora、Mnemosyne、Kernel、Application、Hardware/Robot 各自完成新路径后删除对应文件族。

验收：每个 PR 都有 authority uniqueness、restart/recovery、focused smoke；旧 module 不留 forwarding service。回滚只恢复一个 one-way seam，不能恢复第二 writer/state machine。

### XRET-03：按 adapter/extension 删除 Executive concrete implementations

- SQLite/Linux/provider/Pi/GBrain/Gmail/Google/Robot/episode/plugin 分别切换；
- 每个扩展先通过保存舱 installed equivalence；
- migration、outbox、receipt、credential 和 reconcile owner 固定后删除旧实现。

前置按文件族核验，不要求无关 family 在迁移过程中互等；但 `XRET-03` 整体完成必须同时满足：`APX-04` SQLite/filesystem、`D2` provider、`K5` Linux/execd、`CGP-04` 唯一 wiring、所有受支持的 `E2-K6a/E3/E4-K6b/E5-K6c/E6-K6d` 已完成。plugin/extension-admin 还必须有 `C0/E0` caller、installed config 与 preservation decision；未进入 preservation manifest 的能力不能在这里默认迁出为新产品层。

验收：每个 source-ledger concrete file 都能命中上表恰好一个 owner-family gate；SQLite/FS/provider/Linux/plugin 与受支持扩展的旧 writer/caller 均为零；`core-linux` 不编译扩展；`full-linux` 能编译并按配置启用；Gmail/Pi/GBrain/Robot/Hardware 不拥有第二 Turn/Session/Permission authority。通过后先由 E7 清退 extension-specific rich rows/re-export，再由 D6 收口 non-extension/root surface，最后才由独立 `AK2-25` PR 做纯机械 Contracts rename。

### XRET-04：删除 Executive host/composition 与 compatibility seams

硬前置：`K7`、`RA-06`、`D6`、`APX-05`、`CGP-08`、`E7` 的对应 per-slice deletion gate 均为绿；`XRET-03` 已通过；`AK2-25` 已完成机械 Contracts rename，旧 Fabric path/alias 为零；新 `aletheon` composition、Gateway、TUI/CLI/ACP、official socket topology 已稳定。

- 删除 RequestHandler/HandlerPorts/launcher/RuntimeCore/UserRuntime/SystemCoreRuntime；
- 逐条关闭 legacy route/read，验证计数为零；
- 删除 `compatibility/**` 和 persistence master shim；
- 运行 full-linux rollback/reconcile drill 和 canary。

验收：Executive 只剩空 `lib.rs/Cargo.toml` 或完全无业务源码；所有 compatibility seam、alias、legacy route/read 和 public re-export 已删除，无 writer/worker/I/O。发生回滚时只能恢复读取当前 owner schema 的单向 binary facade，不能恢复过期 schema writer。通过后才能进入 `XRET-05`。

### XRET-05：机械删除 crate

硬前置：`XRET-04` 已通过；D6/E7 已清退 rich types/re-export，`AK2-25 fabric -> contracts` 机械 rename 已完成，旧 Fabric/Executive path alias、compatibility seam、legacy read 与 public re-export 的计数和 source gate 均为零。

- 从 workspace members、Cargo.lock、profiles、architecture inventories、CI、docs、examples 删除 Executive；
- 删除 crate 目录；
- 所有旧 symbol/path gate 从 non-increasing 转为 hard zero；
- final installed provenance、fault injection 和 soak。

验收：`cargo metadata`/resolved graph 无 Executive；source、manifest、scripts、docs 的生产引用为零；active workspace 无 Executive。该 PR 不允许业务 diff或 schema change。

## 6. 必须落地的 Fitness Gates

### 6.1 依赖方向

```text
contracts -> no workspace crate
kernel -> contracts only
domains -> contracts / own narrow ports
runtime -> contracts + kernel + domain APIs
application -> contracts + runtime
gateway -> application
interact -> gateway-client/protocol
aletheon -> concrete composition dependencies
```

检查 resolved features，而不只看直接 `Cargo.toml`。硬失败规则：

- Kernel 引用 Runtime/domain/Application/Gateway/TUI/SQL/HTTP；
- Runtime 引用 Gateway/TUI/SQLite/HTTP/Gmail/Robot/Hardware/provider concrete type；
- 领域引用 Application/Host/composition root；
- Interact 引用 Executive/Runtime/Kernel/repository/Unix framing；
- composition root 被任何 core/domain crate 反向依赖；
- `contracts` 出现 repository、service、wire payload 或 extension domain model。

### 6.2 Authority uniqueness

自动 census 并要求每项恰好一个生产 owner/implementation：

- Agent/Session/Turn aggregate constructor 和 ID allocator；
- Runtime journal、Kernel execution journal、Approval store、Self store、Meta store；
- Agent supervisor、Delegate backend registry、Capability executor registry；
- operation/permit/receipt settlement writer；
- official user socket Runtime writer；
- migration executor。

`SessionManager/SessionService/TuiSessionManager/LegacySessionService/SessionGateway` 等同义类型不能靠不同名字绕过；gate 同时扫描 trait impl、constructor、journal append 和 table write call sites。

### 6.3 I/O 与 composition purity

- Application/Runtime/domain production prefix 禁止 `rusqlite`、raw FS/process/network/platform SDK，除非该 crate 本身就是明确的 adapter package；
- repository `open` 不 migrate，composition `compose` 不 I/O/spawn，transport handler 不持有 component graph；
- `aletheon migrate` 是唯一 migration executor；startup 只 compatibility check；
- old/new writer PID、socket 和 DB lock 不得并存；
- secret value 不进入 Debug/Serialize/Clone、argv、environment dump、TUI、trace、prompt、receipt。

### 6.4 代码形状与命名

- 删除 Executive/Fabric 后禁止永久 `executive::`/`fabric::` compatibility path；目标新名是 `contracts`，不是 ABI；
- `Runtime` 只指 canonical Agent Runtime；Pi 为 `DelegateBackend`，Cognit 单 Turn 为 `CognitiveRun`，machine provider 为 `InferenceBroker`；
- `Manager` 默认禁止；`Service` 仅 Application facade；`Handler` 仅 transport；Adapter 名必须指出 Port；
- 普通文件目标 `<=500` 行、`mod.rs <=200` 行、函数 `<=60` 行、constructor 依赖 `<=5`；超限必须是有期限 exception；
- 禁止 crate-level blanket Clippy allow、裸 `String` ID/status/kind、`anyhow::Result` 穿过 public domain API；
- 禁止全局 service bag、无 caller public re-export、只为拆行数而拆 helper。

### 6.5 Presentation 与 error chain

- TUI/CLI/ACP 零 core ID mint、effective policy、TaskKind 推导、repository、raw JSON method/framing；
- Runtime/Kernel/domain error -> ApplicationError -> GatewayErrorCode -> Presentation；
- 禁止 `message.contains(...)` 分类、吞掉 I/O result、把 provider rejection 渲染成成功；
- projection 与 journal/receipt/PID/log 冲突时整体失败，不以 monitor PASS 覆盖事实。

### 6.6 Compatibility budget

配置一个只降不升的 seam budget：

```text
executive production files
executive public symbols
external executive imports
legacy RPC aliases
legacy persistence reads
compatibility writers
fabric/executive source paths
blanket lint allows
oversized constructors/functions
```

任一新增或反弹都失败。Fabric path/alias 由 `AK2-25` 清零，host/compatibility seam、legacy route/read 与 public re-export 在 `XRET-04` 清零，其余 Executive crate/file/symbol 目标在 `XRET-05` 前全部为零；compatibility writer 从 `XRET-00` 起目标就必须为零。

## 7. 验收策略

不恢复“大而全测试”思路。每个 owner PR 只验证它真正保护的不变量：

- narrow compile/lint/dependency/authority gate；
- owner state-machine/port contract；
- migration-on-copy、restart/replay/reconcile；
- 一条真实入口到 authoritative receipt 的 installed smoke；
- 涉及模型 routing/arguments 时连续三次真实 TUI；
- Gmail/Pi/GBrain/Robot/Hardware 分别执行保存舱 smoke；
- 最终删除前执行 maintenance/drain、binary rollback、external-effect forward reconcile 和 canary/soak。

任何 false success、writer 冲突、未知 authority、secret/scope violation、terminal hanging task、valid late receipt 丢失或 projection 与 journal 冲突，均阻止 seam 删除。

## 8. 回滚与删除安全

- 删除文件前先用 production caller、runtime route metrics、schema reader 和 installed profile 四项证据确认目标；不能只凭 `rg` 零引用。
- 每个 PR 可通过 revert 恢复 adapter/facade，但不得恢复双 writer；先 drain/stop 当前 writer，再启动旧 binary。
- schema contract/delete 单独于 behavior cutover；旧 binary 仍需 rollback 时只做 additive schema。
- 已发生不可逆 effect 后保留新 receipt/outbox，以 reconcile 前进；不能恢复会导致重复 Gmail send/Robot command 的旧快照。
- 最终 XRET-05 仅机械删除；若失败，恢复空 facade crate，不把业务实现重新塞回 Executive。

## 9. 完成定义

- workspace、resolved dependency graph、production source、scripts 和 examples 中无 Executive。
- 所有旧 Executive file 都有可追溯的 owner、迁移 PR、等价证据和删除记录。
- compatibility seam、writer、alias、legacy read、public re-export 预算全部为零。
- Agent/Session/Turn/Kernel/Self/Metacog/Memory/Approval/Extension 各有唯一 authority 与持久化 owner。
- 删除 Executive 后，`core-linux` 与 `full-linux` 均可按各自 profile 启动；Gmail、Pi、GBrain、Robot VLA 和 Hardware 未被误删。
- 架构 gate 能阻止 Executive 以新名字、service bag、mega handler 或第二 composition root 的形式回归。

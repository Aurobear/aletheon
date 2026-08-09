# Application 与持久化/主机 Adapter 提取计划

> 状态：Draft，只有迁移设计，不包含生产代码变更
> 基线：`dev@bd1ceac2965832f15cf45b811fd34e052e7e9a67`
> 上位计划：`2026-08-08-agent-kernel-v2-complete-rearchitecture.md`
> 目标：把 `executive/src/application` 收缩为可关闭、可替换的纯用例层，把 SQL、文件系统、进程、网络和领域 Runtime 权威全部移给真正 owner

## 1. 这份计划解决什么

当前目录名叫 `application`，但实际同时包含了：

- Agent/Session/Turn 的状态机与恢复；
- Dasein、Metacog、Mnemosyne、Agora、Hardware 的跨领域协调；
- SQLite repository 和 migration；
- 文件系统 trust、quota、checkpoint、extension package 操作；
- Provider/Pi/Robot/Gmail 适配；
- daemon 生命周期、健康检查和 transport-facing facade。

因此，简单把 `executive/src/application` 整个移动到新 `application` crate，只会得到第二个 God Crate。本计划采用“按 authority 拆出，再留下最小用例”的顺序；不会先创建一个同规模目录然后批量改 import。

完成后应满足：

```text
Presentation / Extension transport
        -> Gateway typed command
        -> Application use case
        -> Runtime command/query port 或 Application-owned store port

Application -X-> rusqlite / std::fs / tokio::process / reqwest / UnixStream
Application -X-> Turn state machine / provider loop / Kernel internal table
Application -X-> concrete Gmail / Robot / Pi / GBrain type
```

Application 可以从 `core-linux` composition 中关闭；关闭后 Runtime、Kernel、Cognit、Dasein、Metacog、Agora 和 Mnemosyne 仍能启动并执行由内部/受信调用方提交的 typed command。关闭 Application 只会失去人类用例、Goal Draft 和 Approval 展示/解决入口，不会失去核心 Agent 语义。

## 2. 不做什么

- 不把 Agent、Session、Turn、Delegate 状态机归给 Application；它们属于 Runtime。
- 不把 Self、Metacog、Memory、Workspace 规则包装成 Application 私有“service”后继续持有。
- 不让 Application 构造 SQLite、Provider、Pi、MCP、Robot、Gmail、systemd 或 Linux process adapter。
- 不在 repository constructor 或 daemon startup 中隐式执行 migration。
- 不以 Cargo feature 隐藏反向依赖；引入 SQL、HTTP、gRPC、Linux path/process 的实现必须物理隔离到 adapter package。
- 不在这组 PR 中删除 Gmail、Pi、GBrain、Robot VLA 或 Hardware 能力；对应实现先经各自保存舱迁移。
- 不新增“大而全 Application 集成测试”；只保留 owner contract、migration check 和关键 installed smoke。

## 3. 当前证据与错误边界

以下是 `dev` 上需要拆解的生产路径，不是按目录名推断：

| 当前位置 | 当前实际职责 | 问题 | 目标 owner |
|---|---|---|---|
| `executive/src/application/turn_pipeline.rs`、`turn_engine.rs`、`turn_lifecycle.rs`、`turn_recovery.rs`、`daemon_turn/**` | Turn admission、执行、终态、恢复和 wire projection | 核心 Runtime 状态机被误放进 Application；`TurnPipeline` 文件约 2500 行 | Runtime；legacy wire projection 暂留单向 Gateway seam 后删除 |
| `executive/src/application/agent_control/**` | child Agent spawn、mailbox、generation fence、settlement、recovery、SQLite port | Agent authority 不能由可关闭应用层拥有 | Runtime `AgentSupervisor`；SQLite 实现进 runtime-sqlite adapter |
| `executive/src/application/session_service.rs` | canonical Session 写入、projection、legacy backfill，且直接持有 `rusqlite::Connection` | use case、authority、store 和兼容投影混为一体 | Session authority/journal 进 Runtime；Application 只保留 Session command/query facade；SQLite 进 adapter |
| `executive/src/application/approval_service.rs`、`approval/repository.rs` | Approval aggregate、resolution、delivery/apply ledger 和 SQLite | Approval 是 Application-owned，但 repository 不应在 Application core | aggregate/use cases/store port 留 Application；SQLite adapter 与 migration bundle 外移 |
| `executive/src/application/goal/**`、`goal_service.rs` | Goal CRUD、attempt、worker、budget、verification、artifact、SQLite schema | Goal Draft 用例和 autonomous execution 混在一个 store | `GoalDraft` 留 Application；执行 attempt/worker 迁 Runtime/对应 extension；SQL/FS 进 adapter |
| `executive/src/application/thread_authority.rs` | principal-thread 绑定和 filesystem lock | requested settings、host trust 和 authority grant 混名，且直接文件 I/O | request preference 进 Application DTO；peer/workspace trust 进 Gateway/host；Kernel/Dasein 自己 mint grant；文件实现进 linux/fs adapter |
| `executive/src/application/workspace_trust.rs` | repository trust policy、persisted decision、path inspection | 用例层直接解释主机路径和 executable trust | typed review use case 留 Application；canonical path/metadata/persistence 进 linux/fs adapter；最终 permission 由 Kernel/Dasein owner 判定 |
| `executive/src/application/memory_gateway.rs`、`memory_maintenance.rs`、`memory_projection.rs` | Memory public flow、文件读写、maintenance、GBrain 补充状态 | Memory 领域被 Application 二次实现 | Mnemosyne API/worker；GBrain adapter；Application 只可转发明确的 memory admin use case（若保留） |
| `executive/src/application/conscious*`、`dasein_workspace_adapter.rs`、`coding_metacog_adapter.rs`、`metacog_approval.rs` | Self/Metacog/Agora 协调与 mutation bridge | Application 成为领域间隐藏 authority | Dasein/Metacog/Agora 各自 port；Runtime 只在 Turn settlement 边界调用 |
| `executive/src/application/governed_capability.rs`、`embodiment_*`、`world_state.rs` | capability/permit/embodiment policy 与进度 | Application 介入 Kernel 与 Hardware authority | Kernel enforcement、Hardware safety domain、Robot extension；Application 只展示 Approval |
| `executive/src/application/extension_install.rs`、`extension_manage.rs`、`extension_snapshot.rs` | extension admin 用例以及 archive/FS I/O | 可保留用例语义，但 concrete package store 不应在 Application | optional extension-admin use cases + Corpus/extension store port；FS/archive adapter |
| `executive/src/application/orchestration/**` | workflow graph、delegate selection、LLM builtins、文件 store | 与 Runtime delegate 重复，甚至内嵌 provider | Runtime delegate/goal command；只保留确有用户价值的 optional workflow facade；FS/provider 实现删除或迁 owner |
| `executive/src/application/storage_quota.rs`、`workspace_checkpoint.rs`、`settlement.rs`、`verification/**` | 主机磁盘、工作区 diff/checkpoint/review | host effect/rollback 语义不属于通用 Application core | Kernel effect receipt + linux/fs adapter；Application 可保留 review command/result |
| `executive/src/application/admin_service.rs` | health、approval cache、skills、profile、deploy rollback、background workers；内部还有 SQLite | 一个 facade 聚合大量不相干 owner | 拆成窄 Application use case；host/runtime/extension admin 各自 port；所有 concrete store 外移 |
| `executive/src/application/inference_port.rs`、`harness_factory.rs` | Provider/Cognit construction | Inference port owner 放错层，Application 可直达模型 | `cognit::InferencePort` + provider adapter；Application 零 provider API |

额外结构证据：`executive/src/application/mod.rs` 公开超过 70 个 module；`executive/src/lib.rs` 又通过 `runtime`、`testing` 和根级 facade 重导出 Application 与 concrete adapter。只改 crate 名无法建立真正边界。

## 4. 目标 Application public surface

### 4.1 第一阶段唯一保留的通用用例

```text
SubmitTurn
ContinueTurn
CancelTurn
CreateSession
ResumeSession
ForkSession
ListSessions
GetSession
GetTurn
SpawnDelegate
WaitDelegate
SendDelegate
IngestExternalStimulus
CreateGoalDraft
OpenApproval
ResolveApproval
GetRuntimeProjection
```

上述名称表示用户意图，不表示 Application 获得对应 aggregate 的写权限：

- Session/Turn/Delegate command 调 Runtime；ID 由 Runtime 在 command boundary 内分配并写 journal。
- `CreateGoalDraft` 只创建未执行草稿；把草稿提交为 Agent work 必须形成新的 Runtime command。
- `OpenApproval` 只能关联 owner 已持久化的 `DecisionRequestId`，不能自己伪造 authority challenge。
- `ResolveApproval` 保存人类 resolution，并通知原 owner `ResumeWithDecision`；Kernel/Dasein 必须重新验证 evidence 并在内部 mint opaque single-use grant。
- `Get*` 只读 Runtime query/projection port，不直接读 Runtime journal 表。

### 4.2 Optional application packages

现有 admin、workflow、extension management、evaluation/review 并非核心启动前提。若产品仍需要，分别形成可拔插 package：

```text
application                 # 上述最小通用用例
application-extension-admin # package inspect/install/activate use cases
application-review          # human review/rollback intent，不执行 FS effect
application-operations      # host health/admin query，不持有 worker
```

它们不允许反向进入 `runtime` 或 `kernel`，也不能借 `ApplicationServices` 重新合并成一个全局 service bag。未验证有生产调用者的 workflow/evaluation facade 先冻结；完成调用者 census 后选择迁移或删除，不因“以前能编译”自动进入新核心。

### 4.3 建议的 crate 物理结构

```text
crates/application/src/
├── lib.rs
├── error.rs
├── session_use_cases.rs
├── turn_use_cases.rs
├── delegate_use_cases.rs
├── stimulus_use_cases.rs
├── goal_draft.rs
├── approval/
│   ├── aggregate.rs
│   ├── use_cases.rs
│   └── store.rs             # trait only
└── ports/
    ├── runtime_commands.rs
    ├── runtime_queries.rs
    ├── approval_store.rs
    └── stimulus_sink.rs

crates/adapters/sqlite/src/
├── application_approval_store.rs
├── runtime_journal.rs
├── kernel_execution_journal.rs
└── migration_bundle.rs

crates/adapters/linux/src/
├── workspace_identity.rs
├── trust_store.rs
├── filesystem_effect.rs
└── process_controller.rs
```

Port 由保护不变量的一侧定义：`ApprovalStore` 归 Application；`RuntimeJournal` 归 Runtime；`ExecutionJournal` 和 `ProcessController` 归 Kernel。不能为了复用 SQLite connection 把这些 port 统一放进 `contracts`。

## 5. 持久化拆分规则

### 5.1 每张表只有一个写 owner

迁移前先提交 Storage Topology Inventory，至少记录：

| 字段 | 必须回答的问题 |
|---|---|
| database path | 实际部署路径、权限和备份单元是什么 |
| table/stream | authority journal、outbox、receipt 还是可重建 projection |
| writer process | 哪个 PID/daemon 是唯一生产 writer |
| owner crate | 谁定义状态转换和 schema |
| migration bundle | bundle 版本、checksum、前置版本、失败策略 |
| compatibility | 旧 binary 可读/可写范围和删除期限 |
| recovery | crash 后由谁 replay/reconcile |
| sensitive data | SecretRef、PII、redaction 和 retention |

共享 `objectives.db` 当前同时承载 Goal、Approval、Gmail/Google 相关状态。目标不是立即把文件拆成多个数据库，而是先把表级写 owner 和 migration bundle 拆开；物理拆库只能在 snapshot/restore、foreign-key 和 old-binary compatibility 都证明后进行。

### 5.2 migration 与 repository 生命周期

- repository `new/try_new/open` 不执行 migration；`open` 只校验兼容版本并打开资源。
- 每个 owner 提供 versioned bundle 和 checksum；只有 `aletheon migrate` 按 manifest 顺序执行。
- daemon serving 前只运行 compatibility check，不兼容就 fail closed。
- cutover 期间绝不允许 old/new 双 writer；允许 shadow read/compare，但 shadow reader 不生成 ID、不更新 projection、不修复数据。
- projection 可重建；authority journal、effect receipt、approval resolution 和 outbox 不可当 cache 删除。
- SQL error 映射为 adapter typed error，再由 owner 映射为 domain/Application error；禁止压成裸 `String`。

### 5.3 旧数据迁移顺序

```text
冻结新写入并进入 maintenance mode
-> 停止 Gmail/GBrain/worker 与外部 effect
-> 确认唯一 writer 和无 in-flight operation
-> 备份 DB + migration manifest + receipts/outbox
-> 对副本执行 migrate --check
-> 应用 owner bundle
-> 新 adapter shadow read 校验
-> 切换唯一 writer
-> installed smoke
-> 保留可恢复 checkpoint
```

发生过 Gmail、文件系统、GBrain、Robot 等真实副作用后，不恢复到会丢失新 receipt/outbox 的旧快照；回滚 binary/config，保留 forward-compatible state 并 reconcile。

## 6. 分 PR 执行顺序

每个 PR 从最新 `dev` 建短生命周期分支；只迁移一个 owner 或一条纵向路径。不得把下列 PR 合并为一个“大移动”。

### APX-00：Application 与 Storage census（只加清单和 gate）

工作：

- 生成 `executive/src/application` 每个 production module 的 `keep/move/delete` 清单；
- 生成 table/stream/writer/migration/reader inventory；
- 记录所有 Application public symbol 的真实 production caller；
- 为 `rusqlite`、FS、process、network import 建 non-increasing baseline；
- 给每个 compatibility read/write 标注 owner、计数器和删除 PR。

验收：新增 module/table/caller 不能绕过 inventory；无行为变化。回滚只需删除清单/gate。

### APX-01：建立最小 Application crate 与 Runtime command/query ports

前置：Runtime 已提供 canonical Session/Turn/Delegate command/query facade。

工作：

- 新建纯 Rust Application types、typed errors 和用例；
- 实现 `Create/Resume/Fork/List/Get Session` 与 Turn/Delegate facade；
- 旧 Executive handler 通过单向 adapter 调新 Application；
- Application 不持有 Runtime repository，也不 mint core ID。

验收：内存 fake Runtime port 可验证 command forwarding；依赖图中 Application 只依赖 `contracts`、`runtime`；旧生产 writer 未切换。回滚恢复旧 handler route，不涉及 schema。

### APX-02：Approval aggregate/store port 与 SQLite adapter

工作：

- 从 `approval_service.rs`/`approval/repository.rs` 提取 aggregate、use cases、`ApprovalStore` trait；
- 提取 SQLite repository 和对应 migration bundle；
- 明确 `DecisionRequestId` 来源验证、resolution evidence、expiry、nonce、single-use 状态；
- Gateway route 改调 Application；Application 通过 typed `ResumeWithDecision` command 把 resolution evidence 关联给原 challenge owner。Kernel/Dasein core 不反向依赖 Application，也不直接读取 `ApprovalStore`；它们调用各自拥有的 verifier port。该 port 的 outer adapter 可以查询 Application `ApprovalStore`/Owner evidence，但只返回经过 digest、principal、scope、revision/generation、expiry、nonce 与 single-use 校验的 opaque proof。

验收：伪造/过期/错 principal/错 scope/重复消费 evidence 均 fail closed；旧表 shadow read 与新 adapter 一致；只有一个 writer。回滚切回旧 facade，保留兼容 schema 和全部 resolution。

### APX-03：Goal Draft 与 External Stimulus 纵向切换

工作：

- 把 `CreateGoalDraft`、`IngestExternalStimulus` 提取为 provider-neutral 用例；
- Gmail/Google 专名和 OAuth/transport 留在 extension adapter；
- Goal attempt、worker、budget、verification 不复制进 Application；依 Runtime/extension owner 的迁移结果选择 move/delete；
- 将共享 `objectives.db` 表按 owner bundle 管理。

验收：关闭 Gmail 时用例仍可接受 generic stimulus；关闭 Application 时 Runtime 不受影响；Gmail preservation smoke 仍通过。回滚保留旧 Gmail facade 单向委托，不重放已发送消息。

### APX-04：文件系统、SQLite、进程和 host admin adapter 外移

工作：

- 拆 `thread_authority`、`workspace_trust`、`storage_quota`、checkpoint/review、extension package store、workflow store 和 admin cache；
- use case 只依赖窄 port；canonicalize、lock、metadata、archive、SQLite、process/systemd 进入对应 adapter；
- 删除 Application 对 `rusqlite`、`std/tokio::fs`、`std/tokio::process`、`nix`、`reqwest`、`UnixStream` 的 production import；
- 把 background worker lifecycle 移给 composition 返回的显式 handle。

验收：Application dependency/import gate 为零；每个 adapter 有 owner contract 与故障分类；rollback 可逐 adapter 切回旧单向 facade。

### APX-05：Application caller drain 与自身 facade 删除

前置按待删文件族核验，不把无关 owner 串成全局等待：

| 待删误归文件族 | 精确 owner gate |
|---|---|
| Turn/Agent/Session/Delegate authority、scheduler、worker | `RA-03/RA-04/RA-05` 对应 authoritative seam；本行 caller 清零后再允许 `RA-06` final deletion |
| Dasein/Metacog/Agora/Mnemosyne/Cognit rich implementation | `D2/D3/D4/D5` 对应 owner cutover；本行 caller 清零后再允许 `D6` |
| capability/admission/process/settlement enforcement | `K4/K6` authoritative enforcement seam；本行 caller 清零后再允许 `K7` |
| Hardware/embodiment/Robot bridge | `E4-K6b/E5-K6c` 对应保存舱与 cutover |
| extension concrete adapter/bootstrap/config/schema writer | 对应 `E2-K6a/E3/E4-K6b/E5-K6c/E6-K6d` authoritative cutover，或 `C0/E0` 已证明不保留；本行 caller-zero evidence 交 matching owner 与 `XRET-03` 唯一物理删除 |
| extension-specific Fabric rich row/re-export | matching extension cutover + `XRET-03` drain；本行 caller-zero evidence 交 `E7` 唯一删除 |
| Application 自有 SQLite/FS/host 混合实现 | `APX-04` adapter/writer cutover |

一个文件只等待自身涉及的 owner gates；跨 owner 文件必须先 SPLIT，再分别满足对应 gate。未知 caller/writer 仍按 `APX-00/C0` 阻塞，不能因为其他 family 已完成就批量删除。

工作：

- 对 Turn/Agent/Session、domain、Kernel、Hardware/Robot 等非 Application owner 文件，只切断 Application caller/import、迁窄 command/query port，并产出 caller-zero/deletion evidence；Runtime/domain/Kernel 旧物理实现分别唯一由 `RA-06/D6/K7` 删除，extension concrete implementation 由 matching E2..E6 + `XRET-03` 删除，只有 extension-specific Fabric row/re-export 由 `E7` 删除；APX-05 不抢 deletion owner；
- 只删除真正由 Application 拥有且已无 production caller 的 optional workflow/evaluation/admin facade、Application compatibility branch 与 re-export；
- 清理 `application/mod.rs` 和 Executive 根级中仅属于 Application facade 的重导出；跨 owner module 先 SPLIT，不能用删除 facade 顺带删除 owner implementation；
- 每个非 Application family 的 evidence 精确绑定其后续 owner deletion PR，禁止 APX-05 与该 PR 各删一次同一文件。

验收：Application 对非 owner 实现的 caller/import 为零；Application-owned dead facade 已删除；每个其余旧文件恰好有一个 `RA-06/D6/K7`、matching `E2..E6+XRET-03` 或 `E7`（仅 Fabric extension row/re-export）物理删除 owner，且对应计划能消费本阶段 evidence。回滚只恢复窄 Application facade，不恢复第二 authority、writer 或跨层 import。

验收：Application public surface 与 4.1/4.2 清单一致；没有第二 writer/state machine；Executive 只剩未迁移 owner 的模块和机械 facade。回滚仅恢复最近一个 owner facade，不恢复双 writer。

## 7. 防止新 God Object 的结构 gate

从 APX-01 开始逐步 ratchet：

- Application production crate 不得依赖 `rusqlite`、`reqwest`、`tonic`、`nix` 或启用 Tokio `process/net/fs`；
- source 中不得出现 concrete adapter path、Provider/Gmail/Robot/Pi/GBrain 名称；
- 一个 use-case service 最多持有 5 个显式依赖；禁止 `ApplicationServices`、`ServiceBag`、跨领域 getter；
- `Service` 只表示 Application facade；store implementation 必须以所实现 port 命名；
- 普通 production 文件目标不超过 500 行、函数不超过 60 行；超限必须说明减少了哪个 authority/I/O/state/lifecycle 责任；
- `new` 纯内存且不可失败；资源使用 `open`，装配使用 `compose`，启动 worker 使用 `start/bootstrap`；
- 禁止 crate-level `#![allow(clippy::...)]`；例外局部、有 owner 和删除 PR；
- `rg` 验证 Application 无 `TurnId::new/SessionId::new/AgentId::new`；Application-owned `ApprovalId` 除外；
- Runtime、Kernel 和领域 crate 对 Application 的反向依赖为零。

## 8. 验收矩阵

不以 workspace 全量测试数量作为成功标准。每个纵向 PR 提供最小但权威的证据：

| 层级 | 必须证明 |
|---|---|
| compile/dependency | 目标 package 经 `scripts/cargo-agent.sh` 的窄 check；依赖和 forbidden-import gate 通过 |
| use-case contract | command 只转发一次；query 不写 authority；typed error 保留 retry/reconcile category |
| persistence | migration checksum、copy 上的 `migrate --check`、唯一 writer、restart replay/reconcile |
| approval | challenge 先由 owner 持久化；错误 evidence fail closed；single-use 生效 |
| session/turn | Application 不 mint ID、不写 Runtime journal、不从 projection 推断 terminal |
| optionality | core Runtime 无 Application composition 仍可启动；Application 开启时 basic use cases 可用 |
| installed behavior | 涉及 socket、persistence、daemon、client 或扩展行为的 PR 按仓库规则完成 system-installed smoke 与 binary/PID provenance |

最终需分别验证 `core-linux` 与 `full-linux`；Gmail/Pi/GBrain/Robot/Hardware 使用各自 preservation gate，不能用一个“daemon 启动成功”替代。

## 9. 回滚原则

- 每个 PR 保留至多一个、只向新 owner 委托的旧 facade；下一 owner PR 验收后立即删除。
- facade 不得持有独立 store、cache、worker、ID generator、policy fallback 或第二 writer。
- schema 采用 expand/verify/cutover/contract；contract/drop column 单独延后到 old binary 不再支持之后。
- 回滚优先切 binary/config 和调用 facade；绝不同时启动旧、新 writer。
- 已产生外部 effect 时，以 receipt/outbox reconcile，不回滚掉 effect evidence。
- 任何 shadow mismatch、writer 冲突、authority ID 来源不明或 recovery 无法解释，都停止切换并回到最近可服务版本。

## 10. 完成定义

- `crates/application` 只包含第 4 节允许的用例、aggregate、port 和 typed error。
- Application 的 resolved dependency graph 无 SQL、FS、process、network、Provider 或 platform SDK。
- Agent/Session/Turn/Self/Metacog/Memory/Workspace/Hardware authority 均不在 Application。
- 每个持久化表/stream 有唯一 owner、writer、bundle、恢复与回滚记录。
- Application 可从 composition 拔掉，核心仍能运行；启用时所有入口使用同一 Runtime。
- `executive/src/application` 不再有生产逻辑，且其每个原模块已在负责迁移的 PR 内删除或有明确、短期、可计数的 compatibility seam。

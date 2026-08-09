# Agent Kernel V2：DeepSeek 文件级实施计划

> 状态：Executable Handoff Plan
>
> 计划基线：PR #192 `80358c975f15252139f0983dd3709cfe00e8fd51`
>
> 目标分支：从合入 PR #192 的最新 `dev` 创建短分支
>
> 上位约束：[`Agent Kernel V2 完整架构计划`](./2026-08-08-agent-kernel-v2-complete-rearchitecture.md)
>
> Slice 规范：[`Executive 拆解索引与 canonical crosswalk`](./2026-08-08-executive-decomposition-plan-series.md)
>
> 执行协议：[`DeepSeek 重构执行手册`](./2026-08-09-deepseek-rearchitecture-execution-runbook.md)

## 0. 这份文件解决什么

现有文档已经定义完整架构、owner、DAG、迁移安全和逐文件账本，但不适合直接把整套文档作为一条模型任务。本文件只做三件事：

1. 把首批 evidence 与 owner-seam slice 写成可直接领取的文件级任务包；
2. 固定每个任务的输入、允许改动、禁止改动、产物、命令和停止条件；
3. 给后续 writer cutover、扩展迁移和 Executive 退休提供唯一执行顺序，不复制第二份 authority 决策。

本文不改变上位计划的 owner、writer、DAG、删除 owner 或产品范围。若本文与当前源码或上位计划冲突，按执行手册 §2 的事实源优先级处理，并停止生产代码修改。

## 1. 当前代码事实与目标差距

领取任何任务前必须在当前 checkout 重新验证以下 locator；它们只是 PR #192 基线事实，不是永久真相。

| 主题 | 当前代码事实 | 目标 | 首个 owner slice |
|---|---|---|---|
| Runtime surface | `crates/runtime/src/lib.rs:1-13` 只导出 manifest/selector | Agent/Session/Turn/Delegate semantic owner | `RA-01` |
| Session authority | `crates/executive/src/application/session_service.rs:43` 的 `SessionService` 同时持 append store、active turn、interrupt 和 protocol SQLite | Runtime `SessionAuthority`；Gateway 持 client event log | `RA-00 -> RA-02 -> RA-03` |
| Turn authority | `crates/executive/src/application/turn_coordinator.rs:290` 的 `TurnCoordinator` 仍创建/推进 Turn | Runtime canonical Turn reducer/writer | `RA-00 -> RA-02 -> RA-04` |
| Agent authority | `crates/executive/src/application/agent_control/mod.rs:120` 的 `AgentControlService` 持 repository、registry、live runs、tasks、mailbox 和 settlement | Runtime `AgentSupervisor` | `RA-00 -> RA-05` |
| Delegate registries | `crates/executive/src/core/runtime_registry.rs:11` 与 `crates/executive/src/application/agent_control/execution.rs:435` 并存 | 一个 `DelegateBackendRegistry` | `RA-00 -> RA-05 -> E6-K6d` |
| Profile loader | `crates/executive/src/composition/agent_loader/mod.rs:49` 仍叫裸 `AgentLoader`，由 `crates/executive/src/host/daemon/bootstrap/runtime.rs:11-120` 使用 | Runtime profile port + 精确命名 loader | `RA-00 -> RA-05` |
| Kernel surface | `crates/kernel/src/lib.rs:1-12` 暴露 process/operation/space/supervision 与 `KernelRuntime` | 小型独立 enforcement crate | `K0 -> K1 -> K2` |
| Gateway | `crates/gateway/src/lib.rs:1-34` 仍包含 concrete Telegram/SQLite facade | protocol/client/server 与 adapter 分离 | `CGP-00 -> CGP-02/03` |
| TUI boundary | `crates/interact/src/lib.rs:1-23` 有 crate-level blanket allow；`crates/interact/Cargo.toml:11` 依赖 Executive | 只依赖 typed Gateway client/protocol | `CGP-00 -> CGP-06/07` |
| Composition | `crates/aletheon/Cargo.toml:18` 仍通过 Executive 取得核心实现 | `aletheon` 唯一 concrete composition root | `CGP-00 -> CGP-01` |

PR #192 已删除的 `core/session.rs`、`application/agent/**` 和 `composition/agents/**` 不得恢复。它们的 monotonic gate 与外部 consumer 证据由 foundation checkpoint 管理；它不是 `RA-00` 完成证明。

## 2. 总执行 DAG

```text
PR #192 foundation
        |
        v
RA-00 ----------------------------------------------------+
  |                                                       |
  +--> K0 --> K1 --> K2 ----------------------------------+--> K3/K4/K5
  +--> APX-00 --> APX-01 ---------------------------------+--> APX-02/03/04
  +--> CGP-00 --> CGP-01 + CGP-02 ------------------------+--> CGP-03/04
  +--> E0 --> E1 -----------------------------------------+--> extension cutovers
  +--> D0 --(B2/B4 unknown = 0)--> D1 --------------------+--> D2/D3/D4/D5
  |
  +--> RA-01 --> RA-02 --> RA-03 --> RA-04 --> RA-05
                                              |
                                              v
                                           E6-K6d
                                              |
                                              v
                                            RA-06

matching owner caller-zero
        -> CGP/APX/K/D/E cleanup
        -> E7
        -> D6
        -> AK2-25 fabric -> contracts mechanical rename
        -> XRET-04
        -> XRET-05 delete empty Executive
```

调度规则：

- 一次只把一个 task packet 交给 DeepSeek；完成报告经人工审阅后才领取下一个。
- `RA-00` 是第一个 canonical task；其他 Phase 0 census 可在独立分支完成，但不得与 `RA-00` 共用写文件。
- `RA-02 -> RA-05` 是严格 writer 主链，禁止并行切换。
- `K6` 只提供 enforcement seam；扩展 writer/executor 切换仍由相应 E-series owner 执行。
- `D6`、`E7`、`AK2-25` 不得并行修改 Fabric root。

## 3. 所有任务共同协议

### 3.1 开始前固定输出

实现者开始修改前必须输出并保存：

```text
Slice:
Baseline commit:
Plan revision:
Direct prerequisites:
Current authoritative writer:
Target owner/writer:
IDs minted here:
Production callers:
Test-only callers:
Installed/config callers:
Tables/files/wire schemas:
External side effects:
Compatibility seam:
Deletion owner:
Unknowns/blockers:
Expected files:
Out-of-scope files:
```

任一字段不能回答时，状态仍为 `EVIDENCE_OPEN`，不得创建 owner API 或修改生产行为。

### 3.2 允许的修改类型

Evidence task 只允许：

- `config/architecture/*.tsv|*.txt|*.env` 中的 census 和只减不增 baseline；
- `docs/arch/evidence/*.md` 中的命令、计数、结论与 blocker；
- `scripts/libexec/aletheon/architecture-check.sh` 中的 monotonic gate；
- 对已证明 workspace caller/re-export/config/installed caller 全部为零的幽灵代码做独立删除提交。

Owner-seam task 只允许：

- owner-local command/event/query/value/error；
- 最小 port 与 adapter skeleton；
- 显式 composition handle；
- 不执行 effect 的 read-only shadow；
- 相应 dependency/symbol/size gate。

### 3.3 统一禁止项

- 不恢复已退休 path/symbol；
- 不将 Executive 文件整包复制到 Runtime/Kernel/Application；
- 不在一个 PR 切两个 aggregate writer；
- 不新增 service bag、component getter bag 或第二 composition root；
- 不让 adapter、TUI、Gateway、Pi 或 caller mint canonical Agent/Session/Turn ID；
- 不以 feature flag、re-export 或 fallback-on-any-error 永久隐藏反向依赖；
- 不用 notification/projection/cache 代替 authoritative journal；
- 不通过错误字符串判断 provider failure、policy denial 或 terminal；
- 不删除受支持 Gmail/GBrain/Pi/Robot/Hardware path 来制造架构通过；
- 不运行裸 `cargo`，统一使用 `bash scripts/cargo-agent.sh ...`。

## 4. Task Packet F0：foundation checkpoint

状态：`DONE`，不得重复删除。

### 4.1 已完成内容

- `core/session.rs`、旧 application `AgentRuntime`、第二 profile loader caller-zero 删除；
- Executive layer inventory 与 architecture metrics 下调；
- 退休 authority path/symbol monotonic gate；
- crates.io/GitHub indexed public consumer 检查及其限制说明。

### 4.2 复核文件

- `config/architecture/retired-authorities.tsv`；
- `docs/arch/evidence/2026-08-09-agent-kernel-v2-foundation-checkpoint.md`；
- `scripts/libexec/aletheon/architecture-check.sh` retired-authority block。

### 4.3 退出条件

- architecture acceptance 通过；
- broken symlink、空 retired root、private alias 和跨 crate symbol mutation probe 均被拒绝；
- 不改变任何现任 writer、schema、daemon 或 client 行为。

## 5. Task Packet RA-00：Runtime authority census

### 5.1 目标

关闭 Agent/AgentRun/Session/Turn/Delegate 的 constructor、ID mint、writer、journal、table、registry、composition、Native/Pi caller 证据。只建立 inventory 与只减不增 gate，不修改 writer。

### 5.2 必读文件

- `docs/plans/2026-08-08-runtime-authority-consolidation.md:26-179,216-280`；
- `docs/plans/2026-08-08-executive-source-disposition-ledger.md` 中所有包含 `RA-00` 的行；
- `config/architecture/persistence-surfaces.tsv`；
- `config/architecture/state-machine-inventory.tsv`；
- `config/architecture/id-collisions.tsv`；
- `crates/runtime/src/{lib.rs,manifest.rs,selector.rs}`；
- `crates/executive/src/application/{session_service.rs,turn_coordinator.rs,turn_pipeline.rs,agent_control/**}`；
- `crates/executive/src/core/{runtime_registry.rs,sub_agent.rs}`；
- `crates/executive/src/composition/{turn_coordinator.rs,turn_service.rs,agent_loader/mod.rs}`；
- `crates/executive/src/host/daemon/bootstrap/{agents.rs,runtime.rs,sessions.rs,turn_runtime.rs}`；
- `crates/interact/src/{single_message.rs,intent.rs,tui/**}`；
- `crates/aletheon/src/{main.rs,lib.rs,acp.rs}`。

### 5.3 必交产物

新增：

```text
config/architecture/runtime-authority-census.tsv
docs/arch/evidence/2026-08-09-ra-00-runtime-authority-census.md
```

TSV 固定列：

```text
fact_id
aggregate
role
path
symbol
current_owner
authority_kind
production_callers
test_callers
installed_config_callers
storage_or_wire
current_writer
target_owner
cutover_slice
deletion_slice
status
evidence_command
```

`role` 只允许：

```text
constructor | id_mint | state_machine | journal_writer | projection_writer
registry | composition | compatibility | cache | transport | reader
```

`authority_kind` 只允许：

```text
authority | projection | cache | transport | compatibility | dead
```

`status` 只允许 `CLOSED` 或 `INVESTIGATE:<blocker-id>`；不得用空白、`unknown` 或散文代替 blocker。

### 5.4 证据命令

先生成候选，再逐个读 declaration 和 production caller。测试、`#[cfg(test)]`、examples、docs 与 public re-export 必须单列：

```bash
rg -n '\b(AgentId|AgentRunId|SessionId|TurnId)::(new|default)\b|\b(SessionId|TurnId|AgentId)\s*\(' \
  crates -g '*.rs'

rg -n '\b(create_session|fork_session|start_turn|spawn_agent|spawn_delegate|new_session)\b' \
  crates -g '*.rs'

rg -n '^\s*(pub\s+)?(struct|enum|trait|type)\s+.*(Session|Turn|Agent|RuntimeRegistry)' \
  crates -g '*.rs'

rg -n '\b(SessionCreated|TurnAccepted|TurnStarted|TurnSettled|TurnSettlement|AgentTerminal|AgentRunStatus)\b' \
  crates -g '*.rs'

rg -n '\b(INSERT|UPDATE|DELETE)\s+(INTO\s+)?(sessions|session_|turn_|agent_|protocol_events)' \
  crates -g '*.rs' -i

rg -n '\b(EventSpine|CanonicalEventBus|SessionAppendStore|AgentRunRepository|RuntimeRegistry|AgentRuntimeRegistry)\b' \
  crates -g '*.rs'

rg -n 'runtime|agent|session|turn|pi' \
  config scripts crates/aletheon crates/executive/src/host -g '*.toml' -g '*.json' -g '*.yaml' -g '*.yml' -g '*.rs' -g '*.sh'
```

最终 gate 不得简单禁止所有 `::new()`；它必须按 owner、production scope 和允许的 fixture 分类。

### 5.5 必须回答的问题

1. 谁现在 mint Agent、AgentRun、Session、Turn 和 Delegate identity？
2. 每个 identity 是 canonical、legacy alias、IPC endpoint 还是 UI correlation？
3. 谁 append Session/Turn/Agent terminal？
4. `SessionService.protocol_events` 是 authoritative 还是 Gateway reconnect log？
5. `EventSpine`、Session append store 和 AgentRun repository 分别写什么 stream/table？
6. 两个 runtime registry 的 production 构造点与间接 Goal/Pi caller 各有多少？
7. Native 与 Pi 的 spawn/wait/cancel/restart 入口分别是什么？
8. production loader、package loader、runtime manifest loader 是否语义相同？
9. installed profile/config/schema 是否仍引用已退休 symbol/path？
10. 哪些旧测试属于 `KEEP-EVIDENCE`、`REPLACE`、`DELETE-LATER`？

### 5.6 Gate 要求

- 新增未登记 constructor、mint、writer、registry、Session-shaped type 时失败；
- 已登记计数只能下降；
- inventory path/symbol 必须存在；absence row 必须有专用零匹配命令；
- test-only symbol 不得被误计为 production authority；
- public re-export 不得被误计为 production caller；
- `INVESTIGATE` row 必须有 canonical owner、关闭命令和阻塞的后续 slice。

### 5.7 验证

```bash
git diff --check
bash -n scripts/libexec/aletheon/architecture-check.sh
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/cargo-agent.sh check -p runtime
bash scripts/cargo-agent.sh check -p executive --lib
```

RA-00 不改 Rust production behavior；如只改 inventory/gate，不要求安装态部署。退出时必须按执行手册 §17 报告并停止。

## 6. Task Packet K0：effect/operation/process census

### 6.1 目标与文件

围绕以下入口记录 admission、authorization input、executor、receipt、recovery 和真实副作用：

- `crates/kernel/src/{runtime.rs,operation/**,process/**,capability/**,space/**,supervision/**}`；
- `crates/executive/src/application/{governed_capability.rs,embodied_execution_adapter.rs,agent_control/**}`；
- `crates/executive/src/adapters/runtime/**`；
- `crates/executive/src/adapters/runtime/{pi.rs,pi_protocol.rs,pi_rpc.rs,process_supervisor.rs,worktree_recovery.rs}`；
- `crates/execd/src/**`；
- process/file/git/network/Gmail/Pi/Hardware/Robot executor 入口。

### 6.2 产物

```text
config/architecture/kernel-effect-census.tsv
docs/arch/evidence/2026-08-09-k0-kernel-effect-census.md
```

每行至少记录：effect family、command owner、current executor、authorization input、operation/process ID owner、receipt、timeout/cancel、restart recovery、external irreversibility、target K/E slice。

### 6.3 停止条件

- 找到绕过 Kernel 的 production effect，但无法确认业务 owner；
- current success 不依赖 authoritative receipt；
- real Hardware/Robot path 缺 safety veto/freshness evidence；
- 需要先改变 Session/Turn writer 才能描述 effect。

## 7. Task Packet APX-00：Application 与 I/O census

### 7.1 目标与文件

分类 Executive Application use case、optional package、dead workflow，以及 Application 内 SQLite/filesystem/process/network concrete I/O：

- `crates/executive/src/application/**`；
- `crates/executive/src/adapters/**`；
- `crates/executive/src/host/daemon/handler/**`；
- `crates/executive/src/composition/**`；
- `crates/executive/tests/**` 中每个 public use-case 的唯一行为证据。

### 7.2 分类

```text
basic-use-case | optional-supported | optional-unproven | adapter-io | compatibility | dead
```

`optional-unproven` 必须同时给出 production caller 搜索、installed config/data 搜索、目标 owner 和关闭命令；不能直接重建到新 Application。

### 7.3 产物与退出

```text
config/architecture/application-use-case-census.tsv
docs/arch/evidence/2026-08-09-apx-00-application-io-census.md
```

只有 basic use cases、Approval、GoalDraft/external stimulus 与真实 supported optional package 定案后，才允许 `APX-01` 创建最小 Application surface。

## 8. Task Packet CGP-00：composition/Gateway/Interact census

### 8.1 目标与文件

- `crates/aletheon/src/**` 的 CLI、daemon、ACP 和 composition；
- `crates/executive/src/host/**` 的 socket/RPC/bootstrap；
- `crates/executive/src/composition/**`；
- `crates/gateway/src/**`；
- `crates/interact/src/{host,intent,acp,tui}/**`；
- `docs/plans/2026-08-09-interact-authority-census.md` 的 56/56 baseline。

记录每个 route 的 authentication source、request DTO、command owner、response/error mapping、wire schema、socket/framing owner、retry/reconnect、Session/Turn ID mint、effective policy/TaskKind 推导和 deletion slice。

### 8.2 产物

```text
config/architecture/gateway-route-census.tsv
config/architecture/composition-root-census.tsv
docs/arch/evidence/2026-08-09-cgp-00-composition-gateway-census.md
```

Interact 56 个文件必须与 baseline 集合机械相等。任何 TUI/local overlay truth 必须分类为 presentation；任何 authority claim 必须登记迁移 owner。

## 9. Task Packet E0：受支持扩展 preservation manifest

### 9.1 每个扩展单独记录

```text
Gmail/Google
GBrain
Hardware
Robot VLA
Pi
```

每项记录：production caller、feature/config、installed state、credential reference、schema/table/file、worker/process、external effect、cancel/restart、current equivalence oracle、safe smoke、target owner、cutover slice、rollback blocker。

### 9.2 产物

```text
config/architecture/extension-preservation.tsv
docs/arch/evidence/2026-08-09-e0-extension-preservation.md
```

禁止 destructive smoke。Gmail/GBrain/Pi 使用 installed config 时只能执行明确无破坏路径；Hardware/Robot 保持 simulator/replay，真实执行继续 gated。

## 10. Task Packet D0：Fabric owner/caller/codec census closure

### 10.1 输入

- `docs/plans/2026-08-09-fabric-source-disposition-ledger.md` 174/174 文件；
- `config/architecture/fabric-public-types.tsv` public surface；
- `config/architecture/{wire-surfaces.tsv,id-collisions.tsv,contract-migrations.tsv}`；
- Fabric root re-export、codec、schema reader/writer 与 extension callers。

### 10.2 关闭 B2/B4 unknown

每个 public type 必须有：domain owner、codec owner、production readers、writers/constructors、wire/storage compatibility、MOVE/SPLIT/DELETE/KEEP decision、target slice、re-export deletion owner。

无法定案的 rich type 保持 Fabric，不得进入 D1 Contracts seed。D1 只允许已经证明 ownerless 的 ID wrapper、digest、bounded scalar、wire-safe reference 和 version primitive。

### 10.3 退出条件

- 174/174 文件集合与源码一致；
- public type snapshot 与实际导出一致；
- B2/B4 unknown 为零；
- extension-specific row 不被 D6 抢占；
- `fabric -> contracts` rename 尚未开始。

## 11. Wave B：owner seam 建立顺序

Phase 0 人工审阅通过后，按以下顺序逐 PR 执行。

### 11.1 D1：最小 Contracts seed

- 文件：先在现有 Fabric 内建立受 gate 约束的 ownerless primitive module；是否创建新 crate 以 canonical crosswalk 为准；
- 不搬 rich aggregate/repository/service/workflow；
- 同一提交不得做 package rename；
- 验证 Contracts seed 无 workspace dependency，Fabric public rich surface 不增长。

### 11.2 RA-01：Runtime commands/events/queries/ID owner

目标文件：

```text
crates/runtime/src/command.rs
crates/runtime/src/event.rs
crates/runtime/src/query.rs
crates/runtime/src/ids.rs
crates/runtime/src/error.rs
crates/runtime/src/ports.rs
crates/runtime/src/lib.rs
```

规则：

- `CreateSessionCommand`、`StartTurnCommand`、`SpawnAgentRunCommand` 不接受新 aggregate ID；
- caller correlation 与 `LegacySessionAlias` 不具有 authority；
- ID source 只在 Runtime composition 中构造；
- 只定义 contract/port，不 append、不 spawn、不切旧 writer；
- `UiOverlayId` 不进入 Runtime。

### 11.3 K1/K2：operation seam 与 sealed executor

- K1 只建立 durable Operation authority/ports；
- K2 建立 sealed descriptor、registry revision、invocation digest 与 `CapabilityExecutor`；
- Kernel 不读取 Session/Self/prompt/provider/Robot 语义；
- bootstrap seal 后不可替换 descriptor-to-executor binding；
- 旧 effect path 仍 authoritative，new path 不执行副作用。

### 11.4 CGP-01/02：composition skeleton 与 typed Gateway

- `aletheon` 只接显式 component handle，不接 Executive service bag；
- Gateway protocol/client 与 server/concrete transport 分离；
- ACP 外层 framing/correlation 暂留 ACP owner；
- TUI 尚未切流时不删除 legacy route；
- client command 不携带 effective principal/permission。

### 11.5 APX-01/E1

- APX-01 只建立 basic use-case facade 和 owner ports；
- E1 只建立 extension registration descriptor/port；
- 两者都不构造 concrete adapter，不创建第二 composition root，不切 writer。

## 12. Wave C：Runtime writer 主链

### 12.1 RA-02 RuntimeJournal shadow

- 适配现有 Session append/EventSpine/AgentRun physical schema；
- 分离 Agent/Session/Turn streams、sequence、schema、generation、digest；
- bounded replay + typed mismatch；
- shadow 禁止 append/spawn/effect；
- unknown event/version fail closed；
- 不做不可逆 migration。

### 12.2 RA-03 Session writer cutover

- maintenance/drain；
- freeze old/new writer、generation、watermark、active sessions/turns；
- Runtime 分配 Session ID，append `SessionCreated` 后返回 receipt；
- SessionManager 改 `ContextWorkingSet`，可从 journal/projection 重建；
- SessionService 只保留单向 facade；legacy store 停写；
- old entry 返回 typed retired/wrong-generation；
- installed list/resume/fork/compact/reconnect 与 rollback binary 验收。

### 12.3 RA-04 Turn writer cutover

- 迁唯一 Turn reducer/terminal fence，不搬 TurnPipeline God Object；
- cognition、context、policy、effect、post-settle 变窄 ports；
- cancel/timeout/disconnect/late receipt/crash 全部走 typed transition；
- valid late effect 只能追加 observation/accounting，不反转 terminal；
- TUI core `TurnId` mint 由 CGP-06 独立清理；
- 正式安装态同 Session 多轮至少一次，model-controlled route/arguments 按仓库政策三次。

### 12.4 RA-05 AgentSupervisor

- 迁 AgentControl lifecycle、mailbox、recovery、settlement 与 AgentStream adapter；
- Runtime 分配 child/generation/run ID；
- 只建立 generic `DelegateBackendRegistry`，不迁 Pi concrete 文件；
- running AgentRun 固定 backend generation；reload 不偷换 binding；
- production Markdown loader 迁 profile port 并精确命名；
- Pi 仍通过单向 legacy backend seam，直到 E6-K6d。

每个 writer PR 必须独立提交、独立部署、独立回滚演练；不能把 RA-03/04/05 合并。

## 13. Wave D-F 执行顺序

### 13.1 扩展联合切换

```text
E2-K6a Gmail/Google
E3     GBrain
E4-K6b Hardware
E5-K6c Robot VLA
E6-K6d Pi
```

每项只消费 E0 manifest 中已经冻结的 equivalence oracle。Kernel seam 不拥有扩展 writer；扩展 PR 是唯一 executor/writer cutover owner。

### 13.2 Presentation/host

```text
CGP-03 typed route handlers
CGP-04 official daemon/socket cutover
CGP-05 ACP typed client
CGP-06 TUI/CLI command-only cutover
CGP-07 TuiModel/TuiController/TuiRenderer/GatewayClient split
CGP-08 host/composition deletion-ready evidence
```

### 13.3 Cleanup/rename/retirement

```text
matching caller-zero
  -> RA-06 / K7 / APX-05 / CGP-08 / E7
  -> D6 closes non-extension Fabric rows/root
  -> AK2-25 pure mechanical fabric -> contracts rename
  -> XRET-04 host/composition/compat remnants delete
  -> XRET-05 empty Executive crate delete
```

没有 caller-zero、installed observation window、rollback-compatible binary 和账本 deletion owner，不得删除。

## 14. 验证矩阵

### 14.1 每个 PR

```bash
git diff --check
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/cargo-agent.sh check -p <affected-crate>
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture
```

按改动增加一个最小 invariant/replay/recovery test，不跑默认 workspace 全量测试。不得并发运行 Executive 或 workspace build。

### 14.2 Writer/schema/daemon/client PR

除 focused check 外，必须：

1. `sudo bash scripts/aletheon.sh deploy`；
2. 比较 `target/release/aletheon`、`/usr/bin/aletheon`、running machine/user daemon executable SHA-256；
3. 观察 systemd restart counter 稳定；
4. 使用 `/usr/bin/aletheon` 和 official user socket 完成真实 LLM request；
5. 检查 rendered frame、session evidence、audit/receipt、daemon logs 一致；
6. provider rejection/unavailable 或 rendered inference error 一律判失败；
7. async child 必须 wait 到 authoritative terminal；
8. 执行 crash/restart/cancel/timeout/duplicate/wrong-generation/reconciliation；
9. 演练可读新 schema 的 rollback binary；无法回退时保持 maintenance 并前向修复。

### 14.3 扩展 PR

- Gmail/GBrain/Pi：installed config 无破坏 smoke；
- Native/Pi：分别验收，不互相替代；
- Robot/Hardware：simulator/replay，真实执行继续 fail closed，除非 HIL/实机 gate 已独立满足；
- secret：不得进入 argv、prompt、event、普通日志或 memory。

## 15. 提交与交付粒度

一个非平凡 slice 建议分为：

```text
1. chore(architecture): freeze <slice> evidence
2. refactor(<owner>): introduce <owner seam>       # owner task only
3. refactor(<owner>): cut over <single writer>     # writer task only
4. refactor(<owner>): retire caller-zero <legacy>  # deletion task only
5. test(<owner>): preserve <named invariant>        # 可与对应阶段合并，但不能掩盖行为提交
```

每个提交正文必须说明问题/方案，并列出具体文件变化。提交前检查 staged diff；不得夹带无关格式化、用户未提交文件或其他 slice。

## 16. DeepSeek 任务投喂模板

首个任务只能这样下发：

```text
执行 Agent Kernel V2 的 RA-00 evidence-only slice。
基线必须包含 PR #192 foundation。先读取 AGENTS.md、本文件 §1-5、
runtime-authority-consolidation.md §2-8、canonical crosswalk 和相关 source ledger 行。
开始修改前提交完整 context receipt。
只产出 runtime-authority-census.tsv、RA-00 evidence report 和 monotonic gates；
不得新增 Runtime owner API，不得改 production behavior/writer/schema，不得开始 RA-01。
测试/文档/re-export/production caller 分开计数，每个 unknown 必须绑定 blocker、
关闭命令和后续 canonical slice。
运行本文件 §5.7 的命令，按执行手册 §17 报告后停止。
```

人工审阅 RA-00 后，再单独下发 K0/APX-00/CGP-00/E0/D0；不得使用“按全部计划完成重构”作为任务。

## 17. 人工审阅清单

每个 DeepSeek 结果至少审阅：

- diff 是否只属于一个 slice；
- current/target writer 是否与代码 locator 一致；
- production/test/re-export/installed caller 是否分开；
- 数据表、wire、migration reader 是否漏项；
- 是否新增第二 ID mint、registry、state machine、terminal 或 executor；
- compatibility 是否单向、有计数、有 deadline、有 deletion owner；
- rollback 是否仍保持单 writer；
- architecture gate 是否变强或 baseline 下降，而不是放宽；
- 验证命令是否真实运行，失败是否如实记录；
- installed-runtime claim 是否包含 binary digest、official socket、restart counter、真实请求和日志/frame/receipt 一致性。

任一项不能回答，任务退回 `EVIDENCE_OPEN` 或 `IMPLEMENTING`，不得领取下一 slice。

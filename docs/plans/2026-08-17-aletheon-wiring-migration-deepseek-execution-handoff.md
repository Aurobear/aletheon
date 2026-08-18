# Aletheon wiring 所有权迁移：DeepSeek 后续执行交接计划

状态：**MIGRATION CLOSEOUT DONE · ARCHITECTURE CONVERGENCE TRACKED — M0–M6、M7.1–7.5、M8.1–M8.5、M7.4 补完、M9（删除 wiring）、M10（最终安装态验收）均已完成并验证；2026-08-18 closeout 完成（门禁假绿修复 / warnings 清理 / M8.4 全收口 / legacy sunset ledger / 公共 API doc(hidden)），closeout binary 已重新部署；最终 M10 验收报告见 `docs/testing/x13-m10-installed-acceptance-2026-08-18.md`，legacy sunset ledger 见 `docs/plans/legacy-session-sunset-ledger.md`；2 项显式收敛残留（完整 pub(crate) 收口、runtime::orchestration 退役）见规范 §9.1**

规范源：`docs/plans/2026-08-16-aletheon-wiring-ownership-migration.md`

本文件是执行交接，不替代规范源。若两者冲突，以规范源及当前代码为准；每次开始新 packet 前必须重新读取对应规范段并重新定位代码。

## 给 DeepSeek 的启动任务包（可直接复制）

```text
工作目录：/home/aurobear/Workspace/aletheon

目标：继续完成 Aletheon wiring ownership migration 的 M6–M10。不要把计划编写、
目录移动、LOC 下降或静态 grep 通过当作最终完成。

开始前必须：
1. 阅读仓库 AGENTS.md；
2. 阅读规范源 docs/plans/2026-08-16-aletheon-wiring-ownership-migration.md；
3. 阅读执行交接 docs/plans/2026-08-17-aletheon-wiring-migration-deepseek-execution-handoff.md；
4. 检查 git status，保护当前大量未提交迁移改动；
5. 运行交接 §1.3 的稳定基线；
6. 从 M6.43 开始，一次只完成一个可验证 packet；禁止跳到 M7；
7. 每次声明完成前给出当前 path:line、行为测试、architecture gate 和剩余项；
8. Cargo 只能通过 bash scripts/cargo-agent.sh；禁止 git reset --hard、git checkout --、
   git clean、git add -A；禁止 old/new 双写或 compatibility alias；
9. 不修改 robot actuator/control loop；
10. 最终只有完成 M10 系统 deploy、binary SHA parity、systemd 稳定窗口、真实 LLM 和
    真实 GBrain recall 后，才允许把规范状态改为 COMPLETE。

当前稳定点：M0–M5 完成；M6 Stage 42 完成；M6 execution envelope 尚未完成；
M7–M10 未开始。先复核代码，不要只相信这句话。
```

## 0. 最终目标、边界与授权

最终目标：完成规范源 M6–M10，使 `crates/aletheon/src/wiring/` 被物理删除，Application/Runtime/adapters/Gateway/host/composition 所有权与依赖方向闭合，并通过系统安装态真实使用验收。

已获授权：

- 可修改 architecture checker、architecture fixtures 和测试；
- 可对 M1 列出的 Turn、Agent、Evaluation 核心文件做纯 owner/import cutover；
- 可删除已确认的 compatibility/ghost 文件；
- 后续迁移、核心、测试和 deploy 均已获得用户统一授权；
- 不得修改 robot actuator/control loop；若发现必须修改，停止该 packet 并单独说明。

仓库硬约束：

1. 禁止直接运行 `cargo`；一律使用 `bash scripts/cargo-agent.sh <args>`。
2. 当前 worktree 包含大量本迁移的未提交改动；禁止 `git reset --hard`、`git checkout --`、`git clean`、`git add -A`。
3. 不得把旧、新 owner 做双写或用 feature flag 长期并存。
4. Application 不得拥有 SQLite/HTTP/process/filesystem concrete mutation。
5. Runtime 只拥有 canonical reducer/writer/ID/fence/recovery authority，不拥有跨领域 use-case 编排。
6. Composition 只构造对象、连接 ports、启动可取消任务；不得裁决业务状态或 protocol error policy。
7. source-string/路径测试只证明结构，不能替代行为测试。
8. 每个阶段先做最窄验证，再跑 architecture suite；workspace-wide check 只在 M9/M10 运行。
9. 非平凡提交若由执行者创建，必须按仓库 Conventional Commit + 正文格式；本交接不要求自动提交。

## 1. 接手时的权威稳定点

### 1.1 已完成

- M0–M6 已闭合；规范状态见 `docs/plans/2026-08-16-aletheon-wiring-ownership-migration.md:3`。
- M6.43 execution envelope 已完成：`application::turn::coordinator::run_turn_execution_envelope`
  拥有 checkpoint begin/finalize 与 fail/cancel abort 的相对顺序，`TurnLifecycleHandle` 共享
  runtime 单一 reducer；host pipeline 只注入 concrete 闭包
  （`crates/aletheon/src/wiring/host/turn_pipeline.rs:258-660`）。
- M7.1 `adapters-inference` 已完成：新 crate 拥有 HTTP providers/core RPC/backpressure/registry
  与 `RegistryInferencePort`；`core_rpc` crate-private 且 `CorePeerPolicy` 不进入 providers；
  `MachineInferenceRuntime` 留在 host 只管 socket/peer-policy/lifecycle；旧 inference/core_rpc 目录删除。
  执行记录见规范源 M7.1 段。
- M7.2 `adapters-gbrain` 与 M7.3 `adapters-google` 已完成（执行记录见规范源对应段）。

### 1.1a M7.4 残余（Agent 域桥类型，待裁决）

`wiring/adapters/runtime/{native_cognit,pi_rpc}.rs` 依赖 host composition `agent_control` 的
`AgentRuntimeInput`/`AgentRuntimeLauncher`/`AgentHostEffects`/`RuntimeObservedAgentBackend`（引用
`AgentRecoveryRuntimeInput`/`AgentHostAdapter`/`AgentRunProjection`/`CANCEL_WAIT` 等 host 桥类型，
且按 §4.1 runtime 不得依赖 mnemosyne/agora）。完整迁移等价于 M5 Agent 域拆分，需 backend 中性 port
设计裁决。已按 SCOPED 方案完成可搬部分（pi_protocol/pi/provider_worker 入 backend），native_cognit/pi_rpc
留 host，M8 时归入 `host/runtime`。

### 1.1b 真实部署验收记录（2026-08-18）

当前树（M6–M7.5 + M8 host 收敛）已完成真实系统部署验收：

- SHA parity：`/usr/bin/aletheon` == 构建产物 == `f2b4b03773f0becd31fbd4ab47a9469e29284f3fe1d608e20a121a20061fb811`；
- daemon：`NRestarts=0`、重启后稳定、core socket `/run/aletheon/core.sock` 正常；
- doctor：`installed==running`、`binary_matches_installed: true`、`runtime_versions_compatible: true`；
- 真实 LLM 请求：deploy smoke + `aletheon exec` 实跑，`inference_rounds=1, provider_retries=0, tool_calls=0`，
  日志无 provider_unavailable/rejected；
- 真实 GBrain recall：`aletheon memory recall` 经官方 user socket 返回 9 项，其中
  **7 项 `source=supplemental`（GBrain 绑定，`untrusted_reference: true`）**，证明 supplemental item 可进入
  会话召回结果（request_id `44fd8121-dec3-4446-9fe5-ca12a3bbfe5b`）。

- 旧 `crates/aletheon/src/wiring/application/` 已删除。
- Application Turn owner 位于 `crates/application/src/turn/`。
- `TurnCoordinator` 已迁入 `crates/application/src/turn/coordinator.rs`。
- host effect-port 聚合已经删除，concrete seams 分散到对应 adapters。
- M6 Stage 42 已将 capability preparation → Cognit execution 顺序收入
  `application::turn::coordinator::execute_capability_cognition_sequence`
  （当前约 `crates/application/src/turn/coordinator.rs:19-34`）。
- concrete preparation 位于
  `crates/aletheon/src/wiring/adapters/cognitive/turn_preparation.rs`。
- pre/post 顶层阶段分别由
  `application::turn::context::prepare_pre_cognitive` 和
  `application::turn::evidence::TurnEvidenceAccumulator::settle_post_turn` 拥有。
- 当前 host pipeline 约 925 行：
  `crates/aletheon/src/wiring/host/turn_pipeline.rs`。

### 1.2 当前稳定验证证据

最近一次稳定点已通过：

```text
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/cargo-agent.sh check -p application -p aletheon --all-targets
bash scripts/cargo-agent.sh test -p application --lib                         # 74
bash scripts/cargo-agent.sh test -p aletheon --test governed_capability_path # 4
bash scripts/cargo-agent.sh test -p aletheon --test turn_pipeline_order      # 4
bash scripts/cargo-agent.sh test -p aletheon --test daemon_streaming_turn_e2e # 12
bash scripts/cargo-agent.sh test -p aletheon --test coding_production_e2e    # 2
bash scripts/cargo-agent.sh test -p aletheon --test evaluation_turn_settlement # 3
bash tests/suites/architecture/architecture_check.sh
```

已知非阻塞 warning：

- `crates/aletheon/src/wiring/adapters/inference/backpressure.rs` 的
  `provider_backpressure_snapshot` 未使用；M7.1 迁移 inference 时处理，不要在 M6 为消 warning 删除语义。

### 1.3 接手第一步（必须执行）

```bash
cd /home/aurobear/Workspace/aletheon
git status --short
sed -n '1176,1655p' docs/plans/2026-08-16-aletheon-wiring-ownership-migration.md
nl -ba crates/application/src/turn/coordinator.rs | sed -n '1,80p'
nl -ba crates/aletheon/src/wiring/host/turn_pipeline.rs | sed -n '160,680p'
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/cargo-agent.sh check -p application -p aletheon --all-targets
```

若这里失败，先修复接手环境/未完成编辑，不得进入 M7。

## 2. 实施依赖与 packet 顺序

```text
M6.43 execution envelope
  -> M6 full parity closure
     -> M7.1 inference
     -> M7.2 gbrain
     -> M7.3 google
     -> M7.4 agent backend
     -> M7.5 session/sqlite
        -> M8 gateway/daemon/host/composition
           -> M9 delete wiring + public/API/dependency closure
              -> M10 full + installed-runtime acceptance
```

M7.1–M7.5 在设计上可独立回退，但当前 dirty worktree 建议串行完成。不要并行修改 manifests 和 `wiring/mod.rs`。

## 3. M6.43 — 收口 checkpoint/error/cancel execution envelope

### 3.1 要解决的真实问题

当前 `crates/aletheon/src/wiring/host/turn_pipeline.rs` 仍自行裁决：

```text
role graph
  -> workspace checkpoint begin
  -> TurnPipelineLifecycle state transitions
  -> inner pre/cognitive/post execution
  -> error/cancel lifecycle on_abort
  -> checkpoint finalized/aborted
```

其中 checkpoint begin/finalize 和 failure/cancel abort 的相对顺序仍是 host use-case policy。M6 不能只通过换文件或减 LOC 宣告完成。

### 3.2 推荐设计（不要照搬未验证草稿）

在 `crates/application/src/turn/coordinator.rs` 增加一个 provider-neutral execution-envelope use case：

- consumer ports/closures：
  - begin checkpoint；
  - execute inner Turn；
  - observe cancellation；
  - dispatch abort；
  - classify completed outcome；
  - finalize checkpoint；
- 固定顺序：
  1. begin checkpoint；
  2. execute；
  3. 仅当 execute 为 `Err` 时应用 Fail/Cancel terminal 并调用 abort；
  4. checkpoint 存在时按 authoritative outcome finalize/abort；
  5. 返回原始 Turn result。
- abort notification 是 best-effort：其错误必须记录但不能覆盖原始 pipeline error。
- checkpoint begin/finalize 错误保持 fail-visible。
- Rejected 是 `Ok(TurnPipelineOutcome::Rejected)`，checkpoint 应 `Aborted`，但不能触发 execution-error `on_abort`；保持当前语义。
- Completed 只有同时满足 `TurnStop::Completed` 和 `metrics.completed_normally` 才 finalize success。

避免 Rust borrow 陷阱：

- 不要让两个 `FnOnce` 同时捕获同一个 `&mut TurnPipelineLifecycle`；
- 可选方案 A（优先）：Application 定义单一 `TurnExecutionEnvelope` 对象，内部拥有 reducer，向 inner closure 传一个 cloneable、同步串行的 lifecycle handle；
- 可选方案 B：让 execution closure 返回 typed terminal classification，由 Application 在 closure 完成后统一推进 reducer；
- 不建议用 `Arc<std::sync::Mutex<_>>` 跨 `.await` 持锁；若仅在同步 `apply()` 内短锁，必须证明无锁跨 await；
- 为 prepare closure 和 cognition closure分别 clone `TurnRequest`/settings/cancel/lifecycle snapshots，避免 `E0505`；当前 Stage 42 已采用该模式。

### 3.3 文件范围

主要修改：

- `crates/application/src/turn/coordinator.rs`
- `crates/aletheon/src/wiring/host/turn_pipeline.rs`

必要时修改：

- `crates/application/src/turn/ports/runtime.rs`
- `crates/application/src/turn/outcome.rs`
- `crates/aletheon/src/wiring/adapters/turn_preflight.rs`
- `crates/aletheon/src/wiring/adapters/turn_postflight.rs`
- `crates/aletheon/src/wiring/daemon/turn_lifecycle_adapter.rs`
- 已授权的相邻 Turn lifecycle/checkpoint tests

禁止：

- 把 `WorkspaceCheckpointService` concrete type塞进 Application Turn coordinator；应依赖 port/closure；
- 把 Kernel/Agora/Corpus/Dasein/daemon types导入 `application::turn`；
- 改 robot control/actuator；
- 为解决 borrow error 改变 abort/finalize 顺序。

### 3.4 M6.43 验证

```bash
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/cargo-agent.sh check -p application -p aletheon --all-targets
bash scripts/cargo-agent.sh test -p application --lib
bash scripts/cargo-agent.sh test -p aletheon --test turn_coordinator_lifecycle
bash scripts/cargo-agent.sh test -p aletheon --test turn_pipeline_order
bash scripts/cargo-agent.sh test -p aletheon --test daemon_streaming_turn_e2e
bash scripts/cargo-agent.sh test -p aletheon --test principal_turn_isolation
bash scripts/cargo-agent.sh test -p aletheon --test workspace_checkpoint
bash tests/suites/architecture/architecture_check.sh
```

若不存在名为 `workspace_checkpoint` 的 integration target，先用
`rg -n 'checkpoint.*(final|abort)|finalize_turn' crates/aletheon/tests crates/application/src` 定位真实测试，禁止虚构 target。

### 3.5 M6 完整闭合门禁

按规范原文逐条运行：

```bash
! test -e crates/aletheon/src/wiring/composition/turn_service.rs
! rg -n 'wiring::composition::turn_service' crates --glob '*.rs'
! test -d crates/aletheon/src/wiring/application/daemon_turn
! rg -n 'notification_sender|mpsc::Sender<String>' crates/application/src/turn --glob '*.rs'
! rg -n 'KernelRuntime|AgoraService|gateway::|corpus::|crate::config' crates/application/src/turn --glob '*.rs'
bash scripts/cargo-agent.sh test -p runtime --lib
bash scripts/cargo-agent.sh test -p application --lib
bash scripts/cargo-agent.sh test -p aletheon --test turn_engine_parity
bash scripts/cargo-agent.sh test -p aletheon --test turn_service_equivalence
bash scripts/cargo-agent.sh test -p aletheon --test turn_coordinator_lifecycle
bash scripts/cargo-agent.sh test -p aletheon --test turn_pipeline_order
bash scripts/cargo-agent.sh test -p aletheon --test daemon_streaming_turn_e2e
bash scripts/cargo-agent.sh test -p aletheon --test principal_turn_isolation
bash tests/suites/architecture/architecture_check.sh
```

完成后：

- 更新规范源 Stage 43 和顶部状态为 M6 completed / M7 current；
- 更新本交接文件进度；
- 不以 pipeline LOC 作为完成证明；
- 保留 host pipeline 只要它确实是 concrete adapter/assembly，M9 再物理迁出 wiring；但不得继续拥有跨阶段业务顺序。

## 4. M7.1 — 建立 `adapters-inference`

### 4.1 目标布局

```text
crates/adapters/inference/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── core_rpc/{mod,client,protocol,server}.rs
    ├── providers/{mod,anthropic,openai,ollama}.rs
    ├── backpressure/{mod,...}.rs
    ├── registry.rs
    ├── factory.rs
    ├── runtime_facts.rs
    └── utf8_stream.rs
```

实际可按现有文件规模细分，但必须保持三条内部边界：core RPC、HTTP providers、machine backpressure。

### 4.2 来源与 cutover

来源：

- `crates/aletheon/src/wiring/adapters/inference/**`
- `crates/aletheon/src/wiring/core_rpc/{client,protocol,server}.rs`
- `crates/aletheon/src/wiring/core_runtime.rs` 中 `RegistryInferencePort` 及 `InferencePort` impl

保留在 Aletheon host：

- `MachineInferenceRuntime`
- socket path解析
- peer-policy config
- server task lifecycle/reap

步骤：

1. 新建 package、加入 workspace members/dependencies。
2. 先移动纯 provider/UTF-8/backpressure/registry 单元，不改行为。
3. 将 core RPC 设为 crate-private；只暴露 composition 所需 builder/port implementation。
4. 将 `RegistryInferencePort` 移入 adapter；不要移动 `MachineInferenceRuntime`。
5. 更新 Aletheon composition/bootstrap imports，一次性切换调用点。
6. 删除旧 inference/core_rpc 文件，更新 module declarations。
7. 处理 `provider_backpressure_snapshot`：若新 adapter 的 host metrics仍消费则接线；若全仓无消费者且规范不要求该 public query，证明后删除，不能只加 allow(dead_code)。
8. 更新 architecture ledger、dependencies、path inventory、wire surfaces 和相关 source fixtures。

关键不变量：Retry-After、跨 session machine-wide cooldown、UTF-8 chunk boundary、provider/runtime facts、peer credential fail-closed。

验证：使用规范 M7.1 命令，并额外运行现有 provider/backpressure/core user boundary 测试；先用 `rg --files crates/aletheon/tests | rg 'provider|backpressure|core'` 确认 target 名。

## 5. M7.2 — 建立 `adapters-gbrain`

来源：

- `crates/aletheon/src/wiring/adapters/gbrain/**`
- `crates/aletheon/src/wiring/adapters/context_memory.rs`
- 与 supplemental binding/recall 直接相关的 wrapper（先 grep 消费者，不整包搬 memory domain）

步骤：

1. 新 package 依赖稳定 Mnemosyne/contracts ports，不反向依赖 Aletheon。
2. 移动 bootstrap/MCP/recall concrete implementation。
3. 将 credential、marker SHA、source binding 留在 adapter private types。
4. 保持 legacy read-only attestation 与 OAuth authority 分离。
5. 保持 `MemorySensitivityV1` 贯穿 recall → projection → receipt → persistence。
6. 更新 composition 构造与测试 imports后删除旧目录。
7. 更新 owner/effect/dependency ledger。

行为验收必须覆盖：默认 source、GBrain source、degraded fallback、legacy `ExternalReference` 不升级 authority、source mismatch、sensitivity。重点参考：

- `docs/plans/2026-08-16-gbrain-recall-source-mismatch.md`
- `crates/aletheon/tests/gbrain_mcp_adapter.rs`
- memory bifurcation/source binding 相关 tests（执行前重新 grep）。

## 6. M7.3 — 建立 `adapters-google`

来源：

- `crates/aletheon/src/wiring/adapters/google/**`
- `crates/aletheon/src/wiring/adapters/external/google_use_cases.rs`
- `crates/aletheon/src/wiring/adapters/channel/gmail/**`
- 当前 Google/Gmail SQLite schema/projection 代码（先 grep `google|gmail` in adapters-sqlite）

步骤：

1. 建立单一 Google adapter crate。
2. 明确 schema owner：Google/Gmail tables、migration、cursor、delivery idempotency全部归此 crate；`adapters-sqlite` 不保留一半。
3. Gateway 仅保留 channel-neutral transport/dispatch。
4. Corpus Google tools仅保留 governed action，不拥有 OAuth/sync cursor。
5. contracts 根不新增 token/cursor/provider DTO。
6. 一次性切换 bootstrap/event dispatcher/channel imports并删除旧目录。
7. 更新 architecture ledgers与 fixtures。

行为验证至少覆盖 OAuth失效、sync recovery、重复 delivery、cursor restart、Gmail goal draft/report policy、Google event routing。

## 7. M7.4 — 完成 `adapters-agent-backend`

现有 package：`crates/adapters/agent-backend/`。不要再新建第二个 backend crate。

来源：`crates/aletheon/src/wiring/adapters/runtime/**`。

分类迁移：

- Pi protocol/process → backend `pi/`；
- native Cognit runtime → backend `native/`；
- provider worker → backend `provider_worker/`；
- registry/test helpers → production/private 与 `#[cfg(test)]` 分离；
- `turn_operations.rs` 若本质是 Kernel-backed Application Turn port adapter，可留 Aletheon host/runtime adapter 到 M8，不要因目录同名机械搬入 agent backend；先按消费者与 trait owner判断；
- worktree I/O 不进入 backend：移入/复用 `platform::worktree`；runtime仅保留 lease/recovery disposition authority；
- backend不得 mint Runtime Agent/Operation identity或 GenerationFence。

验证覆盖 Pi/native/provider worker、worktree recovery/quarantine、Agent recovery/settlement/memory isolation，以及 architecture suite。

## 8. M7.5 — Session adapters 合入 `adapters-sqlite::session`

来源：`crates/aletheon/src/wiring/adapters/session/**`。

先分类：

| 文件职责 | 目标 |
|---|---|
| canonical/event-sourced/store/checkpoint persistence | `crates/adapters/sqlite/src/session/` |
| runtime/contracts ID conversion | 单一 session identity adapter（可在 sqlite adapter 暴露 port impl，但不得多处转换） |
| Turn event journal/history persistence | sqlite session modules |
| exec TurnService transport adapter | Aletheon host/exec，不能塞 SQLite |
| lifecycle context pure DTO | Application/daemon adapter真实 owner；不因目录名机械入 SQLite |
| test composition | 对应 crate `#[cfg(test)]` |

步骤：逐文件做 consumer census → 移动 persistence → 切 imports → 删除 duplicates → 移动非-persistence文件到真实 owner → 删除旧 session目录。

必须证明：canonical Session/Turn ID转换只有一个 adapter；restart/reconnect/fork/event journal/checkpoint语义不变；没有重复 store wrapper。

完成 M7 后运行规范中的全部静态目录断言、各 adapter crate check、SQLite lib tests、provider/GBrain/Google行为测试和 architecture suite，并更新规范状态为 M7 completed / M8 current。

## 9. M8 — Gateway、daemon transport、host 与 composition 收敛

开始前必须重新读取规范 M8 表格，并逐行核对 `config/architecture/wiring-ownership.tsv`；任何未分配路径不得默认塞 composition。

### 9.1 目标目录

在 `crates/aletheon/src/` 下形成：

```text
host/
  core/
  unix_server/
  user_daemon/
  exec/
  readiness/
  doctor/
  cli/
composition/
  services/
  turn/
  goal/
  agent/
  memory/
```

具体模块名遵循当前代码，不为追求树形图额外造 facade。

### 9.2 Gateway cutover

- daemon wire-safe protocol → `gateway::protocol`；
- typed RPC route translation → `gateway::server`；
- Unix socket/connection/task lifecycle → `aletheon::host::unix_server`；
- tool executor拆成 Application capability use case + Corpus/Kernel concrete adapter；
- debug/admin handler只调用typed query ports，不直读authority store；
- legacy session若仍受支持，放Gateway explicit legacy adapter并在sunset ledger登记，否则删除。

### 9.3 Host cutover

按规范逐项处理 `core_runtime.rs/readiness.rs/doctor.rs/exec*.rs/user_runtime.rs/mode_router.rs/extension.rs`。特别注意：

- `MachineInferenceRuntime` → `host::core`；
- system core拒绝telegram/supplemental-memory/MCP用户凭据的fail-closed逻辑必须保留；
- background task返回cancel handle，shutdown/restart必须authoritative wait/reap；
- host不含repository SQL、业务transition、provider wire policy。

### 9.4 Composition cutover

将 daemon bootstrap 构造逻辑移入 `aletheon::composition`，但不得简单改目录名：

- builder只能读取config、构造Arc、注册impl、启动受管task；
- request/services超预算按Goal/Turn/Agent/Memory等builder拆分；
- 禁止Goal/Turn/Agent状态分支、daemon_react、protocol error mapping；
- 目标单文件≤1000 LOC，request式聚合不得重新达到1.5K。

M8验证使用规范命令，并额外检查所有background task shutdown/restart行为。更新architecture inventories/ledgers，不能提高hotspot预算掩盖超限。

## 10. M9 — 物理删除 `wiring` 与最终静态收敛

前置：M7/M8所有来源目录均已cutover且行为测试通过。

步骤：

1. 运行 `find crates/aletheon/src/wiring -type f`，逐项与owner ledger核对；禁止批量删除未分配文件。
2. 把最后host/composition文件移到最终路径并切换imports。
3. 删除 `crates/aletheon/src/wiring.rs` 与整个 `src/wiring/`。
4. 删除 `crates/aletheon/src/lib.rs` 的 `pub mod wiring`；`launcher.rs`保持唯一public facade。
5. 清理Aletheon manifest中不再使用的rusqlite/reqwest/adapter-only依赖。
6. 审计 `runtime::orchestration::EvidenceDrivenController` 全部消费者；无生产消费者则删除，不能按名称移入Agora。
7. 更新architecture census、hotspot、dependency、path、wire、authority/effect ledgers及所有docs locator。
8. `docs/design/architecture-overview.md` 只有在门禁全绿后才能写“所有权收敛完成”。

执行规范M9全部静态验收。特别运行完整Cargo metadata resolve graph检查传递回边；direct grep不够。architecture gate必须0 findings且0 allowlist debt，不允许新增allowlist覆盖真实问题。

## 11. M10 — 完整、安装态与真实使用验收

### 11.1 开发态完整验证

```bash
bash scripts/aletheon.sh test changed --report /tmp/aletheon-wiring-migration-changed.json
bash scripts/aletheon.sh test architecture
bash scripts/cargo-agent.sh check --workspace --all-targets
bash scripts/cargo-agent.sh fmt --all -- --check
git diff --check
```

逐项完成规范 M10.2 行为矩阵，报告必须分成：

- source architecture evidence；
- behavior parity evidence。

不得用路径不存在替代 principal/cancel/recovery/sensitivity等行为证明。

### 11.2 系统安装态（不可用dev binary替代）

```bash
sudo bash scripts/aletheon.sh deploy
/usr/bin/aletheon version --json
/usr/bin/aletheon doctor --json \
  --config /home/aurobear/.aletheon/config.toml \
  --project-dir /home/aurobear/Workspace/aletheon
```

验收记录必须包含：

1. `sha256sum target/release/aletheon /usr/bin/aletheon`；
2. system core、user daemon、memory agent实际`ExecStart`解析后的可执行路径与SHA；
3. 两个稳定观察窗口的systemd restart counters，期间不得增长；
4. doctor revision/config hash一致且healthy；
5. `/usr/bin/aletheon` + 官方user socket真实LLM请求；
6. rendered frame、persisted session、audit、daemon logs均无`provider_unavailable`、`provider_rejected_request`或rendered inference error；
7. inference rounds/provider retries/tool calls分开报告；
8. 真实GBrain recall，证明首个相关supplemental item进入新会话上下文；
9. monitor verdict与frame/session/audit/log一致。

任何一项缺证据都不能宣告complete。

## 12. 每个 packet 的标准执行模板

```text
A. Re-read
   - 重新读取规范对应段
   - 重新grep所有引用symbol与消费者
B. Evidence
   - 当前owner/调用图/副作用/authority
   - spec-vs-code若不一致，列三列表并停止裁决
C. Edit
   - 一次性owner/import cutover
   - 不双写、不compat alias、不扩大contracts
D. Narrow verify
   - 最窄crate/test target
E. Architecture verify
   - architecture suite + ledger consistency
F. Document
   - 当前line anchors、行为证据、剩余项
G. Optional commit
   - 只stage packet路径，检查staged diff，完整commit正文
```

## 13. 失败定位与最小回退

- Borrow/move错误：先clone immutable snapshots，不改变生命周期顺序。
- Dependency cycle：consumer-owned port放在调用方；不要把adapter DTO塞contracts。
- Source fixture失败：确认是locator过期还是边界真的回退；不能盲改字符串。
- Behavior test失败：回退整个owner cutover调用点，不启用old/new双路径。
- Schema/restart失败：停止后续packet，恢复单一repository owner并验证idempotency/fence。
- Deploy失败：保留构建日志和service状态；不得把dev binary结果写成系统验收。
- Monitor PASS与frame/log冲突：按失败处理并修monitor，不接受PASS。

## 13.5 最终执行记录（2026-08-18）

M8.1–M8.5、M7.4 补完、M9、M10 全部完成并验证。

- **M8.1 gateway protocol**：`crates/gateway/src/protocol/connection.rs`（
  `ConnectionProtocolState`/`ProtocolEvent`/`reduce_protocol` + 7 tests），旧
  `wiring/daemon/protocol.rs` 删除。
- **M8.2 host UnixServer + ConnectionDispatcher**：
  `crates/aletheon/src/host/unix_server.rs`（`ConnectionDispatcher` seam +
  泛型 `UnixServer<D>`，~1300 LOC 从 daemon/server.rs 迁出）。
- **M8.3 typed RPC route groups**：`daemon/handler/typed_gateway.rs`
  `TypedRouteHandler` + `DaemonTypedApplication::from_parts`。
- **M8.4 HandlerPorts 收窄**：五个 feature-port trait（`EvaluationPort`、
  `MemoryMaintenancePort`、`ExtensionsPort`、`MemoryGatewayPort`、
  `SessionInputPort`），`HandlerPorts` 字段改 `Arc<dyn …>`。
- **M8.5 bootstrap → composition**：`composition/daemon_bootstrap/**`（35 文件）。
- **M7.4 补完**：`host/{launcher,readiness,user_runtime,core}` 归位。
- **M9 删除 wiring**：`crates/aletheon/src/wiring/` 物理删除；lib.rs 改为
  `pub mod adapters; pub mod composition; pub mod daemon; pub mod host;`；
  census/checker/suite 路径全部同步。
- **M10 最终安装态验收**：`docs/testing/x13-m10-installed-acceptance-2026-08-18.md`
  — 构建门禁全绿、SHA parity `d8422316…641c7`、三服务 NRestarts=0、真实
  GBrain recall 进入新会话上下文（nonce 原样复述）、event-sink 良性竞态解释。

剩余非阻塞项（单独跟踪）：`debug`/`review` 宽接口 `&Arc<Self>` 摩擦；候选移除
`kernel`/`workspace_checkpoint`/`conscious_workspaces` HandlerPorts 死字段。

## 14. 最终完成审计

最终必须逐项勾选规范源 §9，而不是只看M编号。至少证明：

- `crates/aletheon/src/wiring`不存在；
- public API不暴露host/composition internals；
-完整依赖图无禁止回边；
- durable fact reducer/write/ID mint owner唯一；
- SQLite/FS/HTTP/process effect owner唯一；
- Goal/Agent/Turn/Approval/Verification具备service + consumer port + adapter；
- canonical ID转换集中；
- 无compat re-export、双写、old/new engine flag；
- architecture 0 findings/0 allowlist debt；
- daemon/exec唯一Turn use case；
- 全部开发态与安装态证据齐全；
- 文档locator与最终代码一致。

只有以上全部由当前代码和运行证据证明后，才能把规范状态改为 COMPLETE。

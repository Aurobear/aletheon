# Executive 源文件迁移处置总账

> 基线：`dev@bd1ceac2965832f15cf45b811fd34e052e7e9a67`
>
> 范围：`crates/executive/src/**/*.rs`，共 **372** 个 Rust 源文件。本文是文件级迁移账本；每个当前文件恰好出现一次。Executive 不是目标层，迁移完成后整个 crate 删除。
>
> 实施 checkpoint：首个 `runtime-authority-foundation-v1` PR 已用 baseline caller/re-export census 证明并删除 5 个零生产调用文件：`core/session.rs`、`application/agent/{mod,harness}.rs`、`composition/agents/{mod,loader}.rs`。本账本仍保留这些行以维持基线 372/372 与删除证据；该 PR 合入后的下一份源码 census 应为 367，而不是把这些行改写成从未存在。

## 1. 判定口径

- `MOVE`：职责单一，只迁唯一生产语义到目标 package；旧路径在 caller 切换后删除，测试仍按下述 evidence 分类而不是默认整套搬迁。
- `MERGE`：与目标 owner 已有实现或同文件内多种职责重叠；只迁唯一语义/实现，禁止复制后长期双写。
- `SPLIT`：一个当前文件/类型同时承担多个 owner 的职责；各语义片迁到列出的 owner，原文件或 God-object 类型必须删除，禁止在任一目标层重建同型 service bag。
- `COMPAT`：只允许作为单向委托的短期兼容面；不得新增逻辑。
- `DELETE`：纯 re-export/module shell、无价值 passthrough 或完成切换后不应存在的实现。
- `INVESTIGATE`：必须先用生产 caller、状态 writer、输入/输出与持久格式证据定案；不得借此长期保留。它允许计划诚实表达尚未证明的语义，但进入对应 writer cutover PR 前必须收敛为 `MOVE/MERGE/SPLIT/COMPAT/DELETE`。
- 表中的 “target package” 是语义 owner；出现 `A + B` 表示当前文件必须先拆职责，不能把整文件原样搬入任一侧。任何 source row 的 target 含 spaced ` + ` 时，Disposition 必须是 `SPLIT`；未决证据写入 blocker 与 `C0`，不得用 `INVESTIGATE` 掩盖已知的跨 owner 事实。
- SQLite、filesystem 与 process 等物理实现只能落到 `adapters/sqlite`、`adapters/filesystem` 或 `adapters/linux/*`；Runtime 与 Kernel 只定义所属 port、状态机与 enforcement 语义，不拥有物理适配器。
- 唯一组装根固定为 `aletheon/wiring`。extension 只暴露 typed registration descriptor/factory，禁止建立 extension-owned composition root 或第二套 wiring。
- 测试与 test support 不默认整套迁移。每项先在 `C0` 分类为 `KEEP-EVIDENCE`（唯一行为/恢复证据）、`REPLACE`（用更窄 invariant/installed smoke 替代）或 `DELETE-LATER`（只覆盖旧兼容路径）；只有最小且仍保护目标不变量的证据才随 owner 迁移。

### 1.1 Implementation slices

| Slice | 对应专项 |
|---|---|
| `C0` | 跨计划 census gate：按文件实际 owner 落到 `XRET-00` 加 `RA-00/D0/APX-00/CGP-00/E0/K0` 中适用项 |
| `RA-00`–`RA-06` | Runtime Agent/Session/Turn/Delegate 唯一权威收敛 |
| `K0`–`K7` | 独立小型 Kernel enforcement、approval evidence、descriptor→executor binding |
| `D2`–`D5` | Cognit；Dasein/Metacog；Agora/Mnemosyne；Corpus 领域收敛 |
| `APX-01`–`APX-04` | Application、Goal/Approval 与 persistence adapter 提取 |
| `CGP-00`、`CGP-01`–`CGP-08` | 现有 Gateway census、唯一 `aletheon/wiring` root、typed Gateway、host/presentation 提取 |
| `E0`–`E7` | preservation census/extension seam；Gmail；GBrain；Hardware；Robot VLA；Pi；兼容清退 |
| `XRET-00`–`XRET-05` | compatibility/re-export 清退与 Executive crate 删除 |

表内出现 `RA-00..RA-06`、`K0..K7` 等范围时，只表示一个文件尚跨多个阶段；`C0` 必须在首个实现 PR 前把它收窄为精确 writer-cutover PR 与精确 deletion PR。任何未收窄范围或 `INVESTIGATE` 都阻塞该文件迁移，不得把范围理解为允许一次 Mega PR。

### 1.2 Cutover blockers

| Code | 删除/切换阻塞条件 |
|---|---|
| `B0` | 生产 caller、writer、re-export 与持久格式证据尚未定案 |
| `B1` | Runtime 唯一 aggregate/state machine/journal writer 尚未切换并完成恢复 smoke |
| `B2` | Kernel policy、approval、operation journal 或 sealed executor binding 尚未切换 |
| `B3` | 领域 owner/port 尚未收敛，旧领域生产 caller 尚未清零 |
| `B4` | Application command/query 与 persistence adapter 尚未切换，旧 writer 尚未清零 |
| `B5` | typed Gateway、唯一 `aletheon/wiring` root、daemon/ACP/TUI caller 尚未切换 |
| `B6` | 保留能力的 manifest、等价 smoke、owner adapter、registration/wiring seam 尚未通过 |
| `B7` | 旧 public re-export、feature、migration reader 或 workspace dependency 尚未清零 |

## 2. 文件级处置

### 2.1 `adapters`

| Current path | Semantic owner / target package | Disposition | Implementation slice | Deletion / cutover blocker |
|---|---|---|---|---|
| `crates/executive/src/adapters/agent_control/mod.rs` | `runtime AgentRun repository port + adapters/sqlite/runtime-agent` | **SPLIT** | `RA-02+RA-05` | `B0+B1+B7` |
| `crates/executive/src/adapters/agent_control/sqlite_repository.rs` | `adapters/sqlite/runtime-agent` | **MOVE** | `RA-02+RA-05` | `B1` |
| `crates/executive/src/adapters/artifact/mod.rs` | `application/artifact port + adapters/sqlite + adapters/filesystem` | **SPLIT** | `C0+APX-04+E2+E5` | `B0+B4+B6+B7` |
| `crates/executive/src/adapters/artifact/store.rs` | `application/artifact port + adapters/sqlite + adapters/filesystem` | **SPLIT** | `C0+APX-04+E2+E5` | `B0+B4+B6` |
| `crates/executive/src/adapters/channel/daemon_adapter.rs` | `gateway channel adapter + aletheon/wiring + application Goal/Approval commands + runtime Turn command` | **SPLIT** | `CGP-01+APX-01+APX-02+APX-03+RA-00..RA-06` | `B1+B4+B5+B6` |
| `crates/executive/src/adapters/channel/execd_client.rs` | `adapters/linux/execd-process + capability-specific executor + aletheon config` | **SPLIT** | `K2+K5+CGP-01` | `B0+B2+B5+B7` |
| `crates/executive/src/adapters/channel/gmail/classifier.rs` | `extensions/gmail` | **MOVE** | `E2` | `B6` |
| `crates/executive/src/adapters/channel/gmail/event_ingress.rs` | `extensions/gmail ingress/dedupe + application GoalDraftProposal command + artifact port/adapter + kernel/storage admission + aletheon worker lifecycle` | **SPLIT** | `E2+APX-03+APX-04+K4+CGP-01` | `B0+B2+B4+B5+B6+B7` |
| `crates/executive/src/adapters/channel/gmail/goal_draft.rs` | `extensions/gmail GoalDraftProposal ingress + application GoalDraft/Approval command/store + owner migration bundle` | **SPLIT** | `E2+APX-02+APX-03+APX-04` | `B0+B4+B6+B7` |
| `crates/executive/src/adapters/channel/gmail/ingest.rs` | `extensions/gmail MIME/attachment policy + application artifact port + adapters/filesystem + adapters/sqlite/artifact` | **SPLIT** | `E2+APX-04` | `B0+B4+B6+B7` |
| `crates/executive/src/adapters/channel/gmail/mod.rs` | `extensions/gmail` | **MERGE** | `E2+XRET-03` | `B0+B6+B7` |
| `crates/executive/src/adapters/channel/gmail/report.rs` | `extensions/gmail report/send-outbox + application Approval/Goal commands + artifact adapter + owner migration bundles` | **SPLIT** | `E2-K6a+APX-02+APX-03+APX-04` | `B0+B2+B4+B6+B7` |
| `crates/executive/src/adapters/channel/gmail/sender_policy.rs` | `extensions/gmail` | **MOVE** | `E2` | `B6` |
| `crates/executive/src/adapters/channel/handlers/gmail_ingest.rs` | `extensions/gmail ingress + gateway channel intent adapter` | **SPLIT** | `E2+CGP-03` | `B0+B5+B6+B7` |
| `crates/executive/src/adapters/channel/handlers/mod.rs` | `—` | **DELETE** | `CGP-08+XRET-04` | `B5+B7` |
| `crates/executive/src/adapters/channel/mod.rs` | `—` | **DELETE** | `CGP-08+XRET-04` | `B5+B7` |
| `crates/executive/src/adapters/episode/mod.rs` | `—` | **DELETE** | `E5` | `B6` |
| `crates/executive/src/adapters/episode/sqlite_episode_sink.rs` | `extensions/robot-vla EpisodeSink port + adapters/sqlite/robot-vla` | **SPLIT** | `E5` | `B6` |
| `crates/executive/src/adapters/evaluation/mod.rs` | `metacog/adapters` | **MOVE** | `D3` | `B3` |
| `crates/executive/src/adapters/evaluation/sqlite_store.rs` | `metacog EvaluationStore port + adapters/sqlite/metacog` | **SPLIT** | `D3` | `B3` |
| `crates/executive/src/adapters/events/agent_timeline.rs` | `runtime/journal-read-model` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/adapters/events/agent_tree_projection.rs` | `runtime/journal-read-model` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/adapters/events/debug_projection.rs` | `telemetry/read-model` | **MOVE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/adapters/events/memory_job_projection.rs` | `mnemosyne/read-model` | **MERGE** | `D4` | `B3` |
| `crates/executive/src/adapters/events/metrics_projection.rs` | `telemetry/read-model` | **MOVE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/adapters/events/mod.rs` | `—` | **DELETE** | `XRET-00..XRET-05` | `B1+B7` |
| `crates/executive/src/adapters/events/projection_set.rs` | `runtime/read-model` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/adapters/events/session_projection.rs` | `runtime/journal-read-model` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/adapters/events/sqlite_event_spine.rs` | `runtime EventJournal port + adapters/sqlite/runtime-journal + owner-specific projections/events + legacy reader + aletheon migration` | **SPLIT** | `C0+RA-02+APX-04+XRET-04` | `B0+B1+B3+B4+B7` |
| `crates/executive/src/adapters/external/google_use_cases.rs` | `application external-integration use case + adapters/google capability port` | **SPLIT** | `E2+APX-01` | `B0+B4+B6+B7` |
| `crates/executive/src/adapters/external/mod.rs` | `adapters/google + optional application integration port` | **SPLIT** | `E2+APX-01+XRET-03` | `B0+B4+B6+B7` |
| `crates/executive/src/adapters/external/repository.rs` | `adapters/sqlite/google-integration + owner migration bundle` | **SPLIT** | `E2+APX-04` | `B0+B4+B6+B7` |
| `crates/executive/src/adapters/gbrain/bootstrap.rs` | `adapters/gbrain factory + typed config/secret/lifecycle in aletheon/wiring` | **SPLIT** | `E3+CGP-01` | `B0+B3+B5+B6+B7` |
| `crates/executive/src/adapters/gbrain/mcp_adapter.rs` | `adapters/gbrain SupplementalMemoryPort transport + mnemosyne binding/use case` | **SPLIT** | `E3+D4` | `B0+B3+B6+B7` |
| `crates/executive/src/adapters/gbrain/mod.rs` | `adapters/gbrain` | **MERGE** | `E3+XRET-03` | `B3+B6+B7` |
| `crates/executive/src/adapters/gbrain/worker.rs` | `adapters/gbrain supervised worker + aletheon lifecycle handle` | **SPLIT** | `E3+CGP-01` | `B0+B3+B5+B6+B7` |
| `crates/executive/src/adapters/google/event_dispatcher.rs` | `adapters/google event source + application GoalDraftProposal/mnemosyne commands + gateway channel outbox` | **SPLIT** | `E2+APX-03+D4+CGP-03` | `B0+B3+B4+B5+B6+B7` |
| `crates/executive/src/adapters/google/mod.rs` | `adapters/google` | **MERGE** | `E2+XRET-03` | `B6+B7` |
| `crates/executive/src/adapters/google/store.rs` | `adapters/sqlite/google-sync` | **MOVE** | `E2+APX-04` | `B4+B6` |
| `crates/executive/src/adapters/google/sync_manager.rs` | `adapters/google sync worker + aletheon lifecycle handle` | **SPLIT** | `E2+CGP-01` | `B0+B5+B6+B7` |
| `crates/executive/src/adapters/mod.rs` | `—` | **DELETE** | `XRET-00..XRET-05` | `B7` |
| `crates/executive/src/adapters/plugin/loader.rs` | `application/extension-admin adapter` 或 `—` | **INVESTIGATE** | `C0+E0+E1` | `B0+B4+B6` |
| `crates/executive/src/adapters/plugin/manager.rs` | `application/extension-admin adapter` 或 `—` | **INVESTIGATE** | `C0+E0+E1` | `B0+B4+B6` |
| `crates/executive/src/adapters/plugin/manifest.rs` | `application/extension-admin` 或 `—` | **INVESTIGATE** | `C0+E0+E1` | `B0+B4+B6` |
| `crates/executive/src/adapters/plugin/mod.rs` | `—` | **INVESTIGATE** | `C0+E0+XRET-00..XRET-05` | `B0+B6+B7` |
| `crates/executive/src/adapters/plugin/runtime.rs` | `adapters/linux/plugin-process` 或 `—` | **INVESTIGATE** | `C0+E0+E1` | `B0+B2+B6` |
| `crates/executive/src/adapters/runtime/mod.rs` | `—` | **DELETE** | `XRET-00..XRET-05` | `B1+B6` |
| `crates/executive/src/adapters/runtime/native_cognit.rs` | `cognit/adapters` | **MERGE** | `D2` | `B3` |
| `crates/executive/src/adapters/runtime/pi.rs` | `adapters/pi protocol/terminal + runtime DelegateBackend + adapters/linux/worktree + aletheon config` | **SPLIT** | `E6-K6d+RA-05+APX-04+CGP-01` | `B0+B1+B2+B5+B6+B7` |
| `crates/executive/src/adapters/runtime/pi_protocol.rs` | `adapters/pi` | **MOVE** | `E6` | `B6` |
| `crates/executive/src/adapters/runtime/pi_rpc.rs` | `adapters/pi RPC/terminal + kernel ProcessController/execd transport + runtime DelegateBackend` | **SPLIT** | `E6-K6d+K5+RA-05` | `B0+B1+B2+B6+B7` |
| `crates/executive/src/adapters/runtime/process_supervisor.rs` | `kernel ProcessController port + adapters/linux/process` | **SPLIT** | `K5` | `B0+B2+B7` |
| `crates/executive/src/adapters/runtime/provider_worker.rs` | `cognit CognitiveRun/inference loop + runtime existing Turn/Agent binding + kernel capability + workspace/policy input` | **SPLIT** | `RA-00+RA-04+D2+K3` | `B0+B1+B2+B3+B7` |
| `crates/executive/src/adapters/runtime/worktree_recovery.rs` | `runtime recovery policy/port + adapters/linux/worktree` | **SPLIT** | `RA-04+APX-04` | `B0+B1+B4+B7` |
| `crates/executive/src/adapters/session/canonical_store.rs` | `adapters/sqlite/runtime-session` | **MERGE** | `RA-02+RA-03` | `B1` |
| `crates/executive/src/adapters/session/checkpoint_store_sqlite.rs` | `adapters/sqlite/runtime-session` | **MERGE** | `RA-02+RA-03` | `B1` |
| `crates/executive/src/adapters/session/event_sourced_store.rs` | `adapters/sqlite/runtime-session` | **MERGE** | `RA-02+RA-03` | `B1` |
| `crates/executive/src/adapters/session/mod.rs` | `runtime SessionRepository ports + adapters/sqlite/runtime-session` | **SPLIT** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/adapters/session/observability/fragment.rs` | `telemetry/session` | **MOVE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/adapters/session/observability/metrics.rs` | `telemetry/session` | **MOVE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/adapters/session/observability/mod.rs` | `telemetry` | **MOVE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/adapters/session/observability/reasoning_logger.rs` | `telemetry/session` | **MOVE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/adapters/session/observability/tool_tracker.rs` | `telemetry/session` | **MOVE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/adapters/session/projection_store.rs` | `adapters/sqlite/runtime-session-projection` | **MERGE** | `RA-02+RA-03` | `B1` |
| `crates/executive/src/adapters/session/prompt_queue_sqlite.rs` | `adapters/sqlite/runtime-session` | **MERGE** | `RA-02+RA-03` | `B1` |
| `crates/executive/src/adapters/session/store.rs` | `adapters/sqlite/runtime-session` | **MERGE** | `RA-02+RA-03` | `B1` |
| `crates/executive/src/adapters/session/transaction_settlement_store_sqlite.rs` | `adapters/sqlite/runtime-turn-settlement` | **MERGE** | `RA-02+RA-04` | `B1` |

### 2.2 `application`

| Current path | Semantic owner / target package | Disposition | Implementation slice | Deletion / cutover blocker |
|---|---|---|---|---|
| `crates/executive/src/application/admin_service.rs` | `optional application/admin + adapters/sqlite + runtime/kernel ports + adapters/linux/deploy` 或 `—` | **SPLIT** | `C0+APX-02+APX-04+CGP-01` | `B0+B2+B4+B5` |
| `crates/executive/src/application/agent/harness.rs` | `—` | **DELETE** | `RA-00 / XRET-00` | `B0+B7 closed：workspace caller/re-export census = 0` |
| `crates/executive/src/application/agent/mod.rs` | `—` | **DELETE** | `RA-00 / XRET-00` | `B0+B7 closed：workspace caller/re-export census = 0` |
| `crates/executive/src/application/agent_control/admission.rs` | `runtime Agent admission + kernel ResourceLedger/Operation port` | **SPLIT** | `RA-05+K4` | `B0+B1+B2+B7` |
| `crates/executive/src/application/agent_control/candidate_projection.rs` | `runtime Agent projection + agora candidate/workspace projection` | **SPLIT** | `RA-05+D4` | `B0+B1+B3+B7` |
| `crates/executive/src/application/agent_control/cleanup.rs` | `runtime/agent-control` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/agent_control/context_fork.rs` | `runtime/agent-control` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/agent_control/execution.rs` | `runtime/agent-control` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/agent_control/execution_runner.rs` | `runtime/agent-control` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/agent_control/generation_fence.rs` | `runtime/agent-control` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/agent_control/identity.rs` | `runtime/agent-control` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/agent_control/lifecycle.rs` | `runtime/agent-control` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/agent_control/lifecycle_hooks.rs` | `runtime/agent-control` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/agent_control/live_runs.rs` | `runtime/agent-control` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/agent_control/mailbox.rs` | `runtime/agent-control` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/agent_control/memory.rs` | `runtime post-settlement hook + mnemosyne memory intake` | **SPLIT** | `RA-05+D4` | `B0+B1+B3+B7` |
| `crates/executive/src/application/agent_control/mod.rs` | `runtime AgentSupervisor + kernel operation/time ports + aletheon/wiring；旧 AgentControlService 删除` | **SPLIT** | `RA-05+K4+CGP-01+RA-06` | `B0+B1+B2+B5+B7` |
| `crates/executive/src/application/agent_control/recovery.rs` | `runtime/agent-control` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/agent_control/repository.rs` | `runtime AgentJournal port + adapters/sqlite/runtime-agent` | **SPLIT** | `RA-02+RA-05` | `B0+B1+B7` |
| `crates/executive/src/application/agent_control/settlement.rs` | `runtime Agent terminal reducer + kernel receipt reference + adapters/sqlite/runtime-agent` | **SPLIT** | `RA-02+RA-05+K4` | `B0+B1+B2+B7` |
| `crates/executive/src/application/agent_control/settlement/tests/audit_tests.rs` | `runtime minimal settlement invariant` 或 `—` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1` |
| `crates/executive/src/application/approval/apply_coordinator.rs` | `application/approval use case + runtime/kernel ports + adapters/git/worktree` | **SPLIT** | `APX-02+APX-04+K3` | `B2+B4` |
| `crates/executive/src/application/approval/mod.rs` | `application/approval` | **MERGE** | `APX-02` | `B4` |
| `crates/executive/src/application/approval/repository.rs` | `application/approval port + adapters/sqlite` | **SPLIT** | `APX-02+APX-04` | `B4` |
| `crates/executive/src/application/approval_service.rs` | `application/approval` | **MERGE** | `APX-02` | `B4` |
| `crates/executive/src/application/cache_shape.rs` | `runtime/telemetry` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/capability_benchmark.rs` | `metacog benchmark policy + runtime/delegation adapter + adapters/sqlite` 或 `—` | **SPLIT** | `C0+D3+RA-05+APX-04` | `B0+B1+B3+B4` |
| `crates/executive/src/application/checkpoint_projection.rs` | `runtime/read-model` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/coding_metacog_adapter.rs` | `metacog/adapters/coding` 或 `—` | **INVESTIGATE** | `C0+D3` | `B0+B3` |
| `crates/executive/src/application/coding_metacog_rubric.rs` | `metacog` 或 `—` | **INVESTIGATE** | `C0+D3` | `B0+B3` |
| `crates/executive/src/application/coding_runtime.rs` | `provider-neutral Runtime/Application work command + adapters/pi conversion` | **SPLIT** | `C0+APX-03+RA-01+E6` | `B0+B1+B4+B6` |
| `crates/executive/src/application/cognitive_role_workflow.rs` | `per-Turn cognit policy + runtime delegate lifecycle + agora workspace projection` 或 `—` | **SPLIT** | `C0+RA-04+D2+D4` | `B0+B1+B3+B7` |
| `crates/executive/src/application/cognitive_role_workflow/stages.rs` | `per-Turn cognit policy` 或 `—` | **INVESTIGATE** | `C0+D2` | `B0+B3+B7` |
| `crates/executive/src/application/cognitive_role_workflow/state_machine.rs` | `per-Turn cognit policy` 或 `—` | **INVESTIGATE** | `C0+D2` | `B0+B3+B7` |
| `crates/executive/src/application/cognitive_workspace.rs` | `agora` | **MERGE** | `D4` | `B3` |
| `crates/executive/src/application/command_dispatcher.rs` | `application` | **MERGE** | `APX-01` | `B4` |
| `crates/executive/src/application/compaction_normalize.rs` | `mnemosyne/compaction` | **MERGE** | `D4` | `B3` |
| `crates/executive/src/application/conscious/agent_processor.rs` | `runtime` | **MERGE** | `RA-00..RA-06` | `B1+B3` |
| `crates/executive/src/application/conscious/corpus_processor.rs` | `corpus` | **MERGE** | `D5` | `B3` |
| `crates/executive/src/application/conscious/memory_processor.rs` | `mnemosyne` | **MERGE** | `D4` | `B3` |
| `crates/executive/src/application/conscious/metacog_processor.rs` | `metacog` | **MERGE** | `D3` | `B3` |
| `crates/executive/src/application/conscious/mod.rs` | `agora` | **MERGE** | `D4` | `B3` |
| `crates/executive/src/application/conscious_action.rs` | `agora/action arbitration adapter + runtime/kernel governed-action port` | **SPLIT** | `D4+RA-00..RA-06+K3` | `B1+B2+B3` |
| `crates/executive/src/application/conscious_context_slot.rs` | `agora` | **MERGE** | `D4` | `B3` |
| `crates/executive/src/application/conscious_core_coordinator.rs` | `agora + runtime` | **SPLIT** | `D4` | `B1+B3` |
| `crates/executive/src/application/conscious_core_inspector.rs` | `gateway/read-model` | **MERGE** | `CGP-03` | `B3+B5` |
| `crates/executive/src/application/conscious_core_ports.rs` | `agora/ports` | **MERGE** | `D4` | `B3` |
| `crates/executive/src/application/conscious_field.rs` | `agora` | **MERGE** | `D4` | `B3` |
| `crates/executive/src/application/conscious_workspace.rs` | `agora` | **MERGE** | `D4` | `B3` |
| `crates/executive/src/application/context_assembler.rs` | `runtime/context` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/context_fragment.rs` | `runtime/context` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/daemon_lifecycle.rs` | `application/daemon-lifecycle + adapters/linux/host` | **SPLIT** | `APX-01+CGP-04` | `B4+B5` |
| `crates/executive/src/application/daemon_react.rs` | `runtime/turn` | **MERGE** | `RA-00..RA-06` | `B1+B3` |
| `crates/executive/src/application/daemon_turn/execute.rs` | `runtime/turn` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/daemon_turn/helpers.rs` | `runtime/turn` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/daemon_turn/lifecycle.rs` | `runtime/turn` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/daemon_turn/mod.rs` | `runtime/turn` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/daemon_turn/orchestrator.rs` | `runtime/turn` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/daemon_turn/test_support.rs` | `runtime minimal Turn invariant/recovery smoke` 或 `—` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1` |
| `crates/executive/src/application/daemon_turn_engine.rs` | `runtime/turn` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/dasein_workspace_adapter.rs` | `dasein/adapters` | **MERGE** | `D3` | `B3` |
| `crates/executive/src/application/deterministic_outcome_verifier.rs` | `extensions/robot-vla` | **MERGE** | `E5` | `B6` |
| `crates/executive/src/application/durable_write.rs` | `runtime/journal` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/embodied_execution_adapter.rs` | `hardware core + adapters/hardware-bridge` | **SPLIT** | `E4` | `B2+B6` |
| `crates/executive/src/application/embodied_recovery.rs` | `hardware core + adapters/hardware-bridge` | **SPLIT** | `E4` | `B6` |
| `crates/executive/src/application/embodiment_approval.rs` | `hardware safety + application/approval` | **SPLIT** | `E4+APX-02` | `B2+B4+B6` |
| `crates/executive/src/application/embodiment_authority.rs` | `hardware safety + kernel enforcement` | **SPLIT** | `E4+K3` | `B2+B6` |
| `crates/executive/src/application/embodiment_progress.rs` | `extensions/robot-vla` | **MERGE** | `E5` | `B6` |
| `crates/executive/src/application/embodiment_service.rs` | `hardware core + application/hardware use case` | **SPLIT** | `E4+APX-01` | `B4+B6` |
| `crates/executive/src/application/evaluation/coding_scorer.rs` | `metacog evaluation policy + optional application/evaluation` 或 `—` | **SPLIT** | `C0+D3+APX-01` | `B0+B3+B4` |
| `crates/executive/src/application/evaluation/contract_issuer.rs` | `metacog evidence contract + optional application/evaluation` 或 `—` | **SPLIT** | `C0+D3+APX-01` | `B0+B3+B4` |
| `crates/executive/src/application/evaluation/evidence_collector.rs` | `metacog evidence port + optional adapters/evaluation` 或 `—` | **SPLIT** | `C0+D3+APX-04` | `B0+B3+B4` |
| `crates/executive/src/application/evaluation/mod.rs` | `—` | **INVESTIGATE** | `C0+D3+APX-01+XRET-00..XRET-05` | `B0+B3+B4+B7` |
| `crates/executive/src/application/evaluation/policy.rs` | `metacog policy + optional application/evaluation` 或 `—` | **SPLIT** | `C0+D3+APX-01` | `B0+B3+B4` |
| `crates/executive/src/application/evaluation/projection.rs` | `metacog projection + optional application/evaluation adapter` 或 `—` | **SPLIT** | `C0+D3+APX-04` | `B0+B3+B4` |
| `crates/executive/src/application/evaluation/service.rs` | `optional application/evaluation facade` 或 `—` | **INVESTIGATE** | `C0+D3+APX-01` | `B0+B3+B4` |
| `crates/executive/src/application/event_projection.rs` | `runtime/read-model` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/evolution_proposer.rs` | `metacog` | **MERGE** | `D3` | `B3` |
| `crates/executive/src/application/exec.rs` | `application/idempotency port + adapters/sqlite` | **SPLIT** | `APX-04` | `B4` |
| `crates/executive/src/application/extension_coordinator.rs` | `application/extension-admin` 或 `—` | **INVESTIGATE** | `C0+E0+APX-01` | `B0+B4+B6` |
| `crates/executive/src/application/extension_install.rs` | `application/extension-admin + adapters/filesystem` 或 `—` | **SPLIT** | `C0+E0+APX-01` | `B0+B4+B6` |
| `crates/executive/src/application/extension_manage.rs` | `application/extension-admin` 或 `—` | **INVESTIGATE** | `C0+E0+APX-01` | `B0+B4+B6` |
| `crates/executive/src/application/extension_runtime_router.rs` | `adapters/linux/plugin-process` 或 `—` | **INVESTIGATE** | `C0+E0+E1` | `B0+B2+B6` |
| `crates/executive/src/application/extension_service.rs` | `application/extension-admin` 或 `—` | **INVESTIGATE** | `C0+E0+APX-01` | `B0+B4+B6` |
| `crates/executive/src/application/extension_snapshot.rs` | `application/extension-admin projection` 或 `—` | **INVESTIGATE** | `C0+E0+APX-01` | `B0+B4+B6` |
| `crates/executive/src/application/goal/attempt.rs` | `optional application-goal state/ports + adapters/sqlite + adapters/filesystem` | **SPLIT** | `C0+APX-03+APX-04` | `B0+B4` |
| `crates/executive/src/application/goal/attempt_coordinator.rs` | `Runtime work command adapter + optional long-goal workflow extension + approval/evaluation ports` 或 `—` | **SPLIT** | `C0+APX-03+RA-04+APX-02` | `B0+B1+B4` |
| `crates/executive/src/application/goal/budget.rs` | `optional application-goal policy/ports + adapters/sqlite` | **SPLIT** | `C0+APX-03+APX-04` | `B0+B4` |
| `crates/executive/src/application/goal/coordinator.rs` | `Application GoalDraft activation command + optional long-goal workflow extension + runtime command/query` 或 `—` | **SPLIT** | `C0+APX-03+RA-04+APX-02` | `B0+B1+B4` |
| `crates/executive/src/application/goal/frame.rs` | `optional application-goal policy` 或 `GoalDraft` policy | **INVESTIGATE** | `C0+APX-03` | `B0+B4` |
| `crates/executive/src/application/goal/mod.rs` | `optional application-goal` 或 `—` | **INVESTIGATE** | `C0+APX-03+XRET-00..XRET-05` | `B0+B4+B7` |
| `crates/executive/src/application/goal/retry.rs` | `optional application-goal policy` | **INVESTIGATE** | `C0+APX-03` | `B0+B4` |
| `crates/executive/src/application/goal/store.rs` | `optional application-goal port + adapters/sqlite + adapters/filesystem` | **SPLIT** | `C0+APX-03+APX-04` | `B0+B4` |
| `crates/executive/src/application/goal/summary.rs` | `optional application-goal projection + adapters/sqlite` | **SPLIT** | `C0+APX-03+APX-04` | `B0+B4` |
| `crates/executive/src/application/goal/transition.rs` | `optional application-goal state machine + adapters/sqlite` | **SPLIT** | `C0+APX-03+APX-04` | `B0+B4` |
| `crates/executive/src/application/goal/verification.rs` | `optional application-goal policy + evaluation port + adapters/filesystem + adapters/sqlite` | **SPLIT** | `C0+APX-03+APX-04` | `B0+B4` |
| `crates/executive/src/application/goal/worker.rs` | `Runtime work command adapter + optional long-goal workflow extension` 或 `—` | **SPLIT** | `C0+APX-03+RA-04` | `B0+B1+B4` |
| `crates/executive/src/application/goal_service.rs` | `Application GoalDraft use case + optional long-goal facade` | **SPLIT** | `C0+APX-03` | `B0+B4` |
| `crates/executive/src/application/governed_capability.rs` | `kernel enforcement + runtime/turn capability + application authority + agora arbitration port` | **SPLIT** | `K3+RA-00..RA-06+APX-02+D4` | `B1+B2+B3+B4` |
| `crates/executive/src/application/governed_review/mod.rs` | `optional application/review` 或 `—` | **INVESTIGATE** | `C0+APX-02` | `B0+B4+B7` |
| `crates/executive/src/application/governed_review/prompt.rs` | `optional application/review presentation policy` 或 `—` | **INVESTIGATE** | `C0+APX-02` | `B0+B4` |
| `crates/executive/src/application/governed_review/service.rs` | `optional application/review facade` 或 `—` | **INVESTIGATE** | `C0+APX-02` | `B0+B4` |
| `crates/executive/src/application/governed_review/store.rs` | `optional application/review port + adapters/sqlite` 或 `—` | **SPLIT** | `C0+APX-02+APX-04` | `B0+B4` |
| `crates/executive/src/application/harness_factory.rs` | `cognit CognitiveRun/InferencePort + runtime route/context + E5 Robot + provider/config ports + aletheon/wiring` | **SPLIT** | `RA-04+D2+E5+CGP-01` | `B0+B1+B3+B5+B6+B7` |
| `crates/executive/src/application/health.rs` | `application/health query + adapters/linux/storage-probe` | **SPLIT** | `APX-04+CGP-03` | `B4+B5` |
| `crates/executive/src/application/hook_lifecycle/mod.rs` | `runtime/lifecycle` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/hook_lifecycle/session_distiller.rs` | `mnemosyne/lifecycle` | **MERGE** | `D4` | `B3` |
| `crates/executive/src/application/host_acceptance.rs` | `optional application/evaluation` 或 `—` | **INVESTIGATE** | `C0+APX-01` | `B0+B3+B4` |
| `crates/executive/src/application/inference_port.rs` | `cognit/ports` | **MERGE** | `D2` | `B3` |
| `crates/executive/src/application/lifecycle_contributors.rs` | `runtime/lifecycle` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/memory_consolidation_worker.rs` | `mnemosyne/workers` | **MERGE** | `D4` | `B3` |
| `crates/executive/src/application/memory_gateway.rs` | `mnemosyne` | **MERGE** | `D4` | `B3` |
| `crates/executive/src/application/memory_maintenance.rs` | `mnemosyne` | **MERGE** | `D4` | `B3` |
| `crates/executive/src/application/memory_policy.rs` | `mnemosyne` | **MERGE** | `D4` | `B3` |
| `crates/executive/src/application/memory_projection.rs` | `mnemosyne/read-model` | **MERGE** | `D4` | `B3` |
| `crates/executive/src/application/metacog_approval.rs` | `application/approval` | **MERGE** | `APX-02` | `B4` |
| `crates/executive/src/application/mod.rs` | `—` | **DELETE** | `XRET-00..XRET-05` | `B7` |
| `crates/executive/src/application/orchestration/agent.rs` | `runtime/delegation` 或 `—` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1` |
| `crates/executive/src/application/orchestration/budget.rs` | `runtime/delegation` 或 `—` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1` |
| `crates/executive/src/application/orchestration/builtin/code_agent.rs` | `runtime/delegation adapter` 或 `—` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1` |
| `crates/executive/src/application/orchestration/builtin/fs_agent.rs` | `runtime/delegation adapter` 或 `—` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1` |
| `crates/executive/src/application/orchestration/builtin/mod.rs` | `runtime/delegation adapter` 或 `—` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1+B7` |
| `crates/executive/src/application/orchestration/builtin/net_agent.rs` | `runtime/delegation adapter` 或 `—` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1` |
| `crates/executive/src/application/orchestration/config_agent.rs` | `runtime/delegation` 或 `—` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1` |
| `crates/executive/src/application/orchestration/coordinator.rs` | `runtime/delegation` 或 `—` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1` |
| `crates/executive/src/application/orchestration/delegate.rs` | `runtime/delegation` 或 `—` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1` |
| `crates/executive/src/application/orchestration/digraph/edge.rs` | `application/workflow` | **MERGE** | `APX-03` | `B4` |
| `crates/executive/src/application/orchestration/digraph/graph.rs` | `application/workflow definitions + runtime command adapter; delete embedded executor` | **SPLIT** | `APX-03+RA-00..RA-06` | `B0+B1+B4` |
| `crates/executive/src/application/orchestration/digraph/mod.rs` | `application/workflow` | **MERGE** | `APX-03` | `B4+B7` |
| `crates/executive/src/application/orchestration/digraph/node.rs` | `application/workflow` | **MERGE** | `APX-03` | `B4` |
| `crates/executive/src/application/orchestration/digraph/state.rs` | `application/workflow execution projection` 或 `—` | **INVESTIGATE** | `C0+APX-03` | `B0+B1+B4` |
| `crates/executive/src/application/orchestration/handoff.rs` | `runtime/delegation` 或 `—` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1` |
| `crates/executive/src/application/orchestration/mod.rs` | `—` | **INVESTIGATE** | `C0+APX-03+RA-00..RA-06+XRET-00..XRET-05` | `B0+B1+B4+B7` |
| `crates/executive/src/application/orchestration/registry.rs` | `runtime/delegation` 或 `—` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1` |
| `crates/executive/src/application/orchestration/selector.rs` | `runtime/delegation` 或 `—` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1` |
| `crates/executive/src/application/orchestration/store.rs` | `application/workflow port + adapters/filesystem` | **SPLIT** | `APX-03+APX-04` | `B4` |
| `crates/executive/src/application/orchestration/termination.rs` | `runtime/delegation` 或 `—` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1` |
| `crates/executive/src/application/post_turn.rs` | `runtime/turn` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/post_turn_projection.rs` | `runtime post-turn lifecycle + corpus hook adapter + metacog/evaluation projection + event adapter` | **SPLIT** | `RA-00..RA-06+D3+D5` | `B1+B3` |
| `crates/executive/src/application/pre_turn.rs` | `runtime/turn` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/prefix_cache_observability.rs` | `runtime/telemetry` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/prompt_partition.rs` | `runtime/context` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/request_use_cases.rs` | `application` | **MERGE** | `APX-01` | `B4+B5` |
| `crates/executive/src/application/robot_audit.rs` | `extensions/robot-vla` | **MOVE** | `E5` | `B6` |
| `crates/executive/src/application/robot_episode_promotion.rs` | `extensions/robot-vla` | **MOVE** | `E5` | `B6` |
| `crates/executive/src/application/robot_harness_composition.rs` | `extensions/robot-vla` | **MERGE** | `E5` | `B6` |
| `crates/executive/src/application/robot_perception.rs` | `extensions/robot-vla` | **MERGE** | `E5` | `B6` |
| `crates/executive/src/application/session_input.rs` | `runtime/session` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/session_projection.rs` | `runtime/read-model` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/session_service.rs` | `runtime/session` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/settlement.rs` | `runtime terminal settlement + optional application/evaluation` 或 `—` | **SPLIT** | `C0+RA-04+APX-01` | `B0+B1+B4` |
| `crates/executive/src/application/storage_quota.rs` | `kernel ResourceLedger/storage policy + adapters/linux/usage + application read projection` | **SPLIT** | `K4+APX-04` | `B2+B4` |
| `crates/executive/src/application/thread_authority.rs` | `application requested preference + gateway authenticated binding + adapters/linux canonical path/lock + Dasein/Kernel evidence verifier` | **SPLIT** | `APX-01+CGP-02+APX-04+D3+K3` | `B0+B2+B3+B4+B5+B7` |
| `crates/executive/src/application/tool_stream_bridge.rs` | `runtime ToolEvent port + adapters/event-stream` | **SPLIT** | `RA-00..RA-06` | `B1+B2` |
| `crates/executive/src/application/turn_coordinator.rs` | `runtime/turn` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/turn_diff_tracker.rs` | `runtime/turn` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/turn_engine.rs` | `runtime Turn aggregate/journal + kernel effect + cognit inference/provider + gateway notification` | **SPLIT** | `RA-04+K4+D2+CGP-03` | `B0+B1+B2+B3+B5+B7` |
| `crates/executive/src/application/turn_lifecycle.rs` | `runtime/turn` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/turn_pipeline.rs` | `runtime Turn reducer/orchestration + kernel effect + cognit CognitiveRun + agora workspace + gateway event projection + corpus hook + metacog/mnemosyne outbox；旧 TurnPipeline 删除` | **SPLIT** | `RA-04+K4+D2+D4+CGP-03+D5+D3` | `B0+B1+B2+B3+B5+B7` |
| `crates/executive/src/application/turn_policy.rs` | `runtime/turn-policy` | **MERGE** | `RA-00..RA-06` | `B1+B2` |
| `crates/executive/src/application/turn_recovery.rs` | `runtime/recovery` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/turn_runtime_ports.rs` | `runtime command/context + dasein policy + cognit inference + mnemosyne + application approval + kernel effect + config ports + aletheon/wiring` | **SPLIT** | `RA-04+D2+D3+D4+APX-02+K3+CGP-01` | `B0+B1+B2+B3+B4+B5+B7` |
| `crates/executive/src/application/turn_services.rs` | `—` | **DELETE** | `D1+D2+D3+D4+RA-04+XRET-02` | `B1+B3+B7` |
| `crates/executive/src/application/turn_tool_projection.rs` | `runtime/read-model` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/application/verification/checks.rs` | `optional application/verification + owner-specific invariant adapters` 或 `—` | **SPLIT** | `C0+APX-01` | `B0+B4` |
| `crates/executive/src/application/verification/command.rs` | `optional application/verification facade` 或 `—` | **INVESTIGATE** | `C0+APX-01` | `B0+B4` |
| `crates/executive/src/application/verification/mod.rs` | `—` | **INVESTIGATE** | `C0+APX-01+XRET-00..XRET-05` | `B0+B4+B7` |
| `crates/executive/src/application/verification/policy.rs` | `optional application/verification policy` 或 `—` | **INVESTIGATE** | `C0+APX-01` | `B0+B4` |
| `crates/executive/src/application/workspace_checkpoint.rs` | `runtime WorkspaceCheckpointPort + adapters/linux/filesystem + adapters/sqlite + kernel lease/receipt` | **SPLIT** | `RA-04+APX-04+K4` | `B0+B1+B2+B4+B7` |
| `crates/executive/src/application/workspace_trust.rs` | `application requested trust preference + gateway/host identity + adapters/filesystem/git-metadata + Dasein/Kernel evidence enforcement` | **SPLIT** | `APX-01+CGP-02+APX-04+D3+K3` | `B0+B2+B3+B4+B5+B7` |
| `crates/executive/src/application/world_state.rs` | `hardware core + adapters/hardware-bridge projection` | **SPLIT** | `E4` | `B6` |

### 2.3 `compatibility`

| Current path | Semantic owner / target package | Disposition | Implementation slice | Deletion / cutover blocker |
|---|---|---|---|---|
| `crates/executive/src/compatibility/legacy_session_service.rs` | `runtime/compat` | **COMPAT** | `RA-00..RA-06` | `B1+B7` |
| `crates/executive/src/compatibility/mod.rs` | `—` | **DELETE** | `XRET-00..XRET-05` | `B7` |
| `crates/executive/src/compatibility/persistence_migrations.rs` | `owner-specific migration bundles + aletheon migrate` | **SPLIT** | `APX-04+XRET-00..XRET-05` | `B4+B7` |

### 2.4 `composition`

| Current path | Semantic owner / target package | Disposition | Implementation slice | Deletion / cutover blocker |
|---|---|---|---|---|
| `crates/executive/src/composition/adapter_registry.rs` | `aletheon/wiring registry` 或 `—` | **INVESTIGATE** | `C0+CGP-01` | `B0+B5+B7` |
| `crates/executive/src/composition/agent_loader/mod.rs` | `runtime/profiles` | **INVESTIGATE** | `C0+RA-00..RA-06` | `B0+B1` |
| `crates/executive/src/composition/agents/loader.rs` | `—` | **DELETE** | `RA-00 / XRET-00` | `B0+B7 closed：仅本地单测；production 使用 composition/agent_loader` |
| `crates/executive/src/composition/agents/mod.rs` | `—` | **DELETE** | `RA-00 / XRET-00` | `B0+B7 closed：workspace caller/re-export census = 0` |
| `crates/executive/src/composition/config/agent.rs` | `runtime/config` | **MERGE** | `CGP-01` | `B1+B5` |
| `crates/executive/src/composition/config/backpressure.rs` | `runtime/config` | **MERGE** | `CGP-01` | `B1+B5` |
| `crates/executive/src/composition/config/channel.rs` | `gateway/extension-owned channel config types + aletheon/wiring config` | **SPLIT** | `E1+CGP-01` | `B0+B5+B6+B7` |
| `crates/executive/src/composition/config/coding.rs` | `cognit/config` | **MERGE** | `D2` | `B3+B5` |
| `crates/executive/src/composition/config/diagnostics.rs` | `aletheon/config` | **MERGE** | `CGP-01` | `B5` |
| `crates/executive/src/composition/config/evaluation.rs` | `metacog/config` | **MERGE** | `D3` | `B3+B5` |
| `crates/executive/src/composition/config/genome.rs` | `metacog/config` | **MERGE** | `D3` | `B3+B5` |
| `crates/executive/src/composition/config/governed_review.rs` | `metacog/config` | **MERGE** | `D3` | `B3+B5` |
| `crates/executive/src/composition/config/grok_hardening.rs` | `cognit/config` | **MERGE** | `D2` | `B3+B5` |
| `crates/executive/src/composition/config/infra.rs` | `aletheon/config` | **MERGE** | `CGP-01` | `B5` |
| `crates/executive/src/composition/config/integrations.rs` | `extension-owned typed config/registration descriptors + aletheon/wiring config` | **SPLIT** | `E1+CGP-01` | `B0+B5+B6+B7` |
| `crates/executive/src/composition/config/memory_policy.rs` | `mnemosyne/config` | **MERGE** | `D4` | `B3+B5` |
| `crates/executive/src/composition/config/mod.rs` | `aletheon/config` | **MERGE** | `CGP-01` | `B5` |
| `crates/executive/src/composition/config/provenance.rs` | `aletheon/config` | **MERGE** | `CGP-01` | `B5` |
| `crates/executive/src/composition/config/provider.rs` | `cognit/config` | **MERGE** | `D2` | `B3+B5` |
| `crates/executive/src/composition/config/robot.rs` | `extensions/robot-vla/config` | **MOVE** | `E5` | `B6` |
| `crates/executive/src/composition/config/schema.rs` | `aletheon/config` | **MERGE** | `CGP-01` | `B5` |
| `crates/executive/src/composition/config/supplemental_memory.rs` | `mnemosyne/config` | **MERGE** | `D4` | `B3+B5` |
| `crates/executive/src/composition/exec_corpus.rs` | `corpus + aletheon/wiring` | **SPLIT** | `D5` | `B3+B5` |
| `crates/executive/src/composition/exec_session.rs` | `runtime/session` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/composition/mod.rs` | `—` | **DELETE** | `CGP-08+XRET-04` | `B5+B7` |
| `crates/executive/src/composition/prefix_builder.rs` | `runtime/context` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/composition/skill_admin.rs` | `application/extension-admin registration + aletheon/wiring` 或 `—` | **SPLIT** | `C0+E0+APX-01` | `B0+B4+B6` |
| `crates/executive/src/composition/turn_coordinator.rs` | `runtime/turn` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/composition/turn_service.rs` | `runtime/turn` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/composition/user_runtime/mod.rs` | `runtime + aletheon/wiring` | **SPLIT** | `CGP-01+CGP-08+XRET-04` | `B1+B5+B7` |

### 2.5 `core`

| Current path | Semantic owner / target package | Disposition | Implementation slice | Deletion / cutover blocker |
|---|---|---|---|---|
| `crates/executive/src/core/checkpoint.rs` | `runtime WorkspaceCheckpointPort + adapters/linux/workspace` 或 `—` | **SPLIT** | `C0+RA-04+APX-04` | `B0+B1+B4+B7` |
| `crates/executive/src/core/corpus_group.rs` | `corpus typed handle + aletheon/wiring；旧 CorpusGroup 删除` | **SPLIT** | `D5+CGP-01+XRET-02` | `B0+B3+B5+B7` |
| `crates/executive/src/core/deploy.rs` | `optional application/admin-review + adapters/linux/deploy` 或 `—` | **SPLIT** | `C0+APX-02+APX-04` | `B0+B4+B5` |
| `crates/executive/src/core/domain_ports.rs` | `individual Agora/Metacog/Corpus/Cognit handles + aletheon/wiring；旧 DomainPorts getter bag 删除` | **SPLIT** | `D2+D3+D4+D5+CGP-01+XRET-02` | `B0+B3+B5+B7` |
| `crates/executive/src/core/evolution_coordinator.rs` | `metacog proposal/evaluation + cognit reflection + dasein signal + application/config owner` 或 `—` | **SPLIT** | `C0+D2+D3+APX-04` | `B0+B3+B4+B7` |
| `crates/executive/src/core/mcp_config.rs` | `—` | **DELETE** | `XRET-00..XRET-05` | `B5+B7` |
| `crates/executive/src/core/memory_group.rs` | `mnemosyne typed handle + aletheon/wiring；旧 MemoryGroup 删除` | **SPLIT** | `D4+CGP-01+XRET-02` | `B0+B3+B5+B7` |
| `crates/executive/src/core/mod.rs` | `—` | **DELETE** | `XRET-00..XRET-05` | `B7` |
| `crates/executive/src/core/mode_router.rs` | `gateway/application requested collaboration preference + cognit/runtime prompt context + dasein verdict + kernel effect scope` | **SPLIT** | `CGP-02+APX-01+D2+RA-04+D3+K2` | `B0+B1+B2+B3+B4+B5+B7` |
| `crates/executive/src/core/orchestrator.rs` | `runtime` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/core/permission_manager.rs` | `dasein + kernel` | **SPLIT** | `D3` | `B2+B3` |
| `crates/executive/src/core/runtime_core.rs` | `runtime typed handles + aletheon/wiring；旧 RuntimeCore getter bag 删除` | **SPLIT** | `RA-03+RA-04+RA-05+CGP-01+CGP-08+XRET-04` | `B0+B1+B5+B7` |
| `crates/executive/src/core/runtime_registry.rs` | `runtime/registry` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/core/security_group.rs` | `kernel typed enforcement handles + aletheon/wiring；旧 SecurityGroup 删除` | **SPLIT** | `K2+K3+K4+CGP-01+XRET-02` | `B0+B2+B5+B7` |
| `crates/executive/src/core/session.rs` | `—` | **DELETE** | `RA-00 / XRET-00` | `B0+B7 closed：workspace caller/re-export census = 0` |
| `crates/executive/src/core/session_gateway/approval_flow.rs` | `application Approval query/projection + gateway typed adapter` | **SPLIT** | `APX-02+CGP-03` | `B0+B4+B5+B7` |
| `crates/executive/src/core/session_gateway/gateway.rs` | `runtime/application/domain query ports + gateway debug projection` | **SPLIT** | `RA-03+APX-01+D3+D4+CGP-03` | `B0+B1+B3+B4+B5+B7` |
| `crates/executive/src/core/session_gateway/mod.rs` | `—` | **DELETE** | `CGP-08+XRET-04` | `B0+B1+B3+B4+B5+B7` |
| `crates/executive/src/core/session_gateway/param_registry.rs` | `owner-specific config/query ports + gateway parameter projection` | **SPLIT** | `D2+D3+D4+CGP-03` | `B0+B3+B5+B7` |
| `crates/executive/src/core/session_gateway/session_state.rs` | `runtime SessionProjection + gateway query adapter` | **SPLIT** | `RA-03+CGP-03` | `B0+B1+B5+B7` |
| `crates/executive/src/core/session_gateway/snapshot.rs` | `gateway projection composed from typed owner queries` | **SPLIT** | `RA-03+D3+D4+APX-02+CGP-03` | `B0+B1+B3+B4+B5+B7` |
| `crates/executive/src/core/session_gateway/subsystem_query.rs` | `owner-specific query ports + gateway query adapter` | **SPLIT** | `D2+D3+D4+CGP-03` | `B0+B3+B5+B7` |
| `crates/executive/src/core/session_gateway/turn_context.rs` | `runtime TurnProjection + gateway query adapter` | **SPLIT** | `RA-04+CGP-03` | `B0+B1+B5+B7` |
| `crates/executive/src/core/session_group.rs` | `runtime SessionAuthority/query handles + aletheon/wiring；旧 SessionGroup 删除` | **SPLIT** | `RA-03+CGP-01+XRET-02` | `B0+B1+B5+B7` |
| `crates/executive/src/core/sub_agent.rs` | `runtime/compat` | **COMPAT** | `RA-00..RA-06` | `B1+B7` |
| `crates/executive/src/core/system_core_runtime.rs` | `adapters/provider + aletheon/inference-host` | **SPLIT** | `D2+CGP-04+CGP-08+XRET-04` | `B3+B5+B7` |
| `crates/executive/src/core/verdict_handler.rs` | `dasein verdict semantics + application approval adapter + runtime action normalization + kernel enforcement` | **SPLIT** | `D3+APX-02+RA-04+K3` | `B0+B1+B2+B3+B4+B7` |

### 2.6 `extensions`

| Current path | Semantic owner / target package | Disposition | Implementation slice | Deletion / cutover blocker |
|---|---|---|---|---|
| `crates/executive/src/extensions/mod.rs` | `—` | **DELETE** | `XRET-00..XRET-05` | `B6+B7` |
| `crates/executive/src/extensions/runtime/mod.rs` | `—` | **DELETE** | `XRET-00..XRET-05` | `B6+B7` |
| `crates/executive/src/extensions/runtime/subprocess.rs` | `kernel ProcessController port + adapters/linux/plugin-process` 或 `—` | **SPLIT** | `C0+E0+K5` | `B0+B2+B6` |

### 2.7 `host`

| Current path | Semantic owner / target package | Disposition | Implementation slice | Deletion / cutover blocker |
|---|---|---|---|---|
| `crates/executive/src/host/container.rs` | `aletheon/host + runtime command ports` | **SPLIT** | `CGP-01+RA-00..RA-06` | `B1+B5` |
| `crates/executive/src/host/core_rpc/client.rs` | `inference protocol + adapters/local-rpc` | **SPLIT** | `D2+CGP-04` | `B3+B5` |
| `crates/executive/src/host/core_rpc/mod.rs` | `—` | **DELETE** | `XRET-00..XRET-05` | `B5+B7` |
| `crates/executive/src/host/core_rpc/protocol.rs` | `inference protocol + adapters/local-rpc` | **SPLIT** | `D2+CGP-04` | `B3+B5` |
| `crates/executive/src/host/core_rpc/server.rs` | `adapters/local-rpc + aletheon/inference-host` | **SPLIT** | `D2+CGP-04` | `B3+B5` |
| `crates/executive/src/host/daemon/bootstrap/agents.rs` | `runtime + aletheon/wiring` | **SPLIT** | `CGP-01` | `B1+B5` |
| `crates/executive/src/host/daemon/bootstrap/approval_gate.rs` | `kernel + aletheon/wiring` | **SPLIT** | `K0..K7` | `B2+B5` |
| `crates/executive/src/host/daemon/bootstrap/bundled_profiles.rs` | `runtime/profiles` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/host/daemon/bootstrap/channels.rs` | `gateway channel adapter factories + aletheon/wiring` | **SPLIT** | `E1+CGP-01+CGP-03+CGP-04` | `B0+B5+B6+B7` |
| `crates/executive/src/host/daemon/bootstrap/cognition.rs` | `cognit + aletheon/wiring` | **SPLIT** | `D2` | `B3+B5` |
| `crates/executive/src/host/daemon/bootstrap/embodiment.rs` | `hardware + adapters/hardware-bridge + aletheon/wiring` | **SPLIT** | `E4+CGP-01` | `B5+B6` |
| `crates/executive/src/host/daemon/bootstrap/extension_bootstrap.rs` | `extension worker factories/handles + aletheon/wiring` | **SPLIT** | `E1+CGP-01` | `B0+B5+B6+B7` |
| `crates/executive/src/host/daemon/bootstrap/extension_connectors.rs` | `extension-owned connector ports + aletheon/wiring` | **SPLIT** | `E1+CGP-01` | `B0+B5+B6+B7` |
| `crates/executive/src/host/daemon/bootstrap/extension_publisher.rs` | `extension-owned publisher port + aletheon lifecycle` | **SPLIT** | `E1+CGP-01` | `B0+B5+B6+B7` |
| `crates/executive/src/host/daemon/bootstrap/extensions.rs` | `extension registration descriptors + aletheon/wiring；旧 aggregate 删除` | **SPLIT** | `E1+CGP-01+XRET-03` | `B0+B5+B6+B7` |
| `crates/executive/src/host/daemon/bootstrap/google.rs` | `extensions/gmail registration + aletheon/wiring` | **SPLIT** | `E2` | `B6` |
| `crates/executive/src/host/daemon/bootstrap/inference.rs` | `cognit + aletheon/wiring` | **SPLIT** | `D2` | `B3+B5` |
| `crates/executive/src/host/daemon/bootstrap/integrations.rs` | `extension-owned typed adapters + aletheon/wiring；旧 service bag 删除` | **SPLIT** | `E1+CGP-01+XRET-03` | `B0+B5+B6+B7` |
| `crates/executive/src/host/daemon/bootstrap/memory.rs` | `mnemosyne + aletheon/wiring` | **SPLIT** | `D4` | `B3+B5` |
| `crates/executive/src/host/daemon/bootstrap/mod.rs` | `—` | **DELETE** | `CGP-08+XRET-04` | `B5+B7` |
| `crates/executive/src/host/daemon/bootstrap/params.rs` | `aletheon/wiring` | **MERGE** | `CGP-01` | `B5` |
| `crates/executive/src/host/daemon/bootstrap/production_embodiment.rs` | `hardware + adapters/hardware-bridge + aletheon/wiring` | **SPLIT** | `E4+CGP-01` | `B5+B6` |
| `crates/executive/src/host/daemon/bootstrap/request.rs` | `runtime/application/domain typed ports + aletheon/wiring；旧 RequestHandler 类型删除` | **SPLIT** | `RA-03+RA-04+RA-05+APX-01+D2+D3+D4+CGP-01+CGP-08+XRET-04` | `B0+B1+B2+B3+B4+B5+B6+B7` |
| `crates/executive/src/host/daemon/bootstrap/request_ports.rs` | `runtime + aletheon/wiring` | **SPLIT** | `CGP-01` | `B1+B5` |
| `crates/executive/src/host/daemon/bootstrap/request_tests.rs` | `minimal wiring invariant/installed smoke` 或 `—` | **INVESTIGATE** | `C0+CGP-01` | `B0+B5` |
| `crates/executive/src/host/daemon/bootstrap/robot.rs` | `extensions/robot-vla registration + aletheon/wiring` | **SPLIT** | `E5` | `B6` |
| `crates/executive/src/host/daemon/bootstrap/role_profiles.rs` | `runtime/profiles` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/host/daemon/bootstrap/runtime.rs` | `runtime + aletheon/wiring` | **SPLIT** | `CGP-01` | `B1+B5` |
| `crates/executive/src/host/daemon/bootstrap/runtime/runtime_tests.rs` | `minimal runtime wiring invariant/installed smoke` 或 `—` | **INVESTIGATE** | `C0+CGP-01+RA-00..RA-06` | `B0+B1+B5` |
| `crates/executive/src/host/daemon/bootstrap/security.rs` | `kernel + aletheon/wiring` | **SPLIT** | `K0..K7` | `B2+B5` |
| `crates/executive/src/host/daemon/bootstrap/services.rs` | `runtime + aletheon/wiring` | **SPLIT** | `CGP-01` | `B1+B5` |
| `crates/executive/src/host/daemon/bootstrap/sessions.rs` | `runtime + aletheon/wiring` | **SPLIT** | `CGP-01` | `B1+B5` |
| `crates/executive/src/host/daemon/bootstrap/storage.rs` | `aletheon/wiring` | **MERGE** | `CGP-01` | `B5` |
| `crates/executive/src/host/daemon/bootstrap/tools.rs` | `corpus + aletheon/wiring` | **SPLIT** | `D5` | `B3+B5` |
| `crates/executive/src/host/daemon/bootstrap/turn_runtime.rs` | `runtime + aletheon/wiring` | **SPLIT** | `CGP-01` | `B1+B5` |
| `crates/executive/src/host/daemon/debug_handler.rs` | `owner-specific debug query ports + gateway projection handler` | **SPLIT** | `CGP-00+CGP-03` | `B0+B1+B3+B4+B5+B7` |
| `crates/executive/src/host/daemon/handler/connection.rs` | `gateway/server` | **MERGE** | `CGP-03` | `B4+B5` |
| `crates/executive/src/host/daemon/handler/format.rs` | `gateway/server` | **MERGE** | `CGP-03` | `B4+B5` |
| `crates/executive/src/host/daemon/handler/init.rs` | `gateway/server` | **MERGE** | `CGP-03` | `B4+B5` |
| `crates/executive/src/host/daemon/handler/mod.rs` | `gateway/server` | **MERGE** | `CGP-03` | `B5` |
| `crates/executive/src/host/daemon/handler/ports.rs` | `per-route typed Application/query ports；旧 HandlerPorts service bag 删除` | **SPLIT** | `CGP-00+CGP-03+CGP-08+XRET-04` | `B0+B1+B3+B4+B5+B6+B7` |
| `crates/executive/src/host/daemon/handler/rpc.rs` | `gateway/server` | **MERGE** | `CGP-03` | `B4+B5` |
| `crates/executive/src/host/daemon/handler/rpc/rpc_admin.rs` | `optional application/admin typed route + gateway adapter` 或 `—` | **SPLIT** | `CGP-00+C0+APX-01+CGP-03` | `B0+B4+B5+B7` |
| `crates/executive/src/host/daemon/handler/rpc/rpc_approval.rs` | `application Approval command/query + gateway adapter` | **SPLIT** | `CGP-00+APX-02+CGP-03` | `B0+B4+B5+B7` |
| `crates/executive/src/host/daemon/handler/rpc/rpc_evaluation.rs` | `optional application/evaluation route + gateway adapter` 或 `—` | **SPLIT** | `CGP-00+C0+D3+APX-01+CGP-03` | `B0+B3+B4+B5+B7` |
| `crates/executive/src/host/daemon/handler/rpc/rpc_extension.rs` | `optional application/extension-admin route + gateway adapter` 或 `—` | **SPLIT** | `CGP-00+C0+E0+APX-01+CGP-03` | `B0+B4+B5+B6+B7` |
| `crates/executive/src/host/daemon/handler/rpc/rpc_goal.rs` | `Application GoalDraft/optional workflow command/query + gateway adapter` 或 `—` | **SPLIT** | `CGP-00+C0+APX-03+CGP-03` | `B0+B4+B5+B7` |
| `crates/executive/src/host/daemon/handler/rpc/rpc_google.rs` | `Gmail/Google application port + gateway adapter` | **SPLIT** | `CGP-00+E2+APX-03+CGP-03` | `B0+B4+B5+B6+B7` |
| `crates/executive/src/host/daemon/handler/rpc/rpc_health.rs` | `aletheon host health snapshot + gateway handler` | **SPLIT** | `CGP-00+CGP-03+CGP-04` | `B0+B5+B7` |
| `crates/executive/src/host/daemon/handler/rpc/rpc_memory.rs` | `mnemosyne query port + gateway handler` | **SPLIT** | `CGP-00+D4+CGP-03` | `B0+B3+B5+B7` |
| `crates/executive/src/host/daemon/handler/rpc/rpc_review.rs` | `optional application/review route + gateway adapter` 或 `—` | **SPLIT** | `CGP-00+C0+APX-02+CGP-03` | `B0+B4+B5+B7` |
| `crates/executive/src/host/daemon/handler/rpc/rpc_session.rs` | `runtime Session command/query + gateway adapter` | **SPLIT** | `CGP-00+RA-03+CGP-03` | `B0+B1+B5+B7` |
| `crates/executive/src/host/daemon/handler/rpc/rpc_skill.rs` | `optional application/extension-admin route + gateway adapter` 或 `—` | **SPLIT** | `CGP-00+C0+E0+APX-01+CGP-03` | `B0+B4+B5+B6+B7` |
| `crates/executive/src/host/daemon/handler/rpc/rpc_turn.rs` | `runtime Turn command/query + gateway adapter` | **SPLIT** | `CGP-00+RA-04+CGP-03` | `B0+B1+B5+B7` |
| `crates/executive/src/host/daemon/handler/rpc/rpc_workflow.rs` | `optional application/workflow route + gateway adapter` 或 `—` | **SPLIT** | `CGP-00+C0+APX-03+CGP-03` | `B0+B4+B5+B7` |
| `crates/executive/src/host/daemon/handler/tool_executor.rs` | `kernel CapabilityExecutor binding + corpus hook/tool adapter + runtime context + owner observers；旧 TurnToolExecutor 删除` | **SPLIT** | `K2+K4+D5+RA-04+D3+D4+XRET-02` | `B0+B1+B2+B3+B5+B7` |
| `crates/executive/src/host/daemon/handler/turn_handler.rs` | `runtime Turn command/query + gateway transport adapter` | **SPLIT** | `CGP-00+RA-04+CGP-03` | `B0+B1+B4+B5+B7` |
| `crates/executive/src/host/daemon/mcp_embedded.rs` | `corpus/adapters/mcp` | **MOVE** | `D5` | `B3+B5` |
| `crates/executive/src/host/daemon/mod.rs` | `aletheon/host` | **MERGE** | `CGP-04` | `B5` |
| `crates/executive/src/host/daemon/model_router.rs` | `cognit/adapters` | **MERGE** | `D2` | `B3` |
| `crates/executive/src/host/daemon/protocol.rs` | `gateway-protocol` | **MERGE** | `CGP-02` | `B5` |
| `crates/executive/src/host/daemon/server.rs` | `gateway/server + aletheon/host` | **SPLIT** | `CGP-04` | `B5` |
| `crates/executive/src/host/daemon/session_manager.rs` | `runtime/context` | **MERGE** | `RA-00..RA-06` | `B1` |
| `crates/executive/src/host/doctor.rs` | `application/diagnostics + adapters/linux + CLI presentation` | **SPLIT** | `APX-01+CGP-05` | `B4+B5` |
| `crates/executive/src/host/launcher.rs` | `aletheon/host` | **MERGE** | `CGP-01+CGP-08+XRET-04` | `B5+B7` |
| `crates/executive/src/host/mod.rs` | `—` | **DELETE** | `CGP-08+XRET-04` | `B5+B7` |
| `crates/executive/src/host/readiness.rs` | `aletheon/host` | **MOVE** | `CGP-04` | `B5` |
| `crates/executive/src/host/systemd.rs` | `aletheon/host` | **MOVE** | `CGP-04` | `B5` |

### 2.8 `tools`

| Current path | Semantic owner / target package | Disposition | Implementation slice | Deletion / cutover blocker |
|---|---|---|---|---|
| `crates/executive/src/tools/mod.rs` | `—` | **DELETE** | `XRET-00..XRET-05` | `B3+B7` |
| `crates/executive/src/tools/self_observe.rs` | `dasein bounded SelfProjection query + corpus capability/tool adapter` 或 `—` | **SPLIT** | `C0+D3+D5` | `B0+B3+B7` |

### 2.9 `src root`

| Current path | Semantic owner / target package | Disposition | Implementation slice | Deletion / cutover blocker |
|---|---|---|---|---|
| `crates/executive/src/lib.rs` | `—` | **DELETE** | `XRET-00..XRET-05` | `B7` |

## 3. 完整性与执行规则

处置计数：`MOVE=19`、`MERGE=123`、`SPLIT=156`、`COMPAT=2`、`DELETE=25`、`INVESTIGATE=47`。`INVESTIGATE` 不是最终处置：对应 slice 的 writer/caller cutover 开始前必须有证据地归零。任何后续新增 Executive 文件都必须先更新本账本；进入实施后原则上禁止新增，只允许减少。

每个迁移 PR 必须同时完成：目标 owner 接管、生产 caller 切换、最小等价/恢复 smoke、旧实现及旧 re-export 删除。不得把 `MERGE` 实施成复制；不得把 `COMPAT` 变成永久 facade；不得等到最终 PR 才批量删除仍含业务逻辑的文件。最终 `XRET-00..XRET-05` 只能删除已经没有业务逻辑、没有生产 caller、没有 workspace dependent 的空壳。

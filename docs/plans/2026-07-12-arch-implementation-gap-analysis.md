# Aletheon 宏内核架构 — 实施差距分析

> **日期**: 2026-07-12
> **基线**: `dev` 分支当前代码 vs `docs/arch/` 全部设计文档
> **方法论**: 6 个设计文档逐个对比实际代码，每个结论有 `path:line` 证据锚点

---

## 1. 总览

| 阶段 | PR 编号 | 设计文档 | 完成度 | 判定 |
|---|---|---|---|---|
| M0 | 0A–0E | 01/07 | ~95% | ✅ Gate 0 通过 |
| Process/Operation | 1A–1C | 02 | ~80% | ✅ Gate 1 通过 |
| Chronos/Supervision | 2A–2B | 02 | ~75% | 🟡 Gate 2 接近通过 |
| Space/Agora | 3A–3C | 03 | ~50% | 🔴 基础设施完成，未接入生产 |
| Communication V2 | 4A–4C | 04 | ~40% | 🔴 EnvelopeV2 存在，旧系统未清理 |
| Admission/Security | 5A–5B | 05 | ~20% | 🔴 类型齐全，无生产实现 |
| Service Ports | 6A | 06 | 0% | 🔴 未开始 |

**总体评估**: M0 完整交付，Phase 1–2 核心代码存在且已接入生产路径，Phase 3–5 基础设施（类型、trait）已定义但**未接入生产路径**或仅有测试实现。

---

## 2. 已完成（含证据）

### M0: 唯一 Turn 执行路径 ✅

| 验收项 | 证据 |
|---|---|
| daemon 和 exec 共用 TurnService | `bin/main.rs:350` `TurnService::new(...)` / `daemon_turn.rs:118` `execute_turn` → `daemon_react.rs:24` `submit_streaming_daemon_turn` |
| ReActLoop 不在 handler/bin 中直接创建 | `rg 'ReActLoop::new\|build_harness' crates/executive crates/bin` → 零命中 |
| 唯一生产 harness factory | `harness_factory.rs:17-20` `build_configured_react_loop` / `build_react_loop` |
| `handle_chat` 不再编排 Cognit | `chat.rs:413-422` — 3 行 delegate 到 `turn_orchestrator.execute_turn()` |
| SandboxFirst fail closed | `rg 'Proceeding without sandbox\|selffield-note>SandboxFirst' crates/executive/src` → 零命中 |
| 测试覆盖 | 20 个测试文件存在并通过，包括 `sandbox_first_fail_closed.rs`, `turn_service_equivalence.rs` |

**注意**: daemon 和 exec 使用不同的 service 入口（daemon 走 `DaemonTurnOrchestrator::execute_turn`，exec 走 `TurnService::submit()`）。这是因为 daemon 需要 streaming JSON-RPC 响应。两者都接受 `TurnRequest`，满足设计约束。

### ProcessTable + OperationTable ✅

| 组件 | 文件 | 行数 | 证据 |
|---|---|---|---|
| ProcessTable (spawn/signal/wait/inspect/reap) | `kernel/process/table.rs` | 208 | `impl ProcessManager for ProcessTable` (line 129) — 全部 5 方法实现 |
| OperationTable (submit/cancel/wait) | `kernel/operation/table.rs` | 201 | `impl OperationManager for OperationTable` (line 151) — 含父取消传播 |
| OperationScope (JoinSet + CancellationToken) | `kernel/operation/task_group.rs` | 89 | JoinSet + grace period drain |
| SystemClock (wall_now/mono_now) | `kernel/chronos/system_clock.rs` | 74 | `impl Clock for SystemClock` (line 25) |
| TestClock (deterministic advance) | `kernel/chronos/system_clock.rs:41-74` | — | `advance(millis)` 原子递增 |
| Timer (is_expired via &dyn Clock) | `kernel/chronos/timer.rs` | 12 | 依赖注入，可测试 |

### SubAgentSpawner 深度集成 ✅

`sub_agent.rs:118-175` `spawn_with_policy()` 执行真实的三步内核事务：
1. `process_table.spawn(SpawnSpec {...})` (line 125)
2. `operation_table.submit(OperationRequest {...})` (line 133)
3. 注册到 `supervisor.supervise(process.id, restart_policy)` (line 160)

生命周期方法全链路：`transition()` (line 206) 映射 SubAgentState → ProcessSignal → ProcessTable。`destroy()` (line 328) 执行完整内核清理。

### SupervisorTree 生产使用 ✅

- `daemon_turn.rs:225-228` — 主 agent 注册为 `RestartOnFailure { max_restarts: 3 }`
- `sub_agent.rs:74,100,160,268-290` — 子 agent 退出时 `record_exit` 决策重启
- 7 个监督测试全部通过

### Agora 事务基础设施 ✅

| 组件 | 证据 |
|---|---|
| Workspace version 计数器 | `workspace.rs:31` `pub version: u64` |
| propose/commit/conflict | `workspace.rs:57-92` — 乐观并发控制 |
| AgoraCommit log | `workspace.rs:79-92` |
| 持久化 append-only commit | `persistence.rs:19-53` — `AgoraPersistence` trait + `InMemoryCommitLog` |
| 恢复 replay | `ops.rs:47-72` `recover_session()` |
| 35 个测试全部通过 | `cargo test -p agora` → 35 passed |

### EnvelopeV2 类型定义 ✅

`envelope_v2.rs:128` — 所有字段：id, schema, source, target, pattern, operation_id, causation_id, correlation_id, namespace, logical_time, deadline, priority, payload。含 `from_legacy()` 转换器 (line 252)。

### Admission 类型全集 ✅

`types/admission.rs` (366 行) — 定义了设计文档中的所有类型：PermitId, AdmissionRequest, ExecutionPermit, SandboxRequirement, BudgetReservationId, ResourceLeaseId, UsageReport, AuditEventId, AdmissionError (10 变体)。

---

## 3. 部分完成（含缺失项）

### Supervision: 缺少组级策略 🟡

- ✅ 已有: `RestartPolicy::Never`, `RestartPolicy::RestartOnFailure { max_restarts }`
- ❌ 未实现: `OneForOne`, `OneForAll`, `RestForOne`
- 影响: 子 agent 批量失败时无协调恢复策略

### Sub-Agent 执行: 生命周期有，实际工作是 Stub 🟡

`sub_agent.rs:149-152` — 生成的 tokio task 只 `await token.cancelled()`。子 agent 的 LLM/工具执行尚未接线。进程管理 (spawn/signal/wait/reap) 完全可用，但"执行什么"是空壳。

### Orchestrator 仍有 deprecated ReActLoop 🟡

`orchestrator.rs:37` `react_loop: ReActLoop` 字段仍在，`process_react` (line 386) 标记 `#[deprecated]`。设计说 PR-0E 后应删除，目前是 deprecated 但未删除。

### SpaceManager: 缺少 snapshot 方法 🟡

`kernel/space/manager.rs:39-59` — `fork_space` 和 `attach_region` 已实现，但 `VersionedOverlay` 存入后从不读取/修改。没有 `snapshot()` API 暴露 overlay 内容。

### AgoraOps: 缺少 reject 方法 🟡

`include/agora.rs:74` `AgoraOps` trait — 有 `propose/commit/changes_since` 但没有 `reject` 方法。设计文档 (`03_CONTEXT_SPACE_AND_AGORA.md:99`) 明确要求 `reject(id, reason)`。

### MailboxService: 缺少独立的 request/signal 方法 🟡

`mailbox.rs:58,83` — `Mailbox` trait 有 `send/recv/close`，`MailboxService` 有 `register/unregister/route`。但 `request` 是自由函数 (`request_response`, line 264) 不是 trait 方法。`signal` 方法不存在。

### Agora 未接入生产路径 🔴

**关键差距**: Agora 的事务模型 (propose/commit/conflict) 在 `crates/agora/` 内完整实现并测试，但在 `crates/executive/src/` 中**零引用**。

- 生产路径仍使用旧的 `commit_agora_snapshot` (`chat.rs:400-411`) — 基于字符串快照
- `rg 'propose\(|commit\(|AgoraProposal|VersionConflict' crates/executive/src/` → 零命中

### 旧 Event/EventBus 未清理 🔴

- `impl Event for` 出现在 3 个文件: `events/event.rs:169`, `events/subscription.rs:120`, `bus/in_process.rs:417`
- `EventBus` 引用在 10+ 文件: `communication_bus.rs` (10 次), `kernel_bus.rs` (5 次), `orchestrator.rs` (2 次), `runtime_core.rs` (2 次)
- `LegacyEventBridge` 在设计文档中出现但**未实现**（零 Rust 代码引用）

---

## 4. 未完成（只有类型/stub 或未开始）

### Admission/Security: 只有测试实现 🔴

| 组件 | 状态 |
|---|---|
| `AdmissionController` trait | ✅ 定义完整 |
| 生产 `AdmissionController` | ❌ 不存在，只有 `AllowAllAdmissionController` (TESTING ONLY) |
| `CapabilityInvoker` trait | ✅ 定义完整 |
| 生产 `CapabilityInvoker` | ❌ 只有 `DefaultCapabilityInvoker<A, E>` 泛型壳，未接入 ToolRunner |
| ToolRunner 要求 `ExecutionPermit` | ❌ `tool_executor.rs:74` `execute()` 无 permit 参数 |
| `BudgetController` | ❌ 零 Rust 代码引用 |
| `QuotaManager` | ❌ 零 Rust 代码引用 |
| `ResourceLease` 管理器 | ❌ 只有 newtype ID (`ResourceLeaseId`)，无管理逻辑 |

### Service Ports + CoreSystems 收缩 (PR 6A) 🔴

完全未开始。设计文档 `06_PR_PLAN_AND_ACCEPTANCE.md` 中列为最后一个 PR。

### 旧执行入口清理 🔴

- `orchestrator.rs` 的 `react_loop: ReActLoop` 已标记 deprecated 但未删除
- `controller.rs` 已标记 deprecated 并持有 `turn_service: Arc<TurnService>` 但旧的 `CoreSystems` 集成仍在

---

## 5. 测试覆盖矩阵

| 测试文件 | 对应阶段 | 状态 |
|---|---|---|
| `turn_service_equivalence.rs` | M0 | ✅ |
| `turn_pipeline_order.rs` | M0 | ✅ |
| `sandbox_first_fail_closed.rs` | M0 | ✅ |
| `cognitive_session.rs` | M0 | ✅ |
| `process_table.rs` | Phase 1 | ✅ |
| `operation_tree.rs` | Phase 1 | ✅ |
| `chronos.rs` | Phase 2 | ✅ |
| `supervision.rs` | Phase 2 | ✅ (7 passed) |
| `context_space.rs` | Phase 3 | ✅ (4 passed) |
| `agora_integration.rs` | Phase 3 | ✅ (3 passed) |
| `capability_invoker.rs` | Phase 5 | ✅ (8 passed) |
| `process_messaging.rs` | Phase 4 | ✅ |

**所有测试通过，无失败。**

---

## 6. 优先级路线图

### 立即做（P0 — 安全/一致性风险）

1. **生产 AdmissionController 实现** — 当前所有能力调用绕过准入控制。SandboxFirst 已 fail-closed 但缺少通用的 Permit 门控。实现 `ProductionAdmissionController` 并接入 `CapabilityInvoker`。
2. **CapabilityInvoker → ToolRunner 接线** — 修改 `tool_executor.rs` 的 `execute()` 要求 `ExecutionPermit` 参数。
3. **清理 deprecated 执行路径** — 删除 `orchestrator.rs` 的 `react_loop` 字段和 `process_react` 方法。确保没有残留的未被 TurnService 覆盖的认知循环。

### 短期（P1 — 架构完整性）

4. **Agora 事务模型接入生产** — 将 `commit_agora_snapshot` (字符串快照) 替换为 `propose → validate → commit` 事务流程。
5. **旧 Event/EventBus 清理** — 实现 `LegacyEventBridge`，迁移 `communication_bus.rs` 到 EnvelopeV2，统计旧 `impl Event for` 使用点。
6. **Sub-agent 执行对接** — 替换 `sub_agent.rs:149-152` 的取消等待 stub 为真实的 LLM/工具执行。

### 中期（P2 — 功能完整性）

7. **Budget/Quota/Lease 控制器** — 类型已就绪，需要实现实际管理逻辑。
8. **Agora reject 方法** — 补充 `AgoraOps::reject` 和 propose-reject 工作流。
9. **MailboxService 扩展** — 增加 `request()` 和 `signal()` 作为 trait 方法。
10. **SupervisorTree 组策略** — 实现 OneForOne/OneForAll/RestForOne。
11. **Service Ports + CoreSystems 收缩** (PR 6A)。

---

## 7. 风险与关注点

### 安全风险

- **Admission 空壳**: 所有能力调用无准入控制。`AllowAllAdmissionController` 只有测试用途，误接入生产路径会导致所有 `warn!` 日志被忽略。
- **ToolRunner 无 Permit**: `tool_executor.rs:74` 的 `execute()` 方法签名不要求 `ExecutionPermit`。即使 AdmissionController 实现后，ToolRunner 也需同步修改。

### 架构风险

- **Agora 双轨**: 事务模型 (propose/commit) 已在 `agora/` 内实现但生产路径使用旧的字符串快照。长期双轨导致语义分裂。
- **通信双轨**: `CommunicationBus` (10 处 EventBus 引用) 与 EnvelopeV2+Mailbox 并存。新代码使用哪个取决于调用方选择。
- **3 个 deprecated 但未删除的认知入口**: `orchestrator.react_loop`, `controller.turn_service` (作为兼容层), `chat.rs` 的旧 helper 方法。长期存在导致维护负担。

### 测试风险

- Sub-agent 执行 stub 意味着 `sub_agent.rs` 的测试只覆盖生命周期状态机，不覆盖实际执行。
- 15 个新测试全绿是好事，但 `capability_invoker` 测试只用 `AllowAllAdmissionController` — 从未测试真实拒绝路径。

---

## 8. 验收命令（当前应全部通过）

```bash
# M0 gate
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test -p fabric turn
cargo test -p cognit cognitive_session
cargo test -p executive turn_service
cargo test -p executive sandbox_first_fail_closed
rg "ReActLoop::new|build_harness" crates/executive crates/bin
rg "Proceeding without sandbox|selffield-note>SandboxFirst|sandbox review" crates/executive/src

# Phase 1-2
cargo test -p executive process_table operation_tree chronos supervision

# Phase 3-5
cargo test -p agora
cargo test -p executive context_space agora_integration capability_invoker process_messaging
```

预期: 全部 PASS，安全字符串零命中。

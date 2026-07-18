# G6 可执行 Spec：子 Agent 资源结算与 Reparent

> 对应研究文档 `../07-subagent-resource-settlement.md`。优先级 P2（依赖 AgentControl、G2、G4 部分能力）。
> 实施前按 `README.md §5` 重新核对 §2 锚点。

## 1. 目标与非目标

**目标**：把子 Agent 完成时的资源处置从当前**内联在 `run_agent`** 的逻辑，升级为**显式结算状态机**（Quiescing→Settling→Terminal）；新增**后台资源分类**（前台/后台）与 **reparent 协议**（`survive_child` 声明 + reparent receipt）；结算幂等（防重复释放/重复 promotion）。

**非目标**：
- 不替换现有 admission settle/revoke、lease 删除、budget reservation、memory promotion（复用/编排）。
- 不改 `AgentControlPort`/`AgentSpawnRequest` 权威模型。
- 第一版 reparent 只支持 background command + notification route；scheduler reparent 后续。

## 2. 当前代码锚点（已验证 @ commit bec15695）

| 符号 | 位置 | 关键事实 |
|---|---|---|
| `AgentSpawnRequest` | `crates/fabric/src/types/agent_control.rs:190-207` | root/parent agent、parent_process、runtime、trusted_workspace、context、broadcast_refs、allowed_tools、budget。**无 survive_child** |
| `AgentControlPort` | 同上 `:506-528` | spawn/wait/send/cancel/inspect/list |
| `AgentBudget` | 同上 `:157-187` | max_input/output_tokens、max_tool_calls、max_elapsed_ms、max_cost_usd、max_depth |
| `RuntimeResumability`/`AgentRecoveryDecision`/`AgentRecoveryReceipt` | 同上 `:26-60` | Never/Checkpointed；Interrupt/Resume/Finalize/Reclaim；receipt 带 idempotency_key |
| `AgentControlService` | `crates/executive/src/service/agent_control/mod.rs:100-152` | 持 kernel/repository/admission/runtimes/events/event_spine/live/tasks(JoinSet)/agent_memory_vault |
| 完成流程 `run_agent` | 同上 `:1026-1207` | 终态：settle usage(1191-1195)、删 lease(1200-1205)、live.remove(1206) |
| lease 创建 | 同上 `:768-780` | 三 lease：admission/mailbox/execution |
| `AgentResourceLease` | `crates/executive/src/service/agent_control/repository.rs:84-94` | lease_key/agent_id/kind(Admission/Mailbox/Execution/Worktree)/owner/expires_at_ms/worktree_* |
| budget reserve/settle/revoke | `crates/executive/src/service/agent_control/admission.rs:184-441` | reserve_child(313-321)、settle_reservation(390-420)、revoke_reservation(423-441)；`BoundedAdmissionLease::Drop`(445-467) async revoke fallback |
| memory promotion | `crates/executive/src/service/agent_control/memory.rs:51-75` | `promote_reviewed(...)→MemoryPromotionReceipt`；`record_child(context, ChildMemoryDraft)`(117-127) |
| live runs | `crates/executive/src/service/agent_control/live_runs.rs:16-36` | snapshots/mailbox_target/cancellation。**无 background monitor 跟踪** |

**核心事实**：结算已有（usage settle/revoke、lease 幂等删除 by (key,owner)、budget drop fallback、memory promotion receipt），但 **(a) 无显式状态机、(b) 无前台/后台分类、(c) 无 reparent、(d) 无统一幂等结算 receipt**。

## 3. 权威归属决策（doc10 §6 八问）

1. **owner**：Fabric 定义结算类型（resource class、settlement receipt、reparent receipt）；`AgentControlService` 拥有结算状态机编排。
2. **scope**：结算 receipt 按 `agent_id + attempt_id + generation` 幂等键。
3. **crash 恢复**：结算幂等重放；daemon crash 后用现有 `AgentRecoveryDecision`（interrupt/resume/finalize/reclaim）决定资源处置。
4. **fail 模式**：reparent 条件不满足 → cancel/kill 资源并写 terminal evidence；结算失败不重复释放。
5. **上限**：后台资源数上限；reparent 数上限。
6. **兼容**：flag 关闭 → 结算走当前内联路径（等价）。
7. **进 event spine**：结算状态转移、reparent receipt 经 `publish_event_v2`。
8. **许可证**：重新实现结算/reparent 语义，不复制 Grok `subagent/mod.rs`。

## 4. 类型定义

### 4.1 Fabric 类型 — `crates/fabric/src/types/agent_settlement.rs`（新文件）

```rust
use serde::{Deserialize, Serialize};

/// 资源类别：决定结算时的默认处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentResourceClass {
    /// 前台命令：child 退出前必须 settle（await 或 cancel）。
    ForegroundCommand,
    /// 后台命令：可 kill / reparent / detach 三选一。
    BackgroundCommand,
    /// 通知路由：child-specific，退出时切 parent 或 durable mailbox。
    NotificationRoute,
    /// worktree：child-owned，清理 / 保留 artifact / 进 recovery。
    Worktree,
}

/// 后台资源的退出处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackgroundDisposition { Kill, Reparent, Detach }

/// 结算状态机。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettlementPhase {
    Running,
    /// 停收新调用、取消/await 前台、分类后台、flush events/usage/memory/artifact。
    Quiescing,
    /// reparent 授权存活者、释放 lease/reservation、必要时持久 recovery checkpoint。
    Settling,
    Terminal,
}

/// 终态类别。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettlementTerminal { Completed, Failed { reason: String }, Cancelled, Recoverable }

/// 幂等结算 receipt。防重复释放/重复 promotion。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementReceipt {
    pub agent_id: String,
    pub attempt_id: String,
    /// daemon 代际，crash 恢复用。
    pub generation: String,
    pub terminal: SettlementTerminal,
    /// 已释放的 lease_key 集合（幂等）。
    pub released_leases: Vec<String>,
    /// 已 reparent 的资源。
    pub reparented: Vec<ReparentReceipt>,
    pub settled_at_ms: i64,
    pub idempotency_key: String,
}

/// reparent 不可变凭证。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReparentReceipt {
    pub resource_id: String,
    pub class: AgentResourceClass,
    pub old_owner: String,
    pub new_owner: String,
    pub reason: String,
    pub at_ms: i64,
}

/// 后台资源必须显式声明才可 reparent（不是 child 临终自升级）。
/// 追加到 AgentSpawnRequest（见 §5）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundResourceDecl {
    pub resource_id: String,
    pub class: AgentResourceClass,
    /// spawn 时声明是否可存活于 child 之后。
    pub survive_child: bool,
}

pub const MAX_BACKGROUND_RESOURCES: usize = 64;
```

### 4.2 结算编排 — `crates/executive/src/service/agent_control/settlement.rs`（新文件）

```rust
use fabric::types::agent_settlement::*;

/// 结算引擎：把 run_agent 终态内联逻辑抽为显式状态机。
/// 复用现有 admission.settle/revoke、lease 删除、memory promotion。
pub struct SettlementEngine;

impl SettlementEngine {
    /// Quiescing：停收新调用、取消/await 前台、分类后台、flush。
    /// 返回待结算的资源清单。
    pub async fn quiesce(&self, /* agent ctx, live run */) -> Vec<BackgroundResourceDecl> { unimplemented!() }

    /// Settling：对每个资源施加处置。reparent 需满足全部条件（见 §4.3 规则），
    /// 否则 kill 并记 terminal evidence。释放 lease/reservation（幂等）。
    /// 产出 SettlementReceipt。
    pub async fn settle(
        &self,
        resources: Vec<BackgroundResourceDecl>,
        /* parent authority, budget, mailbox */
    ) -> SettlementReceipt { unimplemented!() }
}
```

### 4.3 Reparent 规则（编码为断言）

后台资源仅当**全部**满足才可 reparent，否则 kill/detach：

1. spawn 时 `survive_child=true`（非 child 临终升级）。
2. parent 的 workspace/capability authority 覆盖该资源。
3. parent budget 接受剩余 cost/time reservation。
4. notification route 能切 parent 或 durable mailbox。
5. 产出不可变 `ReparentReceipt`。

### 4.4 已裁决冲突（2026-07-18）

| 冲突 | 裁决 | 失败策略 |
|---|---|---|
| budget transfer 与 settlement receipt 的 crash window | reservation/transfer 必须由 durable BudgetController 以单一幂等 transfer identity 落盘；transfer receipt durable 后才允许发布 reparent authority，settlement replay 读取同一 receipt，不重复扣减或返还 | durable transfer receipt 不可确认时禁止 reparent，资源 kill |
| `kill/detach` 的默认边界 | host-reviewed disposition 仅默认允许 `Kill` / 满足 §4.3 的 `Reparent`；`Detach` 需要显式 host authorization，child/model 请求不构成授权 | 未授权 `Detach` 返回拒绝并 kill，不降级为隐式 detach |

上述裁决覆盖原 §4.3 “否则 kill/detach” 的歧义：`detach` 不是普通 fallback。

## 5. 文件变更计划

| 动作 | 文件 | 理由 |
|---|---|---|
| 新增 | `crates/fabric/src/types/agent_settlement.rs` | 结算/reparent 类型 |
| 修改 | `crates/fabric/src/types/mod.rs` + `lib.rs` | 导出 |
| 修改 | `crates/fabric/src/types/agent_control.rs:190-207` | `AgentSpawnRequest` 追加 `background_decls: Vec<BackgroundResourceDecl>`（默认空） |
| 新增 | `crates/executive/src/service/agent_control/settlement.rs` | 结算状态机 |
| 修改 | `crates/executive/src/service/agent_control/mod.rs:1026-1207` | `run_agent` 终态改调 `SettlementEngine`（flag 后） |
| 修改 | `crates/executive/src/service/agent_control/live_runs.rs:16-36` | 跟踪后台资源句柄（供 quiesce 分类） |
| 修改 | feature flag | `grok_hardening.subagent_settlement` 默认关 |

## 6. 任务分解（TDD）

**阶段 A：类型**
- T1. 新建 `agent_settlement.rs` 全类型。`cargo check -p fabric`。
- T2. `AgentSpawnRequest` 追加 `background_decls`（默认空，`#[serde(default)]`）。确认现有构造点编译（默认空 = 当前行为）。

**阶段 B：结算状态机（复用现有释放逻辑）**
- T3. `SettlementEngine::quiesce`：分类 live run 的资源为前台/后台。单测（mock live run）。
- T4. `settle`：释放 lease 复用现有 by-(key,owner) 幂等删除；产出 `SettlementReceipt.released_leases`。单测：二次 settle 不重复释放（幂等键）。
- T5. budget：settle 复用 `settle_reservation`（成功）/`revoke_reservation`（失败）。单测。
- T6. memory：child draft 未 promote 不泄漏到 parent scope（复用 `promote_reviewed` 门）。单测。

**阶段 C：reparent 规则**
- T7. reparent 五条件断言。survive_child=false → 强制 kill。单测。
- T8. 条件不满足（如 parent budget 拒绝剩余 reservation）→ kill + terminal evidence。单测。
- T9. 满足 → 产出 `ReparentReceipt`（old/new owner、reason）。单测。
- T10. notification route reparent：切 parent 或 durable mailbox。单测。

**阶段 D：前台/取消传播**
- T11. 前台命令 child 退出前必须 settle（await 或 cancel）→ 无 orphan。单测。
- T12. parent cancel 传播到所有 child 与不可存活资源（复用现有 cancellation token）。单测。

**阶段 E：crash 恢复**
- T13. 结算 receipt 幂等重放（`agent_id+attempt_id+generation`）。单测。
- T14. daemon crash 后用 `AgentRecoveryDecision` 区分 reclaim/resume/finalize 资源。单测（mock recovery）。

**阶段 F：集成（flag 后）+ 收尾**
- T15. `run_agent:1026-1207` 终态改调 `SettlementEngine`；flag 关走当前内联路径（回归等价）。集成测试。
- T16. 结算状态转移 + reparent receipt 经 `publish_event_v2`。事件断言。
- T17. clippy/fmt；更新 §2 漂移；标注 flag 灰度。

## 7. 兼容与迁移

- **flag 关闭**：`run_agent` 走当前内联 settle/revoke/删 lease/remove（等价当前）。
- **`background_decls` 默认空**：现有 spawn 无后台资源声明 = 无 reparent，行为不变。
- **复用而非替换**：admission、lease、budget、memory promotion 的既有实现全部复用，`SettlementEngine` 只编排 + 加分类/reparent/状态机/幂等 receipt。
- **依赖**：worktree 结算与 G4 workspace identity 关联；后台命令流可观测性与 G2 关联。

## 8. 测试计划（映射研究文档 ../07 §7 验收方向）

| 验收方向 | 测试 |
|---|---|
| child 退出后无 orphan 前台进程 | T11 |
| 批准的后台任务继续并向 parent/mailbox 报告 | T9, T10 |
| parent cancel 传播到所有 child 与不可存活资源 | T12 |
| usage/lease/budget 只结算一次 | T4, T5, T13 |
| child memory 未 promotion 不进 parent scope | T6 |
| crash recovery 区分 reclaim/resume/finalize | T14 |

## 9. 可观测性

- 事件：`agent.settlement.phase`（Quiescing/Settling/Terminal）、`agent.reparent`（old/new owner、resource、reason）、`agent.orphan_killed`。
- 指标：`settlement_duration_ms{agent}`、`reparent_total{class}`、`reparent_denied_total{reason}`、`settlement_idempotent_replay_total`。

## 10. 许可证

重新实现结算状态机与 reparent 规则，不复制 Grok `xai-grok-shell/agent/subagent/mod.rs`。幂等 receipt 模型复用 Aletheon 现有 `AgentRecoveryReceipt` 思路。无 NOTICE 变更。

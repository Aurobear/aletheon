# Agora 事务完整性 — 文档声明 vs 代码实际

## 概述

架构文档 `docs/plans/2026-07-16-a01-agora-transaction-integrity.md` 描述了 5 个 CRITICAL 级别的事务完整性 bug。通过逐行扫描 `crates/agora/src/` 下所有模块，**验证结果：全部 5 个 bug 已在当前代码中修复。Agora 达生产级质量，远超过文档描述的成熟度。**

## 关键背景

`2026-07-16-a01-agora-transaction-integrity.md` 是 A01 实施计划，描述的是**修复前的状态**（PRE-FIX）。计划中所有 6 个 task 均标记为 `[x]`（已完成）。当前代码是**修复后的状态**（POST-FIX）。

---

## Bug #1: "Boolean-only commit authorization" — FALSE

**文档声称:** `CommitPermit { authorized: bool }` 无法将决策绑定到具体事务

**当前代码:** `crates/fabric/src/include/agora.rs:233-277`

`WorkspaceCommitPermit` 现在有 7 个字段：`permit_id`、`space`、`proposal_id`、`process`、`operation_hash`（SHA-256）、`expected_version`、`expires_at_ms`。`validate_for()`（行 256-277）独立检查全部 5 维度 + 过期。旧的 `CommitPermit { authorized: bool }` 已完全移除。Round-trip 测试通过（行 474-510）。

**验证结果: FALSE — 已修复。**

---

## Bug #2: "Commit without rechecking base version" — FALSE

**当前代码:** `crates/agora/src/workspace/mod.rs:130-187`

旧的单步 `commit()` 拆分为两阶段：
- `prepare_commit()`（行 130-164）：复查 expiry、space、base version、operation semantics、permit
- `apply_prepared_commit()`（行 166-187）：复查 checksum、space、version contiguity、proposal existence、operation hash，然后才 APPLY
- 旧方法（行 190-195）标记为 `#[deprecated]`

**验证结果: FALSE — 已修复。**

---

## Bug #3: "Claim/release ignore ownership" — FALSE

**当前代码:** `crates/agora/src/workspace/mod.rs:293-305`

`validate_operation`: `ClaimSharedObject` 拒绝已存在的 claim；`ReleaseSharedObject` 仅允许 claim 所有者释放。测试 `commit_claim_release_manages_claims`（行 631-661）和集成测试 `transaction_claim_and_release_require_current_owner`（`tests/transaction_integrity.rs:83-130`）验证所有权强制执行。

**验证结果: FALSE — 已修复。**

---

## Bug #4: "Global mutex held across I/O" — FALSE

**当前代码:** `crates/agora/src/ops/mod.rs:25-37`

`SpaceSlot` — 每个 session 独立 `Mutex<Workspace>` + 独立 `Mutex<()>` commit gate。`commit_transaction()`（行 78-108）：获取 workspace 锁 → prepare_commit → **释放锁** → 持久化 I/O → 重新获取锁 → apply。测试 `durability_io_does_not_hold_workspace_state_lock`（`tests/transaction_integrity.rs:211-266`）证明并发 session 在 I/O 期间不被阻塞。

**验证结果: FALSE — 已修复。**

---

## Bug #5: "Linear proposal scan" — FALSE

**当前代码:** `crates/agora/src/ops/mod.rs:20`

`proposal_index: Mutex<HashMap<Uuid, String>>` — O(1) 查找，在 propose/commit/reject 时维护。

**验证结果: FALSE — 已修复。**

---

## 新发现：模块成熟度远超文档描述

### Competition — 完全实现

**文件:** `crates/agora/src/competition/mod.rs`

`CandidatePool`（行 148-480）：去重 + per-source 容量 + 总容量 + `select()`（行 237-341）多维评分：加权 salience、aging boost、依赖 boost、重复惩罚、refractory penalty、ignition 阈值、coalition building。`SelectionPolicy`（行 10-74）10 参数可配置。8 个 `AdmissionMetrics` 计数器。

### Broadcast — 真正 push-based 系统

**文件:** `crates/agora/src/broadcast/mod.rs`

`BroadcastHub`（行 62-198）：`RwLock<HashMap<ProcessId, ProcessorRegistration>>`，`deliver()` 并发 `tokio::spawn` + `Semaphore` + 5s 超时 + 可见性过滤。`SqliteBroadcastStore` checksum 验证 + epoch 追踪 + 幂等追加。

### 生产集成

`crates/executive/src/service/turn_pipeline.rs:448-456` 使用 `WorkspaceCommitPermit::issue_for(...)` → `agora.commit_with_permit(...)`。无 permit 的 commit 路径已消除。

---

## 新发现的真实 gap

### Attention — 死状态（MEDIUM）

**文件:** `crates/agora/src/attention/mod.rs:7-31`

`Attention { focus, priorities }` 被声明、序列化、回放，但**没有任何 `AgoraOperation` 变体修改它**。`set_focus()`/`clear_focus()` 仅被测试调用（grep 确认零生产调用方）。不与 competition 集成。

### Blackboard — 无类型 JSON（MEDIUM，设计选择）

**文件:** `crates/agora/src/blackboard/mod.rs:9-62`

`HashMap<String, Value>`（`Value = serde_json::Value`），无 schema 执行。文档化的 "schema-flexible by design"。

### TaskGraph — 无状态转换执行（LOW-MEDIUM）

**文件:** `crates/agora/src/task_graph/mod.rs:25-91`

`HashMap<String, TaskNode>` 仅 CRUD + `ready()` combinator。无状态转换验证（Done→Pending 被允许）、无调度引擎。

---

## 测试覆盖

**文件:** `crates/agora/tests/transaction_integrity.rs`（301 行）

并发 commit、耐久性失败、锁分离、恢复回放、幂等重放 — 全部覆盖。

---

## 文档更新状态 (2026-07-17)

本报告中的发现已同步到 `docs/plans/2026-07-16-a01-agora-transaction-integrity.md`（顶部新增 "Code-Reality Update (2026-07-17)" 章节），包括：
- 5 个 CRITICAL bug 全部标记为 FIXED（7 字段 WorkspaceCommitPermit + SHA-256、两阶段提交、所有权验证、per-session 锁、O(1) 索引）
- 3 个新发现的 gap：Attention 死状态、Blackboard 无类型、TaskGraph 无状态转换验证

原始 A01 计划内容完整保留，仅前置代码实际状态说明。

---

## 总结表

| 文档声称的 Bug | 代码验证 | 修复方式 |
|---------------|----------|----------|
| Boolean-only commit permit | **FALSE — 已修复** | `WorkspaceCommitPermit` 7 字段 + SHA-256 |
| Commit 不复查 base version | **FALSE — 已修复** | 两阶段 prepare + apply |
| Claim/release 忽略所有权 | **FALSE — 已修复** | `validate_operation` 强制执行 |
| 全局 mutex 跨 I/O 持有 | **FALSE — 已修复** | `SpaceSlot` per-session 独立锁 |
| 线性 proposal 扫描 | **FALSE — 已修复** | O(1) `proposal_index` |

| 模块 | 成熟度 | 状态 |
|------|--------|------|
| 事务完整性 | **生产级** | 5/5 bug 修复 |
| Competition | **生产级** | 完整多维评分管线 |
| Broadcast | **生产级** | 真正 push + 持久化 ack |
| Attention | **死状态** | 声明但从未驱动 |
| TaskGraph | 框架级 | 缺状态转换执行 |

**核心结论：Agora 不是 "buggy prototype"，而是已完成核心事务完整性的生产级实现。文档严重低估了 Agora 的当前状态。**

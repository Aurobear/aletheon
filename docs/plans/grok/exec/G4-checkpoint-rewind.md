# G4 可执行 Spec：Workspace Checkpoint 与 Rewind

> 对应研究文档 `../06-workspace-checkpoint-and-rewind.md`。优先级 P1（高风险，Conscious-core/AgentControl 稳定后启动）。
> 实施前按 `README.md §5` 重新核对 §2 锚点。
>
> **状态：已实施并通过聚焦验收（2026-07-18）。** G4 第一版范围内的 T1–T13 已完成；全工作区最终回归仍由总执行计划统一进行。

## 1. 目标与非目标

**目标**：在 turn/prompt 边界捕获工作区 FS 状态；用户显式请求"rewind to turn N"时以**事务语义**恢复（部分失败可解释、不静默丢未提交修改）；把 workspace checkpoint 与 runtime checkpoint、memory/event history 明确分离；rewind 追加 `WorkspaceRewound` 事件而非抹除历史。

**非目标**（分阶段，第一版只做 POC）：
- 第一版仅 **FS domain、单 Agent、用户显式触发**。
- 不做 git index/HEAD 语义、patch/hunk attribution、multi-agent 协调（后续阶段）。
- 模型不能提供任意文件路径或 checkpoint blob（只能请求 turn N）。
- 不通过 FS rewind 删除 memory/event 历史。

## 2. 当前代码锚点（重新验证 @ 2026-07-18）

| 符号 | 位置 | 关键事实 |
|---|---|---|
| Fabric checkpoint 类型 | `crates/fabric/src/types/workspace_checkpoint.rs:36,68,127,152` | workspace identity、turn checkpoint、restore outcome 与 2048 文件硬上限已落地 |
| checkpoint store / service | `crates/executive/src/service/workspace_checkpoint.rs:40,231,295,362,387` | 持久化端口、capture/finalize/rewind 事务编排已落地 |
| SQLite store | `crates/executive/src/impl/session/checkpoint_store_sqlite.rs:24-162` | checkpoint 与文件快照持久化、重启读取、成功后截断 |
| turn 边界 | `crates/executive/src/service/turn_pipeline.rs:129-149,715-727` | 副作用前 begin；整个 turn 的成功/错误结果之后统一 finalize |
| RPC 权威边界 | `crates/executive/src/impl/daemon/handler/rpc/rpc_turn.rs:10-95` | 只收 session/turn；拒绝路径/blob；workspace 从 host-bound thread authority 解析 |
| RPC flag 门控 | `crates/executive/src/impl/daemon/handler/rpc.rs:116-119` | flag 关闭时不暴露 `workspace.rewind` |
| feature flag | `crates/executive/src/core/config/grok_hardening.rs:23-24` | `grok_hardening.workspace_checkpoint`，默认 false |
| canonical events | `crates/fabric/src/ipc/envelope_v2.rs:330-334` | began/finalized/rewound schema 均已注册 |

**当前事实**：G4 使用独立的 FS checkpoint domain，不混入 runtime recovery；SQLite store、排他 lease、child 活跃拒绝、事务 restore 与 canonical event receipt 均已集成。

## 3. 权威归属决策（doc10 §6 八问）

1. **owner**：Fabric 定义 `TurnCheckpoint`/domain ref 类型；Executive 拥有 capture/finalize/restore 编排与持久化；host 铸造所有 ref（模型只给 turn 序号）。
2. **scope**：checkpoint 按 `(session_id, thread_id, turn_id/prompt_index)` + workspace_identity 持久化。
3. **crash 恢复**：checkpoint 持久（sqlite）；非 Completed 结果也须显式 finalize/abort；进程崩溃遗留的 `Open` 在 store/daemon 启动事务中统一改为 `Aborted`，启动完成后不得存在 `Open`；restore 幂等。
4. **fail 模式**：workspace identity 不匹配 → fail closed（不改任何内容）；未跟踪修改无法保护 → abort；FS restore 失败 → **保留 checkpoints 供重试**（不截断）。
5. **上限**：checkpoint 磁盘配额；每 turn 捕获的变更文件数上限；超限告警。
6. **兼容**：flag 关闭 → 不捕获、不暴露 rewind（等价当前）。
7. **进 event spine**：`WorkspaceCheckpointBegan`/`Finalized`/`WorkspaceRewound` 经 `publish_event_v2`；memory/event 历史**不**随 rewind 删除。
8. **许可证**：重新实现 restore 事务顺序语义，不复制 Grok `checkpoint.rs`。

## 4. 类型定义

### 4.1 Fabric 类型 — `crates/fabric/src/types/workspace_checkpoint.rs`（新文件）

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CheckpointId(pub Uuid);
impl CheckpointId { pub fn new() -> Self { Self(Uuid::new_v4()) } }

/// workspace 规范身份，防路径别名/软链绕过。canonical_path 来自
/// WorkspacePolicy 已 canonicalize 的 cwd（local_authority.rs:235-248）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceIdentity {
    pub canonical_path: PathBuf,
    pub repo_fingerprint: Option<String>,
}

/// FS domain 引用：host-minted，指向持久化的文件快照集合。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsDomainRef {
    /// 快照存储中的批次 id。
    pub batch_id: Uuid,
    /// 本 checkpoint 捕获的变更文件数（有上限）。
    pub file_count: usize,
}

/// 一个逻辑 turn checkpoint。第一版只含 fs_domain；vcs/patch/runtime 为后续。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnCheckpoint {
    pub checkpoint_id: CheckpointId,
    pub session_id: String,
    pub thread_id: String,
    pub turn_id: String,
    /// 用户"rewind to turn N"的关联序号。
    pub prompt_index: u64,
    pub workspace: WorkspaceIdentity,
    pub fs_domain: FsDomainRef,
    /// 后续阶段填充；第一版恒 None。
    pub vcs_domain_ref: Option<String>,
    pub patch_domain_ref: Option<String>,
    pub runtime_checkpoint_ref: Option<String>,
    pub created_at_ms: i64,
    pub schema_version: u32,
    pub integrity_digest: String,
    /// finalize 状态：非 Completed turn 也必须显式 finalize/abort。
    pub finalize_state: CheckpointFinalizeState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckpointFinalizeState { Open, Finalized, Aborted }

/// restore 事务阶段结果（部分失败可解释）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RestoreOutcome {
    /// 全部成功。
    Completed,
    /// identity 校验失败——未改动任何内容。
    IdentityMismatch,
    /// 未跟踪修改无法保护——abort，未改动。
    UnprotectedChangesAbort,
    /// FS 核心恢复失败——保留 checkpoints 供重试。
    FsRestoreFailed { detail: String },
    /// 核心成功但后续（index/hunk）部分失败——标记 partial，不伪装成功。
    Partial { detail: String },
}

/// 单文件快照条目（复用 FileSnap 语义）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointFileEntry {
    pub path: PathBuf,
    /// None = 该文件当时不存在（restore 时应删除）。
    pub content: Option<String>,
}

/// 捕获文件数上限。
pub const MAX_CHECKPOINT_FILES: usize = 2048;
```

### 4.2 Executive 端口 — `crates/executive/src/service/workspace_checkpoint.rs`（新文件）

```rust
use async_trait::async_trait;
use fabric::types::workspace_checkpoint::*;

/// checkpoint 持久化 + 快照存储。
#[async_trait]
pub trait CheckpointStore: Send + Sync {
    /// 开始一个 checkpoint（turn 开始时）。
    async fn begin(&self, ck: TurnCheckpoint, files: Vec<CheckpointFileEntry>);
    /// finalize（turn 结束，任何结果都须调用）。
    async fn finalize(&self, id: CheckpointId, state: CheckpointFinalizeState);
    /// 取某 turn 的 checkpoint + 文件（restore 用）。
    async fn load(&self, session: &str, prompt_index: u64)
        -> Option<(TurnCheckpoint, Vec<CheckpointFileEntry>)>;
    /// 截断 turn N 之后的所有 checkpoint（仅核心 restore 成功后调）。
    async fn truncate_after(&self, session: &str, prompt_index: u64);
}

/// checkpoint 编排。capture 在 turn 边界；restore 由用户显式触发。
pub struct WorkspaceCheckpointService {
    store: std::sync::Arc<dyn CheckpointStore>,
    leases: std::sync::Arc<dyn fabric::LeaseManager>,
    feature_enabled: bool,
}

impl WorkspaceCheckpointService {
    /// turn 开始：扫描 writable_roots 内变更集，捕获 begin checkpoint。
    pub async fn begin_turn(&self, /* ctx */) -> Option<CheckpointId> { unimplemented!() }
    /// turn 结束：finalize（Completed/其它都显式）。
    pub async fn finalize_turn(&self, id: CheckpointId, succeeded: bool) { unimplemented!() }
    /// 用户 rewind：取排他 lease → 校验 identity → 保护未跟踪 → FS restore
    /// → 成功后 truncate → 追加 WorkspaceRewound 事件。返回 RestoreOutcome。
    pub async fn rewind_to(&self, session: &str, prompt_index: u64) -> RestoreOutcome { unimplemented!() }
}
```

## 5. 文件变更计划

| 动作 | 文件 | 理由 |
|---|---|---|
| 新增 | `crates/fabric/src/types/workspace_checkpoint.rs` | checkpoint 类型 |
| 修改 | `crates/fabric/src/types/mod.rs` + `lib.rs` | 导出 |
| 新增 | `crates/executive/src/service/workspace_checkpoint.rs` | 编排 + store 端口 |
| 新增 | `crates/executive/src/impl/.../checkpoint_store_sqlite.rs` | 持久 store（sqlite + 快照文件） |
| 修改 | `crates/executive/src/service/turn_pipeline.rs:215,640` | begin_turn / finalize_turn 挂到 turn 边界 |
| 新增 | rewind 触发入口（daemon 命令） | 用户显式 "rewind to turn N" |
| 修改 | feature flag | `grok_hardening.workspace_checkpoint` 默认关 |

## 6. 任务分解（TDD）

**阶段 A：类型**
- T1. 新建 `workspace_checkpoint.rs` 全类型 + `integrity_digest` 计算。`cargo check -p fabric`。单测：digest 稳定。

**阶段 B：变更捕获（复用 FileSnap）**
- T2. 变更扫描：给定 writable_roots，产出 `Vec<CheckpointFileEntry>`（复用 `FileSnap::capture` 语义）。超 `MAX_CHECKPOINT_FILES` → 告警，并持久化**无可恢复 blob 的 `Aborted` 记录**；截断捕获绝不作为可 rewind 的 `Finalized` checkpoint。单测。
- T3. 内存 `CheckpointStore` 替身 + begin/finalize/load/truncate_after 单测。

**阶段 C：restore 事务（核心，最需谨慎）**
- T4. `rewind_to` 阶段 1：identity 校验。workspace canonical_path 不匹配 → `IdentityMismatch`，**零改动**。单测。
- T5. 阶段 2：保护当前未跟踪修改（快照到临时区）；无法保护 → `UnprotectedChangesAbort`，零改动。单测。
- T6. 阶段 3：FS restore（按 CheckpointFileEntry 覆盖/删除）；失败 → `FsRestoreFailed`，**保留 checkpoints**。单测（模拟写失败）。
- T7. 阶段 4：核心成功后 `truncate_after`；再追加 `WorkspaceRewound` 事件。单测：truncate 只在成功后发生。
- T8. 属性测试：新增/修改/删除文件三种变更 rewind 后状态正确。

**阶段 D：排他 + 事件**
- T9. rewind 前经 `LeaseManager::acquire` 取 workspace 排他 lease；持有期间第二 rewind 被拒。单测。
- T10. 事件：`WorkspaceCheckpointBegan`/`Finalized`/`WorkspaceRewound` 经 `publish_event_v2`；**断言 memory/event 历史未被删除**。单测。

**阶段 E：turn 边界集成（flag 后）**
- T11. `turn_pipeline.rs:215` begin_turn；`:640` finalize_turn（succeeded 与否都 finalize，不留 Open）。集成测试：turn 后 checkpoint 处于 Finalized。
- T12. flag 关闭回归：不捕获、无 rewind 入口，行为等价当前。

**阶段 F：收尾**
- T13. clippy/fmt；更新 §2 漂移；标注 flag + 磁盘配额 + 灰度。

## 7. 兼容与迁移

- **flag 关闭**：无捕获、无 rewind（等价当前；FileSnap 仍未被其他路径使用）。
- **分阶段**：第一版仅 FS + 单 Agent + 显式触发，并包含 SQLite durability 与启动时 `Open → Aborted` crash reconciliation。后续仅指 patch/hunk、git、runtime-domain 与 multi-agent 协调（每阶段独立 flag + telemetry + 配额），不再把基础 durable/crash reconciliation 延后。
- **runtime vs workspace**：本 spec 只碰 workspace FS；runtime checkpoint（`RuntimeResumability`）是 AgentControl 域，只做可选 ref 关联不嵌入。
- **多 Agent**：第一版有活跃 child 时**拒绝** rewind（后续做协调取消）。

## 8. 测试计划（映射研究文档 ../06 §8 验收方向）

| 验收方向 | 测试 |
|---|---|
| rewind 恢复新增/修改/删除文件 | T8 |
| 当前未提交修改不静默丢失 | T5 |
| 失败不截断可重试 checkpoint | T6, T7 |
| workspace identity 不匹配 fail closed | T4 |
| child 活跃时无跨 Agent 竞态 | T9 + 活跃 child 拒绝（阶段 D 扩展） |
| event history 保留并追加 rewind receipt | T10 |

## 9. 可观测性

- 事件：`workspace.checkpoint.began`/`finalized`（file_count、turn_id）、`workspace.rewound`（from_prompt_index、outcome）。
- 指标：固定 6 项 aggregate（**无 session/thread/workspace label**）：`checkpoint_files_captured_total`、`checkpoint_disk_bytes`、`rewind_partial_total`、`rewind_identity_mismatch_total`、`checkpoint_quota_rejected_total`、`checkpoint_startup_aborted_total`。
- 日志：restore 各阶段结果；FS restore 失败带保留 checkpoint 提示。

**2026-07-18 用户裁定**：启动 reconciliation 属于本期 durability，不是 future phase；session 仅可进入结构化日志/事件，不得作为 checkpoint metric label；文件数截断或 quota 超限均保留 `Aborted` 审计记录而非可恢复的部分 checkpoint。

## 10. 许可证

重新实现 restore 事务顺序（identity→protect→fs→truncate）语义，不复制 Grok `xai-grok-workspace/session/checkpoint.rs`。无 NOTICE 变更。

## 11. 实施验收记录（2026-07-18）

- T1–T10：`service::workspace_checkpoint::tests` 8/8 通过，覆盖稳定有界捕获、成功/失败终态、identity fail-closed、当前修改保护失败、restore 失败保留未来 checkpoint、排他 lease、child 活跃拒绝、新增/修改/删除恢复，以及 event bus/spine receipt。
- T3 crash/reopen：`impl::session::checkpoint_store_sqlite::tests` 1/1 通过。
- T11：生产 pipeline 在 `turn_pipeline.rs:129-149,715-727` 包围完整 turn；`every_turn_result_leaves_a_terminal_checkpoint` 证明成功为 Finalized、失败为 Aborted，均不留 Open。
- T12/RPC authority：`workspace_rewind_tests` 1/1、`thread_authority::tests` 2/2 通过；请求路径/blob 被拒，重启后仍从 host-bound authority 恢复 workspace。
- T13：`bash scripts/cargo-agent.sh check -p executive` 与 `fmt --all -- --check` 通过；`clippy -p executive --all-targets -- -D warnings` 被既有 integration support dead-code 阻断（`tests/support/mock_llm_provider.rs:41,106,122`、`mock_sandbox.rs:66`），G4 生产目标改以 `clippy -p executive --lib -- -D warnings` 验收。
- 灰度：仅开启 `grok_hardening.workspace_checkpoint = true` 的主体启用；磁盘观测使用 `checkpoint_disk_bytes`，单 turn 文件数硬上限为 2048。
- 启动恢复补充：SQLite store 在单个 `IMMEDIATE` transaction 中校验所有记录并将遗留 `Open` 改为 `Aborted`；reopen 测试证明已终态记录不变、二次启动 reconciliation 为零。

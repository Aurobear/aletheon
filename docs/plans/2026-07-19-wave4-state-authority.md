# Wave 4：状态权威与持久化 Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让每一类 durable fact 只有唯一写入权威，其余表示全部退化为可从权威重放重建的 read projection；统一多库的 schema 版本 / 迁移 / 备份 / 恢复点 / reconciliation；用完整 Trajectory + token-based compaction 取代固定 6 条历史；实现 session branching/checkpoint 与 daemon 重启后对 resumable Runtime 的恢复；用 `kill -9` 恢复测试证明 turn / agent run / lease / approval / checkpoint 可从单一权威重建。

**Architecture:** 收敛而非新增。落地 roadmap §5.6 的权威表：
- **Turn/Item history 权威 = EventSpine**（`events.db`）。`CanonicalSessionStore`（`sessions-v1.db`）降级为可重放重建的 read projection；`SessionService` 的 `protocol-events-v1.db` 是 UI/protocol projection；删除 legacy `SessionManager` / `LegacySessionUseCases` 写路径。
- **Active operation 权威 = Kernel `OperationTable`** + 一条 durable recovery record（用 `submit_with_id` 复原，不再分配第二个 operation）。
- **Agent run lifecycle 权威 = `AgentRunRepository`**（`agent_control.db`）+ Kernel process binding；settlement/mailbox/memory 为其从属 store。
- 新增 `StorageManifest` 作为所有 SQLite 库的单一登记表（owner / authority / schema_version / 备份序 / 是否可从权威重建），启动时由 `MigrationCoordinator` 校验并执行迁移，`ReconciliationCoordinator` 在 admission 前把 projection 与权威对齐。

**Tech Stack:** Rust, SQLite.

**环境说明:** cargo 可用；构建/测试走 `bash scripts/cargo-agent.sh test -p <crate> <filter>`，不要用裸 cargo。

**依赖:** Wave 1（唯一 TurnEngine，统一 settlement）。Wave 4 假设 turn 写路径已收敛到单一 append 入口（`TurnCoordinator::append_tracked`，`crates/executive/src/service/turn_coordinator.rs:641`），否则权威收敛会在多条 turn-path 上各做一遍。

---

## 背景：已验证的当前状态（path:line）

- 固定历史窗口：`crates/executive/src/service/daemon_turn/helpers.rs:12` `MAX_HISTORY_MESSAGES = 6`；`:10-11` 字符上限 16KB/64KB；`:43` `bounded_text_history` 主动丢弃 tool_use/tool_result blocks（注释「replayed by the harness」），跨 turn 丢失工具证据。
- 双重 Session 表示：`crates/executive/src/impl/session/canonical_store.rs:14` `CanonicalSessionStore`（自称 “Canonical transactional store”）；`crates/executive/src/impl/session/event_sourced_store.rs:70` `EventSourcedSessionStore`（自述所有生产 mutation 先过 event spine 再物化 SQLite read model）。
- 生产 bootstrap 的矛盾：`crates/executive/src/impl/daemon/bootstrap/services.rs:227` 直接把 `CanonicalSessionStore` 作为 `TurnCoordinator` 的 `store`（写路径），同时 `:234` 又 `reconcile_committed_session_events(event_spine, projections, &canonical_store)` 把 canonical_store 当作从 spine 物化的 read_model —— 即当前 canonical store 既被直接写、又被 spine 重放覆盖，权威归属不确定。
- EventSpine authority：`crates/executive/src/impl/events/sqlite_event_spine.rs`（`events.db`），`committed_row_watermark` / `read_committed_page` 已支持幂等重放（`event_sourced_store.rs:37-66`）。
- Session schema 版本是单一常量：`crates/fabric/src/types/session.rs:11` `SESSION_SCHEMA_VERSION: u16 = 1`，无跨库统一版本登记。
- Operation 权威：`crates/kernel/src/operation/table.rs:18` `OperationTable`（注释 “Authoritative operation tree”），`:40 submit_with_id` 专为 restart recovery 复用持久 id。
- Agent run：`crates/executive/src/service/agent_control/repository.rs:10` `AgentRunRecord`（含 `version` / `retain_until_ms` / `resumability` / `recovery`）；`crates/executive/src/service/agent_control/recovery.rs:39` `AgentRecoveryCoordinator` 已做 open-run 启动 reconciliation（`MAX_STARTUP_RECOVERY_ROWS = 1000`）。
- Turn recovery：`crates/executive/src/service/turn_recovery.rs`（gated `grok_hardening.compaction_v2`）扫描无 terminal item 的 turn。
- Checkpoint：`crates/executive/src/impl/session/checkpoint_store_sqlite.rs:14` `SqliteCheckpointStore`（`workspace-checkpoints.sqlite`，WAL，`verify_integrity`）。
- Runtime 名称特判（Wave 2/3 债，但影响 storage 决策）：`crates/executive/src/service/agent_control/mod.rs:740` `request.runtime_id.0.contains("pi")` 决定 storage 配额。

**data_dir / agent_state_root 下已打开的库（`bootstrap/services.rs` + `request.rs` + `extensions.rs` + `channels.rs` 实测）：**
`events.db`, `event-projections.db`, `sessions-v1.db`, `protocol-events-v1.db`, `prompt-queue.sqlite`, `workspace-checkpoints.sqlite`, `agent_control.db`, `agent_settlement.db`, `agent_memory.db`, `self_field.db`, `recall_memory.db`, `fact_store.db`, `objectives.db`, `channels.db`, `episodic.db`, `memory_consolidation.db`, `memory_retention.db`, `conscious_workspace.db`, `extension-events.db`。

---

## Task 分组

- **A. 权威裁决与登记**（Task 1–4）：StorageManifest + schema/migration + reconciliation + 授权表落地。
- **B. Session/Event 权威收敛**（Task 5–7）：EventSpine 定为唯一权威，canonical 降级，删除 legacy 写路径。
- **C. Trajectory / 压缩 / 恢复**（Task 8–11）：完整 tool pair 持久化、token compaction、branching/checkpoint、Runtime resume。
- **D. kill-9 恢复测试**（Task 12–14）：turn / agent run / lease / approval / checkpoint 崩溃重启重建。

每个 Task 目标 2–5 分钟增量、可独立编译。带 🧪 的 Task 需要**完整 Rust env**（真起 daemon、发 `kill -9`），不能只 `cargo check`。

---

## A. 权威裁决与登记

### Task 1 — 新增 `StorageManifest` 类型与库登记

**Files:**
- 新建 `crates/executive/src/impl/storage/manifest.rs`
- 新建 `crates/executive/src/impl/storage/mod.rs`
- 改 `crates/executive/src/impl/mod.rs`（`pub mod storage;`）

类型 sketch（无占位、字段来自实测库清单）：

```rust
//! 单一 durable-store 登记表。每个 SQLite 库在此登记 owner / authority
//! / schema 版本 / 备份序 / 可否从权威重建，作为迁移与恢复的唯一事实来源。

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreRole {
    /// 唯一写入权威，崩溃后不可从他处重建，必须最先备份、最后恢复。
    Authority,
    /// 可从某个 Authority 幂等重放重建的读投影。
    Projection { rebuilt_from: AuthorityId },
    /// 独立领域权威（memory/self/objective 等），本波不改归属，仅登记。
    DomainAuthority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthorityId {
    EventSpine,       // events.db
    OperationTable,   // kernel in-memory + durable recovery record
    AgentRunRepo,     // agent_control.db
}

#[derive(Debug, Clone)]
pub struct StorageEntry {
    pub logical_name: &'static str,     // "sessions-v1"
    pub file: PathBuf,
    pub owner: &'static str,            // crate::module 路径
    pub role: StoreRole,
    pub schema_version: u32,
    /// 备份/恢复排序：Authority < Projection；同级按依赖拓扑。数字越小越先备份、越后恢复。
    pub backup_rank: u16,
}

#[derive(Debug, Clone, Default)]
pub struct StorageManifest {
    entries: Vec<StorageEntry>,
}

impl StorageManifest {
    pub fn register(&mut self, entry: StorageEntry) { self.entries.push(entry); }
    /// 备份顺序：Authority 先、Projection 后（backup_rank 升序）。
    pub fn backup_order(&self) -> Vec<&StorageEntry> { /* sort_by_key(backup_rank) */ }
    /// 恢复顺序 = 备份逆序：先恢复 Authority，再重建 Projection。
    pub fn recovery_order(&self) -> Vec<&StorageEntry> { /* backup_order().rev() */ }
    pub fn projections(&self) -> impl Iterator<Item = &StorageEntry> { /* .. */ }
    pub fn authorities(&self) -> impl Iterator<Item = &StorageEntry> { /* .. */ }
}
```

**Acceptance:** 单测 `manifest_backup_order_places_authority_before_projection` 与 `recovery_order_is_reverse_of_backup`；`bash scripts/cargo-agent.sh test -p executive manifest_`。不改运行语义。

---

### Task 2 — 集中登记所有生产库到 manifest（bootstrap 建 manifest）

**Files:**
- 改 `crates/executive/src/impl/daemon/bootstrap/services.rs`（在打开各库处收集 `StorageEntry`；返回结构体附带 `StorageManifest`）
- 改 `crates/executive/src/impl/daemon/bootstrap/request.rs`（memory/self/objective 库登记为 `DomainAuthority`）
- 改 `crates/executive/src/impl/daemon/bootstrap/mod.rs` 或对应 bundle struct，暴露 `storage_manifest`

**做什么：** 把上面「已打开的库」清单逐个 `manifest.register(...)`：`events.db`→`Authority(EventSpine, rank=0)`；`agent_control.db`→`Authority(AgentRunRepo, rank=1)`；`sessions-v1.db`→`Projection{EventSpine}`；`event-projections.db`/`protocol-events-v1.db`→`Projection{EventSpine}`；`agent_settlement.db`/`agent_memory.db`→从属 `AgentRunRepo`（`Projection`/`DomainAuthority` 视是否可重建，settlement 可从 spine settlement 事件重建→`Projection`，memory 独立→`DomainAuthority`）；`workspace-checkpoints.sqlite`→`DomainAuthority`（内容寻址、不可重放重建）；其余 memory/self/objective/channel 库→`DomainAuthority`。

**Acceptance:** 新增 `manifest_registers_every_opened_db`：断言 manifest 条目集合 == bootstrap 实际打开的库集合（用一个 `const EXPECTED_STORES: &[&str]` 对拍，防止以后加库忘记登记）。`bash scripts/cargo-agent.sh test -p executive manifest_registers`。

---

### Task 3 — `MigrationCoordinator`：统一 `PRAGMA user_version` + 启动 fail-fast

**Files:**
- 新建 `crates/executive/src/impl/storage/migration.rs`
- 改 `crates/executive/src/impl/storage/mod.rs`
- 各 store `open()` 内改用统一版本读写（`canonical_store.rs`、`sqlite_event_spine.rs`、`checkpoint_store_sqlite.rs`、`sqlite_repository.rs`）：把 `CREATE TABLE IF NOT EXISTS` 之外加 `PRAGMA user_version` 检查钩子

类型 sketch：

```rust
pub struct MigrationCoordinator<'a> { manifest: &'a StorageManifest }

#[derive(Debug)]
pub enum MigrationOutcome { UpToDate, Migrated { from: u32, to: u32 }, }

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("store {store} on-disk schema {found} newer than binary {expected}; refuse downgrade")]
    Downgrade { store: &'static str, found: u32, expected: u32 },
    #[error("store {store} migration {from}->{to} failed: {source}")]
    Failed { store: &'static str, from: u32, to: u32, source: anyhow::Error },
}

impl MigrationCoordinator<'_> {
    /// 对 manifest 每个库按 recovery_order（Authority 先）打开连接、读 user_version、
    /// 执行注册的迁移步骤。任一库失败必须阻止 daemon 进入 ready。
    pub fn run(&self) -> Result<Vec<(&'static str, MigrationOutcome)>, MigrationError>;
}
```

关键约束（对齐 arch-review §11.3）：**启动中任一 DB migration 失败必须阻止进入 ready** —— coordinator 返回 `Err` 时 bootstrap 直接 `context(...)?` 冒泡，不降级不 `unwrap_or_else` 到内存库。

**Acceptance:** 单测 `migration_refuses_downgrade`（on-disk user_version 高于 binary 时报 `Downgrade`）、`migration_upgrades_bumps_user_version`。`bash scripts/cargo-agent.sh test -p executive migration_`。

---

### Task 4 — `ReconciliationCoordinator`：admission 前把 projection 对齐权威

**Files:**
- 新建 `crates/executive/src/impl/storage/reconcile.rs`
- 改 `crates/executive/src/impl/daemon/bootstrap/services.rs`（把现有 `reconcile_committed_session_events` 调用纳入统一 coordinator，并加 recovery-point 记录）

**做什么：** 把当前分散的启动 reconciliation 统一为一个入口，按 manifest 依次：
1. EventSpine → session projection（复用 `event_sourced_store::reconcile_committed_session_events`，`services.rs:234`）。
2. EventSpine → event-projections（`DefaultEventProjectionSet`）。
3. AgentRunRepo open-run reconciliation（复用 `AgentRecoveryCoordinator`，`recovery.rs:39`）。
4. 记录 **recovery point**：把「已 reconcile 到的 committed row watermark + 时间戳」写入一张 `recovery_points` 表（放 `events.db` 或独立 `recovery-point.db`，登记为 `Projection`）。

```rust
#[derive(Debug, Clone, Default)]
pub struct ReconciliationReport {
    pub session_scanned: u64,
    pub session_materialized: u64,
    pub agent_open_rows: usize,
    pub agent_resumed: usize,
    pub recovery_point_row: i64,   // committed_row_watermark 快照
    pub ready: bool,               // 所有 projection 已追平权威
}
```

**Acceptance:** 单测 `reconcile_is_idempotent_across_double_run`（跑两遍 report.materialized 第二遍为 0）；`reconcile_report_not_ready_blocks_admission`。`bash scripts/cargo-agent.sh test -p executive reconcile_`。

---

## B. Session/Event 权威收敛

### Task 5 — 生产写路径改为「先 EventSpine 后物化 projection」

**Files:**
- 改 `crates/executive/src/impl/daemon/bootstrap/services.rs:227`：`TurnCoordinator` 的 `store` 参数从裸 `Arc::new(canonical_store)` 换成 `Arc::new(EventSourcedSessionStore::new(canonical_store, event_spine, projections))`，使所有 append 先落 spine 再物化 read model（消除「直接写 canonical + 又被 spine 覆盖」的双写）
- 改 `crates/executive/src/impl/session/event_sourced_store.rs`（确认 `EventSourcedSessionStore` 构造签名接受 read_model + spine + projection sink；补齐 `SessionAppendStore::append` 走 spine）
- 改 `crates/executive/src/service/turn_coordinator.rs:641 append_tracked`（不改逻辑，仅确认 `self.store.append` 现在指向 event-sourced store）

**Acceptance:** 现有 `crates/executive/tests/session_event_recovery.rs`、`session_append_store.rs`、`event_spine_repository.rs` 仍绿；新增断言：一次 append 后 `events.db` 的 committed 事件数 +1 且 `sessions-v1.db` 物化行随之出现。`bash scripts/cargo-agent.sh test -p executive session_event_recovery session_append_store`。

---

### Task 6 — `CanonicalSessionStore` 文档/命名降级为 read projection

**Files:**
- 改 `crates/executive/src/impl/session/canonical_store.rs:1,14`：doc 从 “Canonical transactional store” 改为 “Read projection materialized from EventSpine; rebuildable, never authoritative”；类型注释标注 authority=EventSpine
- 改 `crates/executive/src/impl/session/canonical_store.rs:19 default_session_db_path`：注释说明该库可删除后从 spine 重建
- 改 `crates/fabric/src/types/session.rs:11`：`SESSION_SCHEMA_VERSION` 注释说明它只描述 projection 行格式，权威 schema 在 spine 事件

> 不改类型名（避免大范围重命名风险），仅收敛语义与文档，配合 manifest role=Projection。若后续要重命名，另开 PR。

**Acceptance:** `bash scripts/cargo-agent.sh test -p executive` 编译通过；`architecture-check.sh` 若有 authority 计数门禁则 Turn/Item authority=1。

---

### Task 7 — 冻结并删除 legacy Session 写路径

**Files:**
- 删/冻结 `crates/executive/src/service/legacy_session_service.rs`（`LegacySessionUseCases` 全 trait —— 确认无生产调用者后删除；若 CLI 仍引用则先 `#[deprecated]` + feature-gate 并在 manifest/architecture-status 标删除期限）
- 改 `crates/executive/src/impl/daemon/session_manager.rs`：`SessionManager` 若仅服务 legacy in-memory `SessionStore`（`crate::session::store::SessionStore`），把生产读改到 event-sourced projection；否则标 experimental
- 改 `crates/executive/src/core/session_gateway/gateway.rs`、`crates/executive/src/impl/daemon/debug_handler.rs`：去除对 legacy store 的写引用，读引用改走 projection
- 改 `crates/executive/src/impl/daemon/bootstrap/request.rs`、`services.rs`、`turn_runtime.rs`：移除 legacy store 的构造与注入

**先查后删：** `bash scripts/cargo-agent.sh check -p executive` 后 `rg -n "LegacySessionUseCases|session_manager::SessionManager|session::store::SessionStore" crates` 确认零生产调用者再删。

**Acceptance:** 删除后 `bash scripts/cargo-agent.sh test -p executive` 全绿；无第二个 store 宣称 Session 权威（grep 断言 `CanonicalSessionStore` 不再被直接 `.append` 于 turn 写路径之外）。

---

## C. Trajectory / 压缩 / 恢复

### Task 8 — 持久化完整 tool call/result pair（Trajectory 记录）

**Files:**
- 改 `crates/fabric/src/types/session.rs`：确认 `ItemPayload` 覆盖 `ToolUse`/`ToolResult`（若已覆盖，仅确认 spine 事件不丢 block）
- 改 `crates/executive/src/impl/session/event_sourced_store.rs`：append 时保留 `ContentBlock::ToolUse`/`ToolResult` 完整入 spine（不做文本化丢弃）
- 改 `crates/executive/src/service/daemon_turn/helpers.rs:27 bounded_text_history`：**不再作为跨-turn 历史来源**；把它降为「仅对超长注入 payload 做单条上限」的工具函数，历史改由 Task 9 的 compaction 从完整 trajectory 生成
- 新建 `crates/executive/src/service/trajectory.rs`：`TrajectoryReader`，从 event-sourced projection 读回完整含 tool blocks 的 item 序列

```rust
/// 从权威 (EventSpine → projection) 读回一个 session 的完整 trajectory，
/// 保留每个 tool_use 与其对应 tool_result（不做 6 条截断、不丢 tool blocks）。
pub struct TrajectoryReader { store: Arc<dyn SessionAppendStore> }

pub struct TrajectoryItem {
    pub turn_id: TurnId,
    pub sequence: u64,
    pub payload: ItemPayload, // 完整，含 ToolUse/ToolResult
}

impl TrajectoryReader {
    pub async fn full(&self, session: &SessionId) -> Result<Vec<TrajectoryItem>>;
    pub async fn since_turn(&self, session: &SessionId, turn: TurnId) -> Result<Vec<TrajectoryItem>>;
}
```

**Acceptance:** 单测 `trajectory_preserves_tool_use_result_pairs`：写入 user→tool_use→tool_result→assistant，读回 4 项且 tool blocks 完整。`bash scripts/cargo-agent.sh test -p executive trajectory_`。

---

### Task 9 — token-based compaction 取代固定 6 条历史

**Files:**
- 新建 `crates/executive/src/service/compaction.rs`
- 改 `crates/executive/src/service/daemon_turn/execute.rs`（组装 model 上下文处：从 `TrajectoryReader::full` 取完整历史 → 过 `TokenBudgetCompactor` → 得到 model messages，替换现在经 `bounded_text_history` 的 6 条路径）
- 改 `crates/executive/src/service/daemon_turn/helpers.rs:12`：删除 `MAX_HISTORY_MESSAGES = 6`（或标 `#[deprecated]` 仅测试用），保留字符上限常量供 compactor 复用

```rust
pub struct TokenBudgetCompactor {
    /// 上下文 token 预算（来自 ResolvedTurnProfile.budget，Wave 0 已接线）。
    pub max_context_tokens: u32,
    /// 估算器：字符/4 近似或接 tokenizer。
    pub estimator: Arc<dyn TokenEstimator>,
}

pub struct CompactionResult {
    pub messages: Vec<Message>,     // 送 model 的窗口
    pub summarized_turns: u32,      // 被摘要压缩的 turn 数
    pub retained_tool_pairs: u32,   // 保留的完整 tool pair 数
    pub dropped_tokens: u64,
}

impl TokenBudgetCompactor {
    /// 保留最近的完整 tool pair，超预算的旧 turn 生成结构化摘要（累积
    /// 文件操作/决定/约束/进度），而不是先破坏性截断成 6 条纯文本。
    pub fn compact(&self, trajectory: &[TrajectoryItem]) -> CompactionResult;
}
```

**Acceptance:** 单测 `compaction_keeps_recent_tool_pairs_within_budget`、`compaction_summarizes_old_turns_not_truncate_to_6`（构造 >6 turn 且总量超预算，断言最近 tool pair 完整保留、旧 turn 进摘要而非被丢）。`bash scripts/cargo-agent.sh test -p executive compaction_`。

---

### Task 10 — session branching / checkpoint 落到权威事件

**Files:**
- 改 `crates/executive/src/service/session_service.rs`（`SessionService` 已有 `SessionFork` 支持 —— 确认 fork 走 spine 的 `SessionForkedEvent`，`event_sourced_store.rs:14` 已 import）
- 改 `crates/executive/src/impl/session/event_sourced_store.rs`：fork 产出 `SessionForkedEvent` 落 spine（权威），projection 物化分支起点
- 改 `crates/executive/src/service/workspace_checkpoint.rs`：checkpoint 创建/finalize 发 spine 事件（`with_events` 已接 `canonical_event_spine`，`services.rs:305`）—— 确认 checkpoint↔turn boundary 关联通过 spine 事件而非仅本地 sqlite

**Acceptance:** 单测 `session_fork_emits_spine_event_and_projection`；`checkpoint_finalize_recorded_in_spine`。`bash scripts/cargo-agent.sh test -p executive session_fork checkpoint_finalize`。

---

### Task 11 — daemon 重启后恢复 resumable Runtime

**Files:**
- 改 `crates/executive/src/service/agent_control/recovery.rs:39`（`AgentRecoveryCoordinator`）：对 `AgentRunRecord.resumability == RuntimeResumability::Session` 且 `recovery.checkpoint_available` 的 run，重启时用 `OperationTable::submit_with_id`（`crates/kernel/src/operation/table.rs:40`）复原 operation，并向对应 Runtime 发 resume（不分配新 operation/agent id）
- 改 `crates/executive/src/service/agent_control/mod.rs`：把 `runtime_id.0.contains("pi")`（`:740`）的 storage 特判改为读 `AgentRunRecord.resumability` / manifest 声明的 persistence（消除字符串特判，配合 Wave 3 Runtime Manifest；本波先用 record 字段）
- 改 `crates/executive/src/impl/daemon/bootstrap/services.rs`：recovery 纳入 Task 4 的 `ReconciliationCoordinator`，在 admission 前完成

**Acceptance:** 单测 `resumable_run_reuses_operation_id_on_restart`（同一 `OperationId` 被 `submit_with_id` 复用，不新增）；`non_resumable_run_finalized_on_restart`。`bash scripts/cargo-agent.sh test -p executive resumable_run non_resumable_run`。

---

## D. kill-9 恢复测试（🧪 需完整 Rust env：真起 daemon + `kill -9`）

> 这些测试不能用内存库或 `cargo check` 替代。步骤：`bash scripts/cargo-agent.sh test -p executive --test <name> -- --ignored`（标 `#[ignore]`，在有真实文件系统与进程的 CI job 跑）。每个测试的骨架：(1) 用真实 `data_dir` 起 daemon 子进程；(2) 驱动到目标中间态；(3) 对 daemon PID 发 `libc::kill(pid, SIGKILL)`；(4) 用同一 `data_dir` 重启；(5) 断言从权威重建、无 projection 与权威互相宣称权威、无残留 lease。

### Task 12 🧪 — turn / session kill-9 恢复

**Files:**
- 新建 `crates/executive/tests/kill9_turn_recovery.rs`
- 复用 `crates/executive/tests/support/`（daemon 启动 harness）、`session_event_recovery.rs` 断言工具

**测试点：** turn 写到一半（有 UserMessage 无 terminal item）→ `kill -9` → 重启 → `turn_recovery::scan_incomplete_turns`（`turn_recovery.rs`）将其分类 Interrupted；`sessions-v1.db` 删除后仍能从 `events.db` 重放重建（断言 `reconcile_committed_session_events` 物化行数一致）。

**Acceptance:** 重启后 session projection 与 spine 一致（hash/count 对拍，对齐 arch-review §11.3「projection 可从零重建并校验 hash/count」）；被 kill 的未完成 turn 有 terminal 分类。

---

### Task 13 🧪 — agent run / lease / approval kill-9 恢复

**Files:**
- 新建 `crates/executive/tests/kill9_agent_lease_approval.rs`
- 复用 `crates/executive/tests/agent_recovery.rs`、`agent_control_repository.rs`、`agent_mailbox.rs` 的断言

**测试点：**
- **agent run:** open run 中途 `kill -9` → 重启 `AgentRecoveryCoordinator`（`recovery.rs:39`）→ resumable 复用 operation id 重连、non-resumable 被 finalize；`open_rows` 全部 reconcile，`report.ready()==true`。
- **lease:** 持有 `AgentResourceLeaseKind::{Admission,Mailbox,Execution}`（`repository.rs:79`）时 kill → 重启后无泄漏 lease（Kernel `admission/lease.rs` 计数归零或被 recovery 释放）。
- **approval:** 待批 approval 中途 kill → 重启后 approval 状态从权威（spine approval 事件）重建，不丢不重复。

**Acceptance:** `report.recovery_failed==0 && report.unreconciled==0`；kill 后残留进程数=0（对齐 roadmap §8.2 「cancel 后残留进程数」指标）；无 lease 泄漏。

---

### Task 14 🧪 — checkpoint kill-9 恢复与全库 recovery-order 演练

**Files:**
- 新建 `crates/executive/tests/kill9_checkpoint_and_manifest.rs`

**测试点：**
- checkpoint Open 状态中途 `kill -9` → 重启 `SqliteCheckpointStore`（`checkpoint_store_sqlite.rs`）reconcile：Open→Aborted（`workspace_checkpoint.rs:58` 已有 startup reconcile 计数），`verify_integrity` 通过。
- 按 `StorageManifest::recovery_order()`（Task 1）执行恢复：先 Authority（`events.db`/`agent_control.db`）后 Projection；断言删除全部 Projection 库后可完整重建、且无「两个 store 互相宣称权威」（manifest 中 Authority 唯一）。

**Acceptance:** 满足 roadmap §5 Wave 4 验收「kill -9 后可从单一 durable authority 重建 projection；没有两个 store 互相宣称权威」；`ReconciliationReport.ready==true`。

---

## 验收汇总（对齐 roadmap §5 Wave 4 / arch-review §9 A3 / audit PR6）

- [ ] 每类 durable fact 的 authority 数 = 1（Turn/Item→EventSpine；operation→OperationTable；agent run→AgentRunRepo），manifest + architecture-status 可机器校验。
- [ ] `StorageManifest` 登记全部生产库，`MigrationCoordinator` 任一迁移失败阻止 ready，`ReconciliationCoordinator` 记录 recovery point、admission 前追平 projection。
- [ ] 完整 tool call/result pair 持久化；固定 6 条历史被 token-based compaction 取代；session branching/checkpoint 落 spine 事件；resumable Runtime 重启复用 operation id。
- [ ] kill-9 恢复测试覆盖 turn / agent run / lease / approval / checkpoint，全部从单一 durable authority 重建，无残留 lease/进程、无双权威。

## 明确不做（本波边界）

- 不合并/重命名 memory/self/objective 等 `DomainAuthority` 库的归属（仅登记 manifest）。
- 不重写 Runtime Manifest（Wave 2/3）；Task 11 只用 `AgentRunRecord.resumability` 字段消除 `contains("pi")` storage 特判。
- 不引入分布式事务；跨库一致性靠「Authority 先备份/后恢复 + Projection 幂等重放」，不引入两阶段提交。
- 不改 CLI/child turn 主链（假定 Wave 1 已收敛到单一 append 入口）。

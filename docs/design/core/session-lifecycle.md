# 会话管理与生命周期

> 会话管理与生命周期是 OS-Agent 作为系统级服务的核心基础设施。涵盖会话持久化、崩溃恢复、Hook 系统、可观测性。会话持久化与崩溃恢复。

**模块编号:** 10
**关联模块:** [cognitive-engine](cognitive-engine.md), [tool-system](../execution/tool-system.md), [security-model](../security/security-model.md)
**最后更新:** 2026-06-06

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| SessionStore | ✅ Implemented | `session/store.rs` | Session CRUD + metadata |
| EventJournal | ✅ Implemented | `session/journal.rs` | JSONL append-only log + SQLite index |
| Session recovery | 🔶 Partial | `session/journal.rs` | `recover()` exists but unused in practice |
| InterruptManager | ⬜ Planned | — | Designed but not started |
| CompressionManager | ⬜ Planned | — | Designed but not started |
| SessionHierarchy | ⬜ Planned | — | Multi-agent session tree not started |

**Stale reference fixed:** Doc references `ThreadStore` trait; code has `SessionStore` with different API.

---

## 目录

- [1. 概述](#1-概述)
- [2. 当前设计](#2-当前设计)
- [3. 已识别缺陷](#3-已识别缺陷)
  - [P0: Session 持久化与崩溃恢复](#p0-session-持久化与崩溃恢复)
  - [P1: Hook 系统](#p1-hook-系统)
  - [P2: 可观测性/调试接口](#p2-可观测性调试接口)
  - [P0: 崩溃恢复边界条件未定义](#p0-崩溃恢复边界条件未定义)
- [4. 改进设计](#4-改进设计)
  - [4.1 Session 持久化 — Event Journal + SQLite Index](#41-session-持久化--event-journal--sqlite-index)
  - [4.2 Hook 系统](#42-hook-系统) → [hook-system.md](hook-system.md)
  - [4.3 可观测性](#43-可观测性) → [observability-stack.md](../observability/observability-stack.md)
  - [4.4 崩溃恢复协议 — 三种场景的标准处理](#44-崩溃恢复协议--三种场景的标准处理)
- [5. 实现要点](#5-实现要点)
- [6. 参考来源](#6-参考来源)

---

## 1. 概述

会话管理与生命周期是 OS-Agent 作为系统级服务的核心基础设施。涵盖会话持久化、崩溃恢复、Hook 系统、可观测性。

作为永远在线的 daemon，OS-Agent 必须解决三个关键问题：
1. **持久性** — daemon 崩溃或升级后，会话状态不能丢失
2. **可扩展性** — 用户需要在推理循环的关键节点注入自定义逻辑
3. **可诊断性** — 长时间运行的 daemon 需要实时可观测的内部状态

原始设计文档完全没有覆盖这三个方面。Agent 循环纯粹在内存中运行，没有持久化、没有 Hook、没有结构化的调试接口。

---

## 2. 当前设计

原始设计文档对会话管理的覆盖为零。具体缺失：

| 能力 | 原始设计状态 | 说明 |
|------|-------------|------|
| 会话持久化 | 不存在 | Agent 循环在内存中，进程退出即丢失 |
| 崩溃恢复 | 不存在 | 无 checkpoint 机制，无法从断点恢复 |
| Hook 系统 | 不存在 | 工具执行前后无扩展点 |
| 可观测性 | 不存在 | 无 reasoning 日志、无 debug 接口、无 metrics |
| 会话隔离 | 不存在 | 无多会话支持 |

唯一相关的设计是 LangGraph 的检查点模式（§2.2 提及但未展开到实现）和 FUSE 的 `/mnt/agent/logs/reasoning`（§11，但只是只读文件视图，不是结构化的推理日志）。

---

## 3. 已识别缺陷

### P0: Session 持久化与崩溃恢复

**问题:** daemon 崩溃 = 一切丢失。作为系统级服务，daemon 可能因为 OOM、内核 panic、升级重启等原因中断。当前设计中，所有会话状态（对话历史、当前任务进度、工具调用中间结果）都在内存中，任何中断都意味着完全丢失。

**影响:** 用户正在进行的多步任务（如"帮我编译并部署这个项目"）在 daemon 重启后无法继续，必须从头开始。对于长时间运行的任务（编译、测试），这是不可接受的。

**参考:**
- **Codex SQ/EQ + Rollout:** 每个 Submission 和 Event 都持久化，支持 Resumed 和 Forked 重启模式
- **Hermes SQLite WAL:** 完整会话状态存储在 SQLite 中，支持 FTS5 全文检索历史对话
- **LangGraph 检查点:** 通道状态版本化持久化，支持从任意检查点恢复

### P1: Hook 系统

**问题:** 用户无法在工具执行前后注入自定义逻辑。典型需求：参数合规检查、副作用记录、LLM 调用前注入上下文、LLM 调用后过滤响应、感知事件触发自定义处理。

**参考:**
- **Codex hooks:** 三种类型（Command / Prompt / Agent），支持 Sync/Async 执行，作用域为 Thread 或 Turn
- **Hermes plugin hooks:** 插件可在多个生命周期点注入逻辑

### P2: 可观测性/调试接口

**问题:** 永远在线的 daemon 缺乏诊断手段。当 Agent 做出意外决策时，用户无法查看完整推理链路、实时内部状态或性能指标。

### P0: 崩溃恢复边界条件未定义

**问题:** §4.1 的会话持久化设计采用 JSONL 事件日志 + SQLite 索引 + `CheckpointBoundary` 机制，在正常运行下能保证一致性恢复点。但设计未明确定义三种崩溃场景下的边界条件和恢复协议：

**场景 A:** daemon 崩溃，工具调用未开始 — `CheckpointBoundary` 已 fsync，恢复后可重新发起工具调用。

**场景 B:** daemon 崩溃，工具调用已发出（in-flight） — 状态语义不明，工具侧子进程去向未知。

**场景 C:** 工具副作用已产生，daemon 崩溃 — 工具已修改系统状态但事件日志中没有记录。

---

## 4. 改进设计

### 4.1 Session 持久化 — Event Journal + SQLite Index

核心架构变更：用**追加式事件日志**替代全量 blob checkpoint，SQLite 仅作为索引和元数据存储。

#### 4.1.1 事件类型系统 (SessionEvent)

`SessionEvent` 是追加式日志的基本单元。每个事件包含 `seq`（单调递增序号）、`correlation_id`（关联 ID）、`timestamp` 和 `body`（事件体）。

事件体 `SessionEventBody` 为标记联合，涵盖以下分类：
- **生命周期:** SessionStarted, SessionEnded
- **用户交互:** UserMessage, AssistantMessage
- **工具执行:** ToolCallStarted, ToolCallCompleted, ToolCallFailed
- **状态变更:** LoopStateChanged, CoreMemoryChanged, PermissionChanged
- **上下文管理:** Compacted (压缩前后摘要)
- **检查点边界:** CheckpointBoundary (一致恢复点，在每个 tool call 前写入)
- **审批流程:** ApprovalRequested, ApprovalResolved
- **多 Agent:** SubAgentSpawned, SubAgentCompleted

`SessionSource` 枚举标识会话来源：Cli, Daemon, SubAgent, Review, MemoryConsolidation。
`EndReason` 枚举标识结束原因：UserExit, TaskCompleted, Error, Interrupted, Compression, DaemonShutdown。

#### 4.1.2 事件日志存储 (EventJournal)

`EventJournal` 实现追加式事件日志，参考 Codex RolloutRecorder。架构要点：

- **专用 writer task** — 通过 `mpsc::channel` 接收写入命令，单线程顺序写入，避免锁竞争
- **JSONL + SQLite 混合** — 事件体存储在 JSONL 文件（追加式，高效写入），SQLite 仅存储索引元数据（支持快速查询）
- **JournalCmd 协议** — Append（批量追加）、Persist（fsync 刷盘）、Flush（缓冲区刷新）、Shutdown（关闭日志）
- **create()** — 创建新会话日志，返回 (journal, writer_task_handle)
- **resume()** — 恢复已有会话：重放 JSONL 文件，从最后一个 CheckpointBoundary 找到一致状态，返回需要重放的事件
- **append()** — 非阻塞，通过 channel 发送到 writer task
- **checkpoint()** — 写入 CheckpointBoundary 并 fsync，确保恢复点持久化
- **WAL checkpoint 策略** — 每 50 次写入执行 `PRAGMA wal_checkpoint(TRUNCATE)`，防止 WAL 文件无限增长

#### 4.1.3 线程存储抽象 (ThreadStore trait)

`ThreadStore` trait 定义会话存储的核心抽象接口：

```rust
#[async_trait]
trait ThreadStore: Send + Sync {
    async fn create_session(&self, params: CreateSessionParams) -> Result<SessionHandle>;
    async fn resume_session(&self, session_id: &str) -> Result<ResumeResult>;
    async fn fork_session(&self, parent_id: &str, params: ForkParams) -> Result<SessionHandle>;
    async fn read_session_meta(&self, session_id: &str) -> Result<SessionMeta>;
    async fn list_resumable(&self, limit: usize) -> Result<Vec<SessionSummary>>;
    async fn delete_session(&self, session_id: &str) -> Result<()>;
}
```

关键类型：
- `CreateSessionParams` — source, model, cwd, parent_session_id, initial_memory, personality
- `ForkParams` — from_seq (分叉点事件序号), source
- `ResumeResult` — handle, replay_events, last_checkpoint
- `SessionHandle` — session_id, journal, writer_task
- `CheckpointState` — loop_state, message_count, token_usage

`LocalThreadStore` 是基于本地文件系统的实现，会话元数据存储在 SQLite 中。

#### 4.1.4 初始历史状态机 (InitialHistory)

`InitialHistory` 枚举定义四种会话启动模式，参考 Codex 的 InitialHistory：
- **New** — 全新会话
- **Cleared** — 清除历史的会话（新 session_id，无先前历史）
- **Resumed** — 恢复已有会话，从最后 CheckpointBoundary 重放事件
- **Forked** — 分叉自父会话，复制事件历史

`SessionInitGuard` 参考 Codex `LiveThreadInitGuard`，确保初始化原子性：成功则 `commit()`，失败则丢弃未提交状态。

#### 4.1.5 SQLite Schema

SQLite 存储四类数据：
- **sessions** — 会话元数据（session_id, source, model, cwd, parent/fork 关系, started/ended 时间, token 统计）
- **event_index** — 事件索引（session_id, seq, correlation_id, event_type, timestamp），不存储完整事件体
- **compression_locks** — 压缩锁（防止并发压缩，TTL-based）
- **memory_blocks** — Core Memory 块（独立存储，不放在事件日志中）

#### 4.1.6 写入竞争处理

`WriteExecutor` 参考 Hermes 的 jitter retry 模式解决多进程并发写入 SQLite 的竞争问题：
- 使用 `BEGIN IMMEDIATE` 在事务开始时获取写锁
- `SQLITE_BUSY` 时随机退避（20-150ms，最多 15 次重试），避免 convoy effect

#### 4.1.7 WAL 回退检测

启动时检测 WAL 是否生效。NFS/SMB/FUSE 等网络文件系统不支持 WAL，自动回退到 DELETE 模式 + `synchronous=FULL`。

#### 4.1.8 会话压缩 / 上下文分裂

`CompressionManager` 在上下文窗口接近限制时（默认 80%）压缩并创建延续会话：
- 获取压缩锁（TTL-based，防止死锁）
- 生成历史摘要
- 结束当前会话（EndReason::Compression）
- 创建延续会话，注入摘要作为第一条消息
- `get_compression_tip()` 沿压缩链找到活跃的延续会话

#### 4.1.9 中断/恢复协议

`SessionInterrupt` 参考 LangGraph `GraphInterrupt`，用于人机交互断点（审批、确认、输入）：
- `InterruptReason` — ApprovalRequired, UserInputRequired, HumanBreakpoint
- `PendingWrite` — 分离写入和应用，支持原子提交
- `InterruptManager` — raise() 触发中断并持久化，resume() 恢复并应用 pending_writes

#### 4.1.10 多 Agent 会话层级

`SessionHierarchy` 支持父子会话层级，用于 sub-agent、review thread、memory consolidation：
- create_child() — 创建子会话
- get_tree() — 递归获取会话层级树
- get_ancestors() — 获取祖先链（用于上下文传递）

#### 4.1.11 声明式 Schema 管理

`SchemaManager` 参考 Hermes 的声明式 schema 协调：
- 单一 `SCHEMA_SQL` 定义目标 schema
- `_reconcile_columns()` 启动时自动检测并添加缺失列
- 无版本迁移链：列添加不需要版本号，数据转换仍需版本门控迁移

### 4.2 Hook 系统

> **已提取为独立文档** → [hook-system.md](hook-system.md)

Hook 系统提供 21 种事件类型、Matcher 分发、并发执行、信任模型等完整的 Hook 框架。详见独立文档。


### 4.3 可观测性

> **已提取为独立文档** → [observability-stack.md](../observability/observability-stack.md)

可观测性栈包括 Durable/Ephemeral 事件分类、Fragment Accumulator、Debug CLI (JSON-RPC)、Prometheus 指标导出等。详见独立文档。


### 4.4 崩溃恢复协议 — 三种场景的标准处理

针对 §3 中的"崩溃恢复边界条件未定义"缺陷，定义三种崩溃场景的标准恢复协议。

#### 4.4.1 扩展工具调用状态标记

在 `SessionEventBody` 中增加 `ToolCallInFlight` 状态（call_id, tool_name, args, child_pid, started_at），使恢复逻辑能区分场景 A（未启动）和场景 B（状态未知）。

写入时序变为：
```
CheckpointBoundary → ToolCallStarted → ToolCallInFlight → 执行工具 → ToolCallCompleted/Failed
```

#### 4.4.2 场景 A 恢复协议（checkpoint 后、工具未启动）

重放到最后一个 CheckpointBoundary → 检查是否有 ToolCallStarted 但无 ToolCallInFlight → 自动重试工具调用（默认）或询问用户（可配置）。

#### 4.4.3 场景 B 恢复协议（工具 in-flight）

重放到最后一个 CheckpointBoundary → 检查是否有 ToolCallInFlight 但无 Completed/Failed → 查询子进程 PID 是否仍存活 → 向用户呈现恢复选项（重试/跳过/终止）。

#### 4.4.4 场景 C 恢复协议（副作用已产生）

重放到最后一个 CheckpointBoundary → 查询审计日志确定工具是否已执行 → 补写 ToolCallCompleted 事件 → 如果副作用可回滚则询问用户。

#### 4.4.5 审计日志作为二级恢复源

安全模型的审计日志在工具执行后立即写入，比会话事件日志更接近实时。恢复协议在会话日志不完整时，查询审计日志确定工具调用的实际结果。需要在审计日志中增加 `call_id` 字段以便关联。

#### 4.4.6 会话日志与回滚引擎的协调协议

定义 `RollbackAnchor` 事件类型（call_id, snapshot_type, snapshot_id, snapshot_path），在关键工具调用前写入。恢复时从 `RollbackAnchor` 获取 `snapshot_id`，调用回滚引擎恢复。

#### 4.4.7 守护进程孤儿进程管理

daemon 启动时扫描所有 in-flight 的工具调用，根据配置策略处理孤儿进程：
- **Wait** — 等待子进程完成（超时 30s）
- **Terminate** — 终止子进程
- **Detach** — 标记为分离，不再管理

配置项（`/etc/agent/agent.toml`）：

```toml
[lifecycle.crash_recovery]
orphan_policy = "terminate"
orphan_wait_timeout = 30
auto_retry_inflight = false
use_audit_as_fallback = true
```

---

## 5. 实现要点

### 5.1 事件日志存储

- **JSONL + SQLite 混合架构** — 事件体存储在 JSONL 文件（追加式，高效写入），SQLite 仅存储索引和元数据（支持快速查询）
- **CheckpointBoundary 粒度** — 在每个 tool call 前写入 `CheckpointBoundary` 事件并 fsync，确保最多丢失一个工具调用
- **Writer task 单线程模型** — 专用 tokio task 处理所有写入，通过 `mpsc::channel` 接收命令，避免锁竞争
- **WAL checkpoint 策略** — 每 50 次写入执行 `PRAGMA wal_checkpoint(TRUNCATE)`，防止 WAL 文件无限增长

### 5.2 写入竞争处理

- **Jitter retry 模式** — 参考 Hermes，使用 `BEGIN IMMEDIATE` + 随机退避（20-150ms，最多 15 次重试），避免 convoy effect
- **WAL 回退检测** — 启动时检测 WAL 是否生效，NFS/SMB/FUSE 自动回退到 DELETE 模式 + `synchronous=FULL`
- **压缩锁** — 使用 SQLite 表实现分布式锁（TTL-based），防止并发压缩

### 5.3 会话生命周期

- **SessionInitGuard** — 参考 Codex `LiveThreadInitGuard`，初始化失败自动丢弃未提交状态
- **InitialHistory 状态机** — New/Cleared/Resumed/Forked 四种启动模式
- **事件重放恢复** — 从最后 `CheckpointBoundary` 向前重放事件，重建内存状态

### 5.4 上下文管理

- **会话压缩链** — 当 token 使用超过 80% 时，压缩当前会话并创建延续会话
- **get_compression_tip()** — 沿压缩链找到活跃的延续会话
- **中断/恢复协议** — `SessionInterrupt` + `PendingWrite` 模式，支持审批、用户输入、人类断点

### 5.5 Schema 管理

- **声明式 schema 协调** — 单一 `SCHEMA_SQL` 定义目标 schema，启动时自动添加缺失列
- **无版本迁移链** — 列添加不需要版本号；数据转换仍需版本门控迁移

### 5.6 可观测性

- **Durable/Ephemeral 事件分离** — 流式增量事件标记为 Ephemeral，仅实时推送不持久化；所有其他事件标记为 Durable，写入 JSONL 日志可重放
- **Fragment Accumulator** — 流式增量通过 FragmentAccumulator 累积为完整的持久化值
- **工具调用生命周期状态机** — ToolTracker 跟踪每个 call_id 的 input_started/input_ended/called/settled 状态
- **Token Usage 安全归一化** — safe_tokens() 防御 NaN/负值，TokenUsageBreakdown 分项统计
- **事件 Schema 版本控制** — JournalHeader 携带 format_version 和 schema_version，支持前向兼容
- **推理日志轮转** — JSONL 文件按大小轮转（默认 100MB），保留 7 天
- **Debug CLI 通过 Unix socket JSON-RPC 通信** — 8 个 RPC 方法，支持流式输出
- **Metrics 端点** — 使用 `prometheus-client` crate 暴露 `/metrics`，默认监听 `127.0.0.1:9090`

### 5.7 Hook 系统

- **14 种事件类型** — 覆盖工具执行、权限请求、上下文压缩、会话生命周期、子 agent、LLM 调用、感知、安全
- **每事件类型化 Request/Outcome** — 每个事件有强类型的输入和输出
- **三种 Hook 类型** — Command（shell 命令）、Prompt（LLM 模板）、Agent（WASM/Rust 子进程）
- **Matcher 分发** — 三种模式：None、字面量管道分隔、正则表达式
- **并发执行 + 顺序保留** — FuturesUnordered 并发执行，保留声明顺序用于报告
- **信任模型** — 三级信任：Managed、Trusted、Untrusted
- **分层配置发现** — 5 层配置源：System → User → Project → Plugin → SessionFlags

---

## 6. 参考来源

- **Codex (Session):** rollout 持久化（SQ/EQ 协议）、`RolloutRecorder` 后台 writer task、`InitialHistory` 状态机、`LiveThreadInitGuard` 原子初始化、`ThreadStore` trait 抽象
- **Codex (Hooks):** `HookEventName` 10 variants、matcher 分发（三种模式）、并发执行 `FuturesUnordered`、hook trust system、分层配置发现、`HookOutputSpiller`、Preview/Run dual API
- **OpenCode (Plugins):** Immer draft mutation、scoped 生命周期、keyed mutex
- **OpenCode (Observability):** Durable/Ephemeral 事件分离、Fragment Accumulator、per-callID 工具状态机、Token Usage 安全归一化、事件 schema 版本控制
- **Hermes:** SQLite WAL 会话存储、`_execute_write()` jitter retry 模式、`apply_wal_with_fallback()` NFS 回退、`_reconcile_columns()` 声明式 schema 协调、compression chain、FTS5 + trigram tokenizer
- **LangGraph:** 检查点机制、`checkpoint_pending_writes` 分离写入与应用、`GraphInterrupt` 中断/恢复协议、channel-based 状态版本化
- **原始设计文档:** `docs/plans/2026-06-06-argos-design.md` (historical reference, file has been removed)

---

## Implementation Summary

**Code locations:**
- `session/store.rs` — SessionStore (session CRUD + metadata)
- `session/journal.rs` — EventJournal (JSONL append-only log + SQLite index), session recovery

**Key types/traits implemented:**
- `SessionStore` — session CRUD operations and metadata management
- `EventJournal` — append-only JSONL event log with SQLite indexing
- `SessionEvent` / `SessionEventBody` — event type system with lifecycle, tool, state, approval, and multi-agent variants
- `ThreadStore` trait — session storage abstraction (create/resume/fork/list/delete)
- `CheckpointBoundary` — consistent recovery point marker

**Planned (not started):**
- InterruptManager — human-in-the-loop interrupt/resume protocol
- CompressionManager — context window compression and session chaining
- SessionHierarchy — multi-agent session tree management

**Test coverage:** Recovery flow (`recover()`) exists in journal.rs but is unused in practice. No dedicated test suite documented for session lifecycle.

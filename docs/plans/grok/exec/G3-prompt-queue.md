# G3 可执行 Spec：Prompt Queue 与 Mid-turn Interjection

> 对应研究文档 `../04-prompt-queue-and-interjection.md`。优先级 P1（依赖 G1/G2 稳定后启动）。
> 实施前按 `README.md §5` 重新核对 §2 锚点。

## 1. 目标与非目标

**目标**：让 session（`(principal, thread)`）持有一个**版本化 pending prompt 队列**（enqueue/edit/cancel，stale edit 冲突检测，多客户端顺序确定），以及一个 **mid-turn interjection buffer**（在 turn 循环安全点以 FIFO、独立 synthetic user message 注入运行中的 turn）。

**非目标**：
- 不实现任意 reorder / 跨用户协同编辑（第一版）。
- 不把队列放进 Cognit harness——由 Executive 的 session 协调层持有。
- 不把"停止"编码为文本插话（用已有 `Interrupted`/cancellation）。
- 不改 `TurnRequest.input` 的单输入模型（队列在其之上）。

## 2. 当前代码锚点（已验证 @ commit fef90f44）

| 符号 | 位置 | 关键事实 |
|---|---|---|
| `TurnRequest` | `crates/fabric/src/types/turn.rs:9-16` | `{ operation_id, process_id, context: PrincipalContext, input: String, model_policy, deadline }` |
| `PrincipalContext` | `crates/fabric/src/types/local_authority.rs:279-288` | principal_id/os_principal/connection_id/thread_id/turn_id/workspace/permission_profile/approval_policy |
| 提交入口 | `crates/executive/src/service/daemon_turn/execute.rs:53-151` | flag 关直达旧路径；flag 开 enqueue 后由 session 单消费者依序执行 |
| `TurnCoordinator` | `crates/executive/src/service/turn_coordinator.rs:41-68,182-191` | `ActiveTurnKey=(principal,thread)`；持有共享 `SessionInputCoordinator` |
| turn_id 分配 | 同上 `:291` | `submit_with()` 内分配 |
| `TurnId` | `crates/fabric/src/types/session.rs:13-20` | `TurnId(pub Uuid)` |
| `ConnectionId`/`ThreadId` | `crates/fabric/src/types/local_authority.rs:15-36` | `ConnectionId(Uuid)`、`ThreadId(String)` |
| `TurnEventV1` | `crates/fabric/src/ipc/stream.rs:171-277` | 27 变体；`Interrupted`(254)、`Approval`(224) 均 outbound-only |
| Fabric 队列契约 | `crates/fabric/src/types/prompt_queue.rs:16-110` | envelope/state/snapshot/UTF-8 有界截断均已实现 |
| Executive 队列 | `crates/executive/src/service/session_input.rs:144-493` | 内存/持久 store 协调、版本冲突、FIFO interjection、指标与 canonical 事件 |
| 持久层 | `crates/executive/src/impl/session/prompt_queue_sqlite.rs:13-170` | SQLite 顺序、幂等 receipt、running/queued/consumed 重启恢复 |
| 安全点 | `crates/cognit/src/harness/linear/tool_exec.rs:172-185,533-537` | react 完成与 tool/approval-backed invoke 返回且 tool result 已吸收后 drain |

**核心事实**：队列权威态已挂到 `ActiveTurnKey=(principal,thread)` 的 Executive session 层；Cognit 只通过 `TurnServices::drain_interjections` 在安全点消费独立 synthetic user messages。

## 3. 权威归属决策（doc10 §6 八问）

1. **owner**：Fabric 定义 `PromptEnvelope`/队列类型；Executive 的新 `SessionInputCoordinator` 持有队列 + interjection buffer（挂在 `TurnCoordinator` 旁，按 `ActiveTurnKey` 分区）。
2. **scope**：队列按 `(principal_id, thread_id)` 持久化；每条 envelope 记 owner principal + connection。
3. **crash 恢复**：队列走 `SessionAppendStore` 持久化；重启区分 running-but-unconfirmed / queued / consumed（幂等 consume receipt）。
4. **fail 模式**：stale edit → conflict（不静默覆盖）；跨 principal 编辑默认拒绝；广播失败不影响权威队列态（客户端用 snapshot 重同步）。
5. **上限**：队列长度上限；interjection bytes 上限（UTF-8 边界截断）；attachment 数量上限。
6. **兼容**：flag 关闭 → 无队列，`execute_turn` 直提交（等价当前）。
7. **进 event spine**：队列变更 / interjection consumed 经 `publish_event_v2` 按 thread 分区广播。
8. **许可证**：重新实现队列/buffer 语义，不复制 Grok `xai-prompt-queue`/`xai-interjection-core`。

## 4. 类型定义

### 4.1 Fabric 类型 — `crates/fabric/src/types/prompt_queue.rs`（新文件）

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::types::admission::PrincipalId;
use crate::types::local_authority::{ConnectionId, ThreadId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PromptId(pub Uuid);
impl PromptId { pub fn new() -> Self { Self(Uuid::new_v4()) } }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptKind { Prompt, Interjection }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptState { Queued, Running, Completed, Cancelled, Rejected }

/// 队列/插话的统一信封。持久化单元。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptEnvelope {
    pub prompt_id: PromptId,
    /// 单调版本号；edit/cancel 必须带 expected version。
    pub version: u64,
    pub principal_id: PrincipalId,      // owner，永不因编辑改变
    pub connection_id: ConnectionId,    // last editor 来源
    pub thread_id: ThreadId,
    pub kind: PromptKind,
    pub content: String,                // 有界；超限截断（UTF-8 安全）
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub state: PromptState,
    /// 幂等键：重放/重连去重。
    pub idempotency_key: String,
}

/// 编辑/取消的乐观并发结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueOpResult {
    Ok { new_version: u64 },
    /// expected version 过期。返回当前 envelope 供客户端 rebase。
    Conflict { current: PromptEnvelope },
    /// 跨 principal 或 running 原地编辑等被拒。
    Rejected { reason: String },
}

/// 队列上限。
pub const MAX_QUEUE_LEN: usize = 64;
pub const MAX_PROMPT_BYTES: usize = 128 * 1024;
pub const MAX_INTERJECTION_BYTES: usize = 16 * 1024;

/// 按 thread 分区的队列快照（客户端重同步用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueSnapshot {
    pub thread_id: ThreadId,
    pub running: Option<PromptId>,
    pub pending: Vec<PromptEnvelope>,
    pub queue_position: std::collections::BTreeMap<PromptId, usize>,
}
```

### 4.2 Executive 协调器 — `crates/executive/src/service/session_input.rs`（新文件）

```rust
use async_trait::async_trait;
use fabric::types::prompt_queue::*;

/// 队列持久化端口（复用 SessionAppendStore 的 sqlite）。
#[async_trait]
pub trait PromptQueueStore: Send + Sync {
    async fn append(&self, env: &PromptEnvelope);
    async fn update(&self, env: &PromptEnvelope);
    async fn snapshot(&self, thread: &ThreadId) -> QueueSnapshot;
    /// 幂等 consume：标记已被 turn 消费，重放不重复副作用。
    async fn mark_consumed(&self, id: PromptId, receipt: &str);
}

/// session 输入协调器：按 (principal, thread) 持有队列 + interjection buffer。
/// 挂在 TurnCoordinator 旁，不进 Cognit。
pub struct SessionInputCoordinator {
    store: std::sync::Arc<dyn PromptQueueStore>,
    // 内存镜像 + 版本；权威在 store。
}

impl SessionInputCoordinator {
    /// enqueue：多客户端确定顺序（append 时分配单调 seq；同一时刻按 store 顺序）。
    pub async fn enqueue(&self, principal: PrincipalId, conn: ConnectionId,
                         thread: ThreadId, kind: PromptKind, content: String,
                         idem: String) -> PromptEnvelope { unimplemented!() }

    /// edit：乐观并发，expected_version 不匹配 → Conflict；跨 principal → Rejected；
    /// running prompt 原地编辑 → Rejected（只能转 interjection 或 enqueue next）。
    pub async fn edit(&self, id: PromptId, expected_version: u64,
                     editor: (PrincipalId, ConnectionId), new_content: String)
                     -> QueueOpResult { unimplemented!() }

    pub async fn cancel(&self, id: PromptId, expected_version: u64,
                       requester: PrincipalId) -> QueueOpResult { unimplemented!() }

    /// 取下一个待运行 prompt（turn 结束后由 coordinator 调）。
    pub async fn take_next(&self, key: &(PrincipalId, ThreadId)) -> Option<PromptEnvelope> { unimplemented!() }
}
```

### 4.3 Interjection buffer — 同文件

```rust
/// mid-turn 插话缓冲：FIFO、capped、在安全点 drain。
pub struct InterjectionBuffer {
    thread_id: ThreadId,
    // FIFO 队列，字节上限 MAX_INTERJECTION_BYTES
}

impl InterjectionBuffer {
    /// 缓冲一条插话（超字节上限 UTF-8 安全截断）。
    pub fn push(&mut self, env: PromptEnvelope) -> bool { unimplemented!() }
    /// 在安全点 drain：返回 FIFO 顺序的 synthetic user messages，逐条独立。
    pub fn drain_at_safe_point(&mut self) -> Vec<String> { unimplemented!() }
    pub fn has_pending(&self) -> bool { unimplemented!() }
}
```

## 5. 文件变更计划

| 动作 | 文件 | 理由 |
|---|---|---|
| 新增 | `crates/fabric/src/types/prompt_queue.rs` | 队列/信封类型 |
| 修改 | `crates/fabric/src/types/mod.rs` + `lib.rs` | 导出 |
| 新增 | `crates/executive/src/service/session_input.rs` | 协调器 + interjection buffer + store 端口 |
| 新增 | `crates/executive/src/impl/.../prompt_queue_sqlite.rs` | `PromptQueueStore` sqlite 实现（复用 SessionAppendStore 基础） |
| 修改 | `crates/executive/src/service/turn_coordinator.rs:36-61` | 关联 `SessionInputCoordinator`；turn 结束后 `take_next` 起下一 turn |
| 修改 | `crates/executive/src/service/turn_pipeline.rs:404-548` | 在安全点 drain interjection buffer 注入 synthetic user message |
| 修改 | `crates/executive/src/service/daemon_turn/execute.rs:19-71` | flag 开时经队列；flag 关时直提交 |
| 修改 | feature flag | `grok_hardening.prompt_queue` 默认关 |

## 6. 任务分解（TDD）

**完成证据**：T1–T18 与 §8–§9 已由提交 `5506c5a2`、`ea03f7b8`、`6a28e810`、`19d9b26f`、`cb2f2f33`、`05eccf66`、`4a1fd81d` 覆盖；定向验收为 Executive queue 8 项、SQLite reopen 1 项、Cognit safe-point 5 项、Executive strict Clippy 与 workspace fmt。

**阶段 A：Fabric 类型**
- T1. 新建 `prompt_queue.rs` 全类型。`cargo check -p fabric`。
- T2. 内容截断辅助（复用 `truncate_utf8_bytes`）+ 上限常量单测。

**阶段 B：队列语义（内存 store 先）**
- T3. 内存 `PromptQueueStore` 替身。`enqueue` 分配单调版本，两客户端并发 enqueue 顺序确定（seq 单调）。单测。
- T4. `edit` 乐观并发：expected_version 匹配 → Ok(new_version)；过期 → Conflict(current)。单测。
- T5. `edit` 跨 principal → Rejected。单测（多用户隔离）。
- T6. `edit` running prompt → Rejected。单测。
- T7. `cancel` + 版本检测。单测。
- T8. owner 不因编辑改变、last editor 更新。单测。

**阶段 C：interjection buffer**
- T9. `push` FIFO + 字节上限截断。单测。
- T10. `drain_at_safe_point` FIFO、逐条独立、不合并、drain 后清空。单测。
- T11. 幂等 consume：已 drain 的插话标 consumed，重放不重复。单测。

**阶段 D：持久化 + 恢复**
- T12. sqlite `PromptQueueStore`。crash 恢复测试：enqueue 3 条 → 重开 → snapshot 一致；running-but-unconfirmed 与 queued 可区分。
- T13. `snapshot` 按 thread 分区，不泄漏其他 thread 的 prompt text。单测。

**阶段 E：turn 循环集成（flag 后）**
- T14. `turn_coordinator`：turn 结束 `take_next` 起下一 turn。集成测试：enqueue 2 → 顺序执行。
- T15. `turn_pipeline` 安全点 drain：在 react 完成(406)/tool result 吸收后(430-473)/approval 恢复后 drain interjection，注入 synthetic user message。**不**在工具写文件中途/lease 未 settle/checkpoint 提交中 drain。集成测试断言注入点。
- T16. flag 关闭回归：`execute_turn` 直提交，行为等价当前。

**阶段 F：事件 + 收尾**
- T17. 队列变更 / interjection consumed 经 `publish_event_v2` 按 thread 广播。事件断言。
- T18. clippy/fmt；更新 §2 漂移；标注 flag 灰度。

## 7. 兼容与迁移

- **flag 关闭**：无队列层，`execute_turn` → `submit_with` 直提交（等价当前）。
- **插话 vs interrupt**：停止仍走已有 `Interrupted`/cancellation，绝不编码为文本插话（见研究文档 ../04 §6）。
- **单输入模型不变**：队列产出的仍是单 `TurnRequest.input`；插话是 turn 内 synthetic message，不改 TurnRequest 结构。
- **灰度**：`grok_hardening.prompt_queue` 默认关闭；先在单用户 daemon 开启，观察 queue depth、edit conflict、interjection dropped bytes 与 event-spine append，再扩大到多连接共享 thread。

## 8. 测试计划（映射研究文档 ../04 §8 验收方向）

| 验收方向 | 测试 |
|---|---|
| 两客户端同时 enqueue 顺序确定可观察 | T3 |
| stale edit 返回 conflict | T4 |
| owner/last editor/principal 隔离 | T5, T8 |
| 插话只在 safe point、FIFO、不合并不重复 | T10, T15 |
| cancel 与 interjection 语义不混淆 | T6, T7 + interrupt 不走文本 |
| daemon 重启不丢 queued、不重复已消费插话 | T11, T12 |

## 9. 可观测性

- 事件：`prompt.enqueued`/`prompt.edited`/`prompt.cancelled`/`interjection.consumed`（按 thread 分区）。
- 指标：`prompt_queue_depth{thread}`、`prompt_edit_conflict_total`、`interjection_dropped_bytes_total`。

## 10. 许可证

重新实现队列与 interjection 语义，不复制 Grok `xai-prompt-queue`/`xai-interjection-core` 源码。多用户 principal 归属是 Aletheon 扩展。无 NOTICE 变更。

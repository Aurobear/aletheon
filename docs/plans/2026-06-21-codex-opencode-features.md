# Codex/OpenCode Feature Integration Plan

**Date:** 2026-06-21
**Goal:** Port 4 key features from Codex/OpenCode into aletheon.

---

## Phase 1: Session 持久化

### Problem
aletheon daemon 重启后会话丢失。Codex 用 SQLite 支持 resume/fork/archive。

### Design

New module: `crates/aletheon-runtime/src/impl/session_store.rs`

```rust
pub struct SessionStore {
    db: Connection,
}

pub struct SessionRecord {
    pub session_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub status: SessionStatus,  // Active, Paused, Archived
    pub messages_json: String,  // serialized Vec<Message>
    pub metadata_json: String,  // key-value pairs
}

pub enum SessionStatus { Active, Paused, Archived }
```

### Schema
```sql
CREATE TABLE sessions (
    session_id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    status TEXT NOT NULL DEFAULT 'active',
    messages_json TEXT NOT NULL DEFAULT '[]',
    metadata_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX idx_sessions_status ON sessions(status);
CREATE INDEX idx_sessions_updated ON sessions(updated_at DESC);
```

### API
- `save(session_id, messages, metadata)` — upsert
- `load(session_id) -> Option<SessionRecord>`
- `list(status, limit) -> Vec<SessionRecord>`
- `fork(session_id, new_id) -> SessionRecord` — copy messages
- `archive(session_id)` — status → Archived
- `delete(session_id)`
- `auto_save(session_id, messages)` — called after each turn

### handler.rs integration
- Add `session_store: Arc<Mutex<SessionStore>>` field
- After each turn: auto-save messages
- "resume" method: load from store
- "sessions" method: list from store
- "new_session": create new record
- "archive": archive record

---

## Phase 2: Tool Visibility

### Problem
所有工具都暴露给模型。Codex 有 ToolExposure 控制可见性。

### Design

在 `aletheon-abi/src/tool.rs` 的 Tool trait 上添加:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExposure {
    Direct,       // 模型可见，可调用
    Deferred,     // 注册但隐藏，需要 tool_search 发现
    Hidden,       // 内部用，模型不可见
}

impl Default for ToolExposure {
    fn default() -> Self { ToolExposure::Direct }
}
```

Tool trait 添加:
```rust
fn exposure(&self) -> ToolExposure { ToolExposure::Direct }
```

### handler.rs integration
- 构建 tool definitions 时过滤: `tools.iter().filter(|t| t.exposure() != ToolExposure::Hidden)`
- 添加 `tool_search` 元工具: 搜索 Deferred 工具并返回描述

---

## Phase 3: Streaming

### Problem
notify_tx 是单向通知，不是真正的流式输出。

### Design

```rust
pub enum StreamEvent {
    TextDelta { delta: String },
    ReasoningDelta { delta: String },
    ToolCallStart { name: String, call_id: String },
    ToolCallDelta { call_id: String, args_delta: String },
    ToolResult { call_id: String, result: String },
    Usage { tokens_in: u32, tokens_out: u32 },
    Done,
    Error { message: String },
}
```

在 handler.rs 中:
- ReActLoop.run() 改为流式: 每次 LLM 返回 chunk 时发送 StreamEvent
- notify_tx 改为 `mpsc::Sender<StreamEvent>`
- 客户端收到 TextDelta 可以实时渲染

### 关键改动
- `ReActLoop::run()` 改为 `run_streaming()` 接受 `StreamSender`
- 或在 handler.rs 中包装: LLM provider 支持 streaming → 中间结果通过 notify_tx 发送

---

## Phase 4: Permission ask/reply

### Problem
ApprovalGate 是基础版，不支持 once/always/reject。

### Design

```rust
pub enum ApprovalResponse {
    Once,     // 仅本次批准
    Always,   // 批准并记住 (session scope)
    Reject,   // 拒绝
}

pub struct ApprovalManager {
    /// Session-scoped approval rules: pattern → approved
    session_approvals: HashMap<String, bool>,
    pending: HashMap<String, oneshot::Sender<ApprovalResponse>>,
}
```

### handler.rs integration
- "approval_response" 方法支持 Once/Always/Reject
- Always: 将 pattern 加入 session_approvals
- PreTool hook: 先检查 session_approvals，命中则自动放行
- 未命中: 发送 ApprovalRequest 到客户端，等待回复

---

## Implementation Order

| Phase | Feature | 估计 | 依赖 |
|---|---|---|---|
| 1 | Session 持久化 | ~300 行, +10 tests | 无 |
| 2 | Tool Visibility | ~100 行, +5 tests | 无 |
| 3 | Streaming | ~200 行, +5 tests | 无 |
| 4 | Permission ask/reply | ~200 行, +8 tests | 无 |

**总计:** ~800 行, +28 tests

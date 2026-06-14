# 共享类型 (Shared Types)

> 跨模块核心数据模型定义。

**关联模块:** 所有模块
**最后更新:** 2026-06-06

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| ContentBlock | ✅ Implemented | `message.rs` | Text, ToolUse, ToolResult, Image |
| Message | ✅ Implemented | `message.rs` | Conversation message wrapper |
| ToolCall | ✅ Implemented | `message.rs` | Tool invocation request |
| ToolResult | ✅ Implemented | `tool/mod.rs` | Tool execution result |
| PerceptionEvent | ✅ Implemented | `perception/event.rs` | System event type |
| AgentError | ⬜ Planned | — | Uses `anyhow::Error` instead of typed error |

---

## 1. 消息类型

- **ContentBlock** — Content-block 消息协议（兼容 Anthropic SDK），包含 Text, ToolUse, ToolResult, Image 四种变体
- **Message** — 对话消息包装器（role: System/User/Assistant + content: Vec<ContentBlock>）
- 代码位置: `message.rs`

## 2. 工具调用

> **Canonical ToolResult definition** — other docs MUST reference this file, not redefine these types.

- **ToolCall** — 工具调用请求（id, name, input）
- **ToolResult** — 工具执行结果（tool_call_id, content: Vec<ToolContent>, is_error, exit_code, metadata）
- **ToolContent** — 输出内容变体：Text / Image / Binary
- **ToolResultMeta** — 元数据（execution_time_ms, tokens_used, truncated）
- 代码位置: `message.rs`, `tool/mod.rs`

## 3. 感知事件

- **PerceptionEvent** — 系统事件（id, source, kind, payload, priority, timestamp）
- **EventSource** — Ebpf / Proc / Sys / Journald / Inotify / Udev / DBus
- **EventKind** — FileCreated/Modified/Deleted, ProcessStarted/Exited, NetworkConnect/Disconnect, ServiceStarted/Failed, DeviceAdded/Removed, CpuPressure/MemoryPressure/DiskPressure
- 代码位置: `perception/event.rs`

## 4. 错误类型

- **AgentError** — 分类错误（category, severity, message, source, context）
- ⬜ **Planned** — 目前使用 `anyhow::Error` 替代

---

## Implementation Summary

| Component | Code Location | Key Types |
|-----------|---------------|-----------|
| ContentBlock / Message / ToolCall | `message.rs` | `ContentBlock`, `Message`, `ToolCall` |
| ToolResult / ToolContent | `tool/mod.rs` | `ToolResult`, `ToolContent`, `ToolResultMeta` |
| PerceptionEvent | `perception/event.rs` | `PerceptionEvent`, `EventSource`, `EventKind` |

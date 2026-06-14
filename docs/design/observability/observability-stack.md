# 可观测性栈 (Observability Stack)

> OS-Agent 作为系统级服务的诊断核心，包括事件分类、Fragment Accumulator、Debug CLI、Prometheus 指标等。
>
> **从 `session-lifecycle.md` 提取** — 原文 §4.3。

**关联模块:** [Session 生命周期](../core/session-lifecycle.md), [FUSE 接口](../perception/fuse-interface.md) <!-- 健康检查/指标/调试接口 => planned docs, not yet created -->

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| EventJournal | ✅ Implemented | `session/journal.rs` | JSONL append-only log |
| Durable/Ephemeral split | ⬜ Planned | — | Event classification designed, not started |
| Fragment Accumulator | ⬜ Planned | — | Streaming delta accumulation designed |
| Debug CLI (JSON-RPC) | ⬜ Planned | — | 8 RPC methods designed |
| Prometheus metrics | ⬜ Planned | — | prometheus-client integration designed |
| FTS5 full-text search | ⬜ Planned | — | SQLite FTS5 designed |

---

## 目录

- [1. Durable/Ephemeral 事件分类](#1-durable-ephemeral-事件分类)
- [2. 流式 Fragment Accumulator](#2-流式-fragment-accumulator)
- [3. 工具调用生命周期状态机](#3-工具调用生命周期状态机)
- [4. 结构化推理日志](#4-结构化推理日志-reasoninglogger)
- [5. Token Usage 安全归一化](#5-token-usage-安全归一化)
- [6. 事件 Schema 版本控制](#6-事件-schema-版本控制)
- [7. Debug CLI — Unix Socket JSON-RPC 协议](#7-debug-cli--unix-socket-json-rpc-协议)
- [8. Prometheus Metrics 导出](#8-prometheus-metrics-导出)
- [9. 与 SessionStore EventJournal 集成](#9-与-sessionstore-eventjournal-集成)
- [10. FUSE 集成](#10-fuse-集成)
- [11. CLI 命令参考](#11-cli-命令参考)

---

## 1. Durable/Ephemeral 事件分类

事件流中的事件分为两类：持久化（Durable）和即时（Ephemeral）。持久化事件写入 JSONL 日志，可重放；即时事件仅用于实时显示，不持久化。

**EventPersistence 枚举：** Durable（持久化到 JSONL，可重放）、Ephemeral（仅实时推送，不持久化）

**分类规则：**
- Ephemeral：TextDelta、ReasoningDelta、ToolInputDelta（流式增量，仅用于实时显示）
- Durable：所有其他事件

**扩展事件体（Ephemeral）：**
- `TextDelta { message_id, delta }` — 文本生成增量
- `ReasoningDelta { reasoning_id, delta }` — 推理过程增量
- `ToolInputDelta { call_id, delta }` — 工具输入增量

**扩展事件体（Durable）：**
- `HookExecuted { hook_name, event_name, result, duration_ms }` — Hook 执行记录

## 2. 流式 Fragment Accumulator

流式增量（TextDelta/ReasoningDelta/ToolInputDelta）需要累积为完整的持久化值。Fragment Accumulator 收集增量片段，然后 flush 为单个 Durable 事件。

参考 OpenCode `createLLMEventPublisher.fragments()`，`FragmentAccumulator` 维护三类 chunk 映射：
- `text_chunks: HashMap<message_id, Vec<delta>>`
- `reasoning_chunks: HashMap<reasoning_id, Vec<delta>>`
- `tool_input_chunks: HashMap<call_id, Vec<delta>>`

核心操作：start_text/append_text/end_text（推理和工具输入同理），以及 `flush_all()` 用于中断/错误恢复时刷出所有未完成的累积。

## 3. 工具调用生命周期状态机

每次工具调用跟踪完整的生命周期状态，防止重复事件、检测不一致状态、在中断时 fail 未完成的工具调用。参考 OpenCode per-callID state machine。

**ToolCallState** 跟踪：assistant_turn_id, name, input_started, input_ended, called, settled, started_at

**ToolTracker** 核心操作：
- `register(call_id, assistant_turn_id, name)` — 注册新工具调用
- `mark_input_started/ended/called/settled` — 状态推进
- `unsettled_calls()` — 获取未完成的工具调用（用于中断恢复）
- `fail_unsettled(reason)` — 为所有未完成的工具调用生成 Failed 事件，参考 OpenCode `failUnsettledTools`
- `cleanup_settled()` — 清理已结束的工具调用（避免内存泄漏）
- `detect_inconsistencies()` — 检测不一致状态（如 called 但 input 未 ended）

## 4. 结构化推理日志 (ReasoningLogger)

与 EventJournal 的区别：EventJournal 记录会话事件（用于恢复），ReasoningLogger 记录推理过程（用于调试和审计）。

**ReasoningLogger** 特性：
- JSONL 格式，按大小轮转（默认 100MB），保留 7 天
- 日志路径：`{base_dir}/reasoning/{session_id}.jsonl`

**ReasoningEntry** 包含 timestamp, session_id, step, entry_type。

**ReasoningEntryType 变体：** LlmRequest, LlmResponse, ToolCallStarted, ToolCallCompleted, ToolCallFailed, Thinking, Checkpoint, HookExecution

核心操作：`log(entry)` 写入并检查轮转，`rotate()` 重命名当前文件并重新打开，`cleanup_old_logs()` 清理超过保留天数的旧日志。

## 5. Token Usage 安全归一化

参考 OpenCode `tokens()` helper，防御 NaN/负值，标准化为统一结构。

`safe_tokens(value: Option<i64>) -> i64` — 过滤负值，None 返回 0。

**TokenUsageBreakdown** 分项统计：input, output, reasoning, cache_read, cache_write。支持 `from_raw()` 安全构造、`total()` 总计、`accumulate()` 累加。

## 6. 事件 Schema 版本控制

**JournalHeader** 携带 format_version, session_id, created_at, schema_version，用于前向兼容的 schema 演进。

版本历史：
- v1: 初始版本，基础生命周期、工具、状态变更事件
- v2: 增加流式增量事件（TextDelta/ReasoningDelta/ToolInputDelta）
- v3: 增加 Hook 执行事件（HookExecuted）

规则：新增字段不需要 bump 版本；删除或重命名字段需要 bump 版本并提供迁移逻辑。

replay 时：第一行可能是 JournalHeader，跳过无法解析的行（前向兼容），新 schema 版本不兼容时发出警告。

## 7. Debug CLI — Unix Socket JSON-RPC 协议

Debug CLI 通过 Unix socket 与 daemon 通信，使用 JSON-RPC 2.0 协议。

**支持的 RPC 方法：**

| 方法 | 描述 | 响应类型 |
|------|------|----------|
| `session.status` | 获取当前会话状态 | JSON (active_sessions, current_step, token_usage, pending_approvals) |
| `session.subscribe` | 订阅事件流（支持过滤） | 流式 SessionEvent |
| `session.replay` | 重放会话历史 | 流式 durable SessionEvent |
| `hooks.list` | 列出已注册的 hooks | JSON array |
| `metrics.snapshot` | 获取 metrics 快照 | JSON |
| `memory.status` | 获取内存状态 | JSON (blocks, total_size) |
| `reasoning.recent` | 获取最近 N 条推理步骤 | JSON array |
| `reasoning.follow` | 流式输出推理日志（类似 tail -f） | 流式 ReasoningEntry |

## 8. Prometheus Metrics 导出

使用 `prometheus-client` crate 暴露 `/metrics` 端点，默认监听 `127.0.0.1:9090`。

**指标分类：**

| 类别 | 指标 |
|------|------|
| 推理性能 | inference_duration (Histogram), llm_call_duration (Histogram) |
| Token 消耗 | tokens_input/output/reasoning/cache_read/cache_write_total (Counter) |
| 工具调用 | tool_calls_total/success/failed (Counter), tool_call_duration (Histogram) |
| 会话 | active_sessions (Gauge), sessions_created/resumed/compressed_total (Counter) |
| Hook | hook_executions_total (Counter), hook_execution_duration (Histogram), hook_blocks_total (Counter) |
| 检查点 | checkpoint_duration (Histogram), checkpoints_total (Counter) |
| 系统 | memory_usage_bytes (Gauge), journal_size_bytes (Gauge), db_size_bytes (Gauge) |

核心操作：`record_inference(duration, usage)`, `record_tool_call(tool_name, duration, success)`, `record_hook_execution(duration, blocked)`

## 9. 与 SessionStore EventJournal 集成

**EventPublisher** 解耦事件生产者（SessionLoop）和消费者（EventJournal、DebugCLI、MetricsExporter）。

架构：接收 SessionEvent → 推送给实时订阅者（所有事件） → Durable 事件写入 JSONL 日志 → 更新 metrics → 更新 FragmentAccumulator 和 ToolTracker。

`add_live_subscriber()` 返回 `mpsc::Receiver<SessionEvent>` 用于 Debug CLI 流式输出。`cleanup_subscribers()` 移除已断开的订阅者。

## 10. FUSE 集成

**ReasoningFuseMount** 将推理日志暴露为只读文件系统：
- 路径：`/mnt/agent/logs/reasoning/{session_id}.jsonl`
- 支持：readdir（列出 .jsonl 文件）、open/read（读取日志内容）、getattr（文件元信息）
- 不支持写入操作

## 11. CLI 命令参考

```bash
# 查看当前会话状态
agent-cli debug status

# 流式输出推理日志
agent-cli debug follow-reasoning [--filter "Thinking|ToolCall*"]

# 查看最近 N 条推理步骤
agent-cli debug recent-steps [--n 20]

# 查看内存使用
agent-cli debug memory

# 查看 Hook 注册状态
agent-cli debug hooks

# 查看 Prometheus metrics
agent-cli debug metrics

# 重放指定会话历史
agent-cli debug replay --session-id <id> [--from-seq 0]

# 订阅实时事件流
agent-cli debug subscribe [--filter "ToolCall*"] [--ephemeral]
```

---

## Implementation Summary

**Code locations:**
- `session/journal.rs` — EventJournal (JSONL append-only log)

**Key types/traits designed (not yet implemented):**
- `EventPersistence` — Durable/Ephemeral event classification
- `FragmentAccumulator` — streaming delta accumulation for TextDelta/ReasoningDelta/ToolInputDelta
- `ToolTracker` — per-callID tool call lifecycle state machine (input_started/input_ended/called/settled)
- `ReasoningLogger` — structured reasoning log with rotation and retention
- `TokenUsageBreakdown` — safe token usage normalization with NaN/negative defense
- `JournalHeader` — event schema versioning for forward compatibility
- `DebugCli` — Unix socket JSON-RPC 2.0 client with 8 RPC methods
- `MetricsExporter` — Prometheus metrics exporter (prometheus-client crate)
- `EventPublisher` — event fan-out to journal, live subscribers, metrics, and state trackers
- `ReasoningFuseMount` — FUSE read-only view of reasoning logs

**Planned (not started):**
- Durable/Ephemeral event classification — designed but no code started
- Fragment Accumulator — streaming delta accumulation designed
- Debug CLI (JSON-RPC) — 8 RPC methods designed
- Prometheus metrics — prometheus-client integration designed
- FTS5 full-text search — SQLite FTS5 designed

**Test coverage:** No dedicated test suite documented for the observability stack.

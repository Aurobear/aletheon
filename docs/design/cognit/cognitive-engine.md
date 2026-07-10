# 认知引擎 (Cognitive Engine)

> Migrated from docs/design/core/cognitive-engine.md — code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)
> Note: Context compaction/compression moved to runtime/memory; LoopDetector integration moved to self/loop-detector.md

> 驱动 Agent 推理与决策的核心循环，采用 ReAct 工具循环 + content-block 消息协议。主动行为引擎。

**模块编号:** 01
**关联模块:** [memory-system](memory-system.md), [tool-system](../execution/tool-system.md)
**最后更新:** 2026-06-06

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| ReAct loop | ✅ Implemented | `engine.rs:run_turn()` | Core tool loop works end-to-end |
| ContentBlock types | ✅ Implemented | `message.rs` | Text, ToolUse, ToolResult, Image |
| Context compaction | ✅ Implemented | `memory/compressor/` | AdvancedCompressor with token-budget tail protection, iterative summary, tool output pre-pruning. Old `CompactionManager` in `memory/compaction.rs` kept for reference |
| Streaming | ✅ Implemented | `llm/provider.rs:complete_stream()` | `LlmStream` trait with SSE chunk streaming |
| Checkpointable trait | ⬜ Planned | — | `session/journal.rs` has `EventJournal` instead |
| LoopDetector integration | ✅ Implemented | `security/loop_detector.rs` | Wired to engine via `pre_check()`/`post_check()` |

**Stale references fixed:** `tool_runner.rs` → `security/runner.rs`, `channel.rs` → removed (does not exist), `checkpoint.rs` → `session/journal.rs`

---

## 目录

- [1. 概述](#1-概述)
- [2. 当前设计](#2-当前设计)
  - [2.1 ReAct 推理循环](#21-react-推理循环)
  - [2.2 Content-Block 消息协议](#22-content-block-消息协议)
- [5. 实现要点](#5-实现要点)
- [6. 参考来源](#6-参考来源)

---

## 1. 概述

认知引擎是 Aletheon 的推理核心。它实现了 Anthropic SDK 的 ReAct (Think-Act-Observe) 工具循环模式，负责：

1. 接收用户请求或系统感知事件
2. 分析当前状态与目标
3. 制定计划并选择工具
4. 执行工具调用并观察结果
5. 根据结果决定是否继续推理或返回最终响应

认知引擎通过 content-block 协议统一所有 Agent 通信格式，并具备上下文压缩能力以支持长会话。

---

## 2. 当前设计

### 2.1 ReAct 推理循环

**ReAct loop** — 采用 Anthropic SDK 的 Think-Act-Observe 工具循环模式，驱动 Agent 推理与决策。
- 代码位置: `engine.rs:run_turn()`

```
┌─────────────────────────────────────────────────────────┐
│                    认知引擎                               │
│                                                         │
│  ┌───────────────────────────────────────────────────┐  │
│  │              推理循环 (Think-Act-Observe)          │  │
│  │                                                   │  │
│  │  ┌──────────┐   ┌──────────┐   ┌──────────┐     │  │
│  │  │ THINK    │──▶│ PLAN     │──▶│ ACT      │     │  │
│  │  │          │   │          │   │          │     │  │
│  │  │ 分析当前  │   │ 制定计划  │   │ 执行动作  │     │  │
│  │  │ 状态和   │   │ 分解步骤  │   │ 调用工具  │     │  │
│  │  │ 目标     │   │ 选择策略  │   │ 观察结果  │     │  │
│  │  └──────────┘   └──────────┘   └──────────┘     │  │
│  │       ▲                                  │        │  │
│  │       │                                  │        │  │
│  │       └──────────────────────────────────┘        │  │
│  │                   反馈循环                          │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### 2.2 Content-Block 消息协议

**ContentBlock** — 统一的 content-block 消息格式（借鉴 Anthropic SDK），用于所有 Agent 通信。
- 代码位置: `message.rs`
- 包含 Text, ToolUse, ToolResult, Image 四种变体
- 与 LLM API 原生格式对齐，减少转换开销；`ToolResult` 的 `is_error` 字段实现结构化工具错误

## 6. 参考来源

| 来源 | 关键文件 | 借鉴内容 |
|------|----------|----------|
| Anthropic SDK | `lib/tools/_beta_runner.py:261` | ReAct 工具循环 (`__run__`) |
| Anthropic SDK | `lib/tools/_beta_runner.py:177` | 上下文压缩 (`_check_and_compact`) |
| Anthropic SDK | `lib/tools/_beta_builtin_memory_tool.py:55` | 文件系统记忆工具 |
| Letta (MemGPT) | `letta/services/summarizer/compact.py` | 便宜模型压缩策略 |
| LangGraph | `langgraph/pregel/_loop.py:592-713` | 超步循环 + 检查点恢复 |
| LangGraph | `langgraph/pregel/_algo.py:232` | `apply_writes` 原子写入 |

---

## Implementation Summary

| Component | Code Location | Key Types |
|-----------|---------------|-----------|
| ReAct loop | `engine.rs:run_turn()` | `Engine`, `TurnConfig`, `TurnResult` |
| ContentBlock protocol | `message.rs` | `ContentBlock` (Text/ToolUse/ToolResult/Image), `Message` |
| LoopDetector integration | `security/loop_detector.rs` | `LoopDetector`, `pre_check()`, `post_check()` |
| Session journal | `session/journal.rs` | `EventJournal` (替代 Checkpointable) |

# 认知引擎 (Cognitive Engine)

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
  - [2.3 上下文压缩](#23-上下文压缩)
- [3. 已识别缺陷](#3-已识别缺陷)
  - [3.1 Session 持久化缺失（影响循环稳定性）](#31-session-持久化缺失影响循环稳定性)
  - [3.2 无迭代深度保护](#32-无迭代深度保护)
  - [3.3 P1: 迭代深度保护与 LoopDetector 集成](#33-p1-迭代深度保护与-loopdetector-集成)
- [4. 改进设计](#4-改进设计)
  - [4.1 Session 持久化检查点接口](#41-session-持久化检查点接口)
  - [4.2 LoopDetector 集成到 ReAct 循环](#42-loopdetector-集成到-react-循环)
- [5. 实现要点](#5-实现要点)
- [6. 参考来源](#6-参考来源)

---

## 1. 概述

认知引擎是 OS-Agent 的推理核心。它实现了 Anthropic SDK 的 ReAct (Think-Act-Observe) 工具循环模式，负责：

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

### 2.3 上下文压缩

> ⬜ **Planned** — 保持完整设计。

借鉴 Anthropic SDK 的 `_check_and_compact()` + Letta 的 `compact.py`：

```rust
async fn compact(&mut self) {
    // 用便宜模型压缩 (如 Qwen3-8B 本地)
    let summary = self.summarizer
        .summarize(&self.recent_messages, SummarizeModel::Local)
        .await?;

    // 旧消息移入 Recall Memory (SQLite)
    self.recall_db.store(&self.evicted_messages).await?;

    // 关键事实提取 → Archival Memory (向量库)
    let facts = self.extract_key_facts(&self.evicted_messages).await;
    for fact in facts {
        self.archival_db.insert(fact).await?;
    }

    // 上下文替换为摘要
    self.messages = vec![ContentBlock::Text(summary)];
}
```

**压缩触发条件：**
- Token 计数超过阈值（默认 70% 上下文窗口）
- 可由 `ContextBudget` 模块精确追踪（详见 [memory-system](memory-system.md) §2.2）

---

## 3. 已识别缺陷

### 3.1 Session 持久化缺失（影响循环稳定性）

**严重程度:** P0

当前 ReAct 循环是纯内存态。如果 agentd 进程崩溃或重启，正在进行的推理状态（`messages`、`tool_call` 上下文、当前迭代计数）全部丢失。

**影响范围：**
- 长时间任务（编译、测试流水线）中断后无法恢复
- 用户需要从头重新描述任务
- 已执行的工具调用结果无法复用

**缓解方案：** 需要引入检查点机制（借鉴 LangGraph 的 checkpoint + 版本调度），将推理状态序列化到 SQLite。此问题的具体解决方案属于会话持久化模块的设计范畴，认知引擎侧需要实现 `Checkpointable` trait。

### 3.2 无迭代深度保护

**严重程度:** P1

当前 `should_stop()` 仅检查最大迭代次数。缺少对单次推理中工具调用数量的细粒度控制，可能因为工具输出循环（工具 A 的输出触发工具 B，B 的输出又触发 A）导致无限循环。

### 3.3 P1: 迭代深度保护与 LoopDetector 集成

**严重程度:** P1

认知引擎的 ReAct 推理循环 (§2.1) 当前仅通过 `should_stop()` 检查最大迭代次数来终止循环，缺乏对工具调用序列的细粒度防护。存在三个具体缺陷：

**缺陷 1: 无风险分级阈值** — `should_stop()` 使用单一的 `MaxIterationsExceeded` 阈值，不区分工具调用的风险等级。只读操作（`grep`、`ls`）和破坏性操作（`rm`、`mkfs`）共用同一上限，导致低风险探索性调用过早被截断，高风险破坏性操作拥有过大的尝试窗口。

**缺陷 2: 无停滞检测（Stagnation Detection）** — 安全模型的 `LoopDetector` (`security/security-model.md:948-974`) 实现了 `check_stagnation()` 方法（连续 K 次调用无成功结果且 token 消耗变化低于阈值时触发 `Warn`），但 ReAct 循环完全没有调用此检测。典型场景：Agent 尝试 `make` 失败 -> 读取 Makefile -> 重新 `make` -> 再次失败 -> 循环。

**缺陷 3: 无连续失败检测（Fail-Streak Detection）** — 安全模型的 `LoopDetector` (`security/security-model.md:920-945`) 实现了 `check_fail_streak()`（连续 M 次失败触发 `Escalate` 升级到人工介入）。认知引擎未集成此检测，导致工具连续返回 `is_error: true` 时 Agent 不会停下来反思。

安全模型中已定义的阈值 (`security/security-model.md:243-249`)：

| 风险等级 | same_call_threshold | fail_streak_threshold |
|----------|---------------------|-----------------------|
| ReadOnly | 5                   | 7                     |
| FileModification | 3               | 5                     |
| SystemChange | 2                 | 3                     |
| Destructive | 2                  | 2                     |

这些阈值已设计完备，但认知引擎侧没有任何集成点。安全模型的 `CircuitBreaker` (`security/security-model.md:570-661`) 设计了 per-turn 的连续阻断检测（3 次 Block -> InterruptTurn），但因认知引擎不调用 `LoopDetector`，熔断器从未被触发。

---

## 4. 改进设计

针对 §3.1，认知引擎需要实现检查点接口：

```rust
#[async_trait]
trait Checkpointable {
    async fn checkpoint(&self) -> Result<CheckpointData>;
    async fn restore(data: CheckpointData) -> Result<Self> where Self: Sized;
}
```

> ⬜ **Planned** — CheckpointData 结构体定义、恢复逻辑保持完整设计。实现使用 EventJournal 替代（见 session-lifecycle.md）。

---

### 4.2 LoopDetector 集成到 ReAct 循环

安全模型 (`security/security-model.md`) 已经设计了完整的 `LoopDetector` 及其子系统（RiskClassifier、CircuitBreaker、OutputGuardrail）。问题不在于缺少防护组件，而在于认知引擎未消费这些组件。

**改动概要：**
1. 在 ReAct 循环的 tool call 前注入 LoopDetector pre-check（风险分级 + 循环检测）
2. 认知引擎持有 `loop_detector: LoopDetector` 字段，与 `policy_engine` 并列
3. 复用安全模型的 RiskClassifier 四级风险分类阈值（`security/security-model.md:243-249`）
4. 遵循 fail-closed 语义：LoopDetector 自身出错时阻断调用并记录告警

具体集成点见 `security/loop_detector.rs` 中的 `pre_check()` / `post_check()` / `validate_output()` 接口。

---

| 项目 | 说明 |
|------|------|
| **核心循环** | `agent-core/src/engine.rs` — ReAct loop (`run_turn()`), 参考 Anthropic SDK `lib/tools/_beta_runner.py:261` |
| **压缩逻辑** | `agent-core/src/engine.rs:396` — 目前仅 warning，未实现实际压缩 |
| **消息协议** | `agent-core/src/message.rs` — ContentBlock enum，序列化对齐 Anthropic API |
| **检查点** | `agent-core/src/session/journal.rs` — EventJournal 追加式日志 (替代原 checkpoint.rs 设计) |
| **停止条件** | MaxIterations + MaxTokens + Timeout，可组合（And/Or） |

---

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

# 记忆系统 (Memory System)

> 借鉴 Letta (MemGPT) 的三级自编辑记忆架构，让 Agent 能像 OS 管理虚拟内存一样管理自己的记忆。自学习循环。

**模块编号:** 02
**关联模块:** [cognitive-engine](cognitive-engine.md), [tool-system](../execution/tool-system.md)
**最后更新:** 2026-06-06

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| CoreMemory (L1) | ✅ Implemented | `crates/agent-core/src/memory/core_memory.rs` | Block-based in-context memory with self-edit tools |
| RecallMemory (L2) | ✅ Implemented | `crates/agent-core/src/memory/recall_memory.rs` | SQLite-backed conversation history |
| ArchivalMemory (L3) | ✅ Implemented | `crates/agent-core/src/memory/archival_memory.rs` | `InMemoryArchival` (keyword search) + `VectorArchival` (vector-backed via VectorStore) |
| Memory tools | ✅ Implemented | `crates/agent-core/src/memory/tools.rs` | core_memory_append/replace/recall_search etc. |
| ContextBudget | ✅ Implemented | `crates/agent-core/src/memory/budget.rs` | Token budget tracking |
| AdvancedCompressor | ✅ Implemented | `crates/agent-core/src/memory/compressor/mod.rs` | Token-budget tail protection with iterative summary updates |
| Tail Protection | ✅ Implemented | `crates/agent-core/src/memory/compressor/tail.rs` | `TailProtectionConfig`, `find_tail_cut()` — soft ceiling + hard minimum + boundary alignment |
| Summary Template | ✅ Implemented | `crates/agent-core/src/memory/compressor/template.rs` | `SummaryTemplate` with `render()` and `render_iterative()` for iterative summary updates |
| MemoryScope | ✅ Implemented | `crates/agent-core/src/memory/scope.rs` | 3-tier isolation (Global/Session/Agent) with `ScopedCoreMemory`, `PendingWrite` approval, `Scratchpad` |
| Scoped Recall | ✅ Implemented | `crates/agent-core/src/memory/scope.rs` | `ScopeFilter`, `ScopedRecallFilter` — scope-aware recall queries via metadata JSON |
| MemoryPipeline | ✅ Implemented | `crates/agent-core/src/memory/pipeline/mod.rs` | Two-phase pipeline: Phase1 extraction + Phase2 consolidation |
| Phase1Extractor | ✅ Implemented | `crates/agent-core/src/memory/pipeline/phase1.rs` | Parallel session extraction with lease-based claiming |
| Phase2Consolidator | ✅ Implemented | `crates/agent-core/src/memory/pipeline/phase2.rs` | Global lock, rollout summaries, raw_memories.md output |
| StateDatabase | ✅ Implemented | `crates/agent-core/src/memory/pipeline/state_db.rs` | In-memory session tracking with lease/watermark |
| Vector DB | ✅ Implemented | `crates/agent-core/src/memory/vector_store.rs` | QdrantVectorStore, LanceVectorStore, OpenAIEmbedder |

---

## 目录

- [1. 概述](#1-概述)
- [2. 当前设计](#2-当前设计)
  - [2.1 三级记忆架构](#21-三级记忆架构)
  - [2.2 上下文预算追踪](#22-上下文预算追踪)
  - [2.3 与 OS 感知的结合](#23-与-os-感知的结合)
  - [2.4 Rust 结构定义](#24-rust-结构定义)
- [3. 已识别缺陷](#3-已识别缺陷)
  - [3.1 崩溃时记忆损坏](#31-崩溃时记忆损坏session-持久化问题的延伸)
  - [3.2 向量数据库选型未确定](#32-向量数据库选型未确定)
  - [3.3 P1: 多 Agent 记忆隔离缺失](#33-p1-多-agent-记忆隔离缺失)
- [4. 改进设计](#4-改进设计)
  - [4.1 原子化 Core Memory 更新](#41-原子化-core-memory-更新)
  - [4.2 记忆恢复流程](#42-记忆恢复流程)
  - [4.3 MemoryScope — 三级记忆作用域](#43-memoryscope--三级记忆作用域)
- [5. 实现要点](#5-实现要点)
- [6. 参考来源](#6-参考来源)

---

## 1. 概述

记忆系统是 OS-Agent 持久化认知的基础。它借鉴 Letta (MemGPT) 的三级记忆架构，将记忆分为三个层级：

- **L1 Core Memory** — 上下文窗口内的可编辑块，Agent 可自行管理
- **L2 Recall Memory** — SQLite 存储的完整对话历史与工具调用记录
- **L3 Archival Memory** — 向量数据库存储的长期知识与模式

三层之间通过压缩/驱逐机制联动，类似 OS 的 CPU cache → RAM → disk 层次结构。记忆系统同时维护上下文预算，确保推理循环不会因 token 超限而失败。

---

## 2. 当前设计

### 2.1 三级记忆架构

```
┌─────────────────────────────────────────────────────────────┐
│                    记忆系统                                   │
│                                                             │
│  ┌───────────────────────────────────────────────────────┐  │
│  │  L1: Core Memory (核心记忆) — 在上下文窗口内           │  │
│  │                                                       │  │
│  │  Block 结构: label + value + limit + read_only        │  │
│  │                                                       │  │
│  │  示例 blocks:                                         │  │
│  │  • system_state: "当前焦点: coding, CPU: 45%, ..."    │  │
│  │  • user_prefs: "偏好 Arch, 用 vim, 英文优先..."       │  │
│  │  • safety_rules: "禁止 rm -rf /" (read_only)          │  │
│  │                                                       │  │
│  │  Agent 自编辑工具:                                    │  │
│  │  • core_memory_append(label, content)                 │  │
│  │  • core_memory_replace(label, old, new)               │  │
│  │  • core_memory_rethink(label, new_content)            │  │
│  └───────────────────────────────────────────────────────┘  │
│                         │ 定期压缩/驱逐                      │
│                         ▼                                    │
│  ┌───────────────────────────────────────────────────────┐  │
│  │  L2: Recall Memory (回忆记忆) — SQLite                 │  │
│  │                                                       │  │
│  │  存储: 完整对话历史 + 工具调用记录 + 系统事件          │  │
│  │  索引: 时间戳 + 会话ID + 事件类型                      │  │
│  │  查询: conversation_search, event_search,              │  │
│  │        tool_call_search                               │  │
│  │  容量: GB 级，保留最近 7 天完整 + 更早的摘要           │  │
│  └───────────────────────────────────────────────────────┘  │
│                         │ 向量化存储                         │
│                         ▼                                    │
│  ┌───────────────────────────────────────────────────────┐  │
│  │  L3: Archival Memory (归档记忆) — 向量数据库            │  │
│  │                                                       │  │
│  │  存储: 长期知识 + 用户习惯模式 + 历史决策              │  │
│  │  检索: archival_memory_insert/search, pattern_match    │  │
│  │  容量: TB 级，持久化                                   │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

**L1 Core Memory 设计要点：**
- Block 是上下文窗口内的可编辑单元，每个 block 有 `label`（标识）、`value`（内容）、`limit`（字符上限）、`read_only`（权限标志）
- Agent 通过 `core_memory_append`、`core_memory_replace`、`core_memory_rethink` 三个工具自编辑记忆（借鉴 Letta `letta/functions/function_sets/base.py:246-280`）
- `read_only` block（如 `safety_rules`）由系统注入，Agent 不可修改
- Core Memory 内容直接注入到 LLM 的系统提示中，占用上下文窗口

**L2 Recall Memory 设计要点：**
- SQLite 存储，保留最近 7 天完整记录
- 支持按时间、会话、事件类型、工具名称多维查询
- 对话历史超 7 天后自动压缩为摘要

**L3 Archival Memory 设计要点：**
- 向量数据库（ChromaDB/Qdrant/LanceDB），存储长期知识
- 支持语义检索 + 标签过滤
- `pattern_match` 可检索历史类似情境，辅助决策

### 2.2 上下文预算追踪

借鉴 Letta 的 `ContextWindowOverview`（`letta/schemas/memory.py:23-65`）：

```
┌───────────────────────────────────────────────────────┐
│  上下文预算追踪 (ContextWindowOverview)                 │
│                                                       │
│  系统提示:     1200 tokens  ████████░░░░░░░░          │
│  Core Memory:   800 tokens  █████░░░░░░░░░░░          │
│  工具定义:      600 tokens  ████░░░░░░░░░░░░          │
│  对话消息:     4000 tokens  ██████████████████████    │
│  ──────────────────────────────────────               │
│  总计:         6600 / 8192 tokens                     │
│  剩余:         1592 tokens                            │
│  压缩触发:     >7000 tokens → 自动压缩                │
└───────────────────────────────────────────────────────┘
```

**预算管理规则：**
- 每次 LLM 调用前，按类别统计当前 token 消耗
- 总量超过阈值（默认 70%）时触发上下文压缩
- Core Memory 各 block 的 `limit` 字段防止单个 block 膨胀
- 压缩优先驱逐低优先级的对话消息，保留 Core Memory

### 2.3 与 OS 感知的结合

Core Memory 中的 `system_state` block 由感知引擎自动更新。`system_state` block 是 `read_only: false`，由感知引擎周期性覆盖。更新频率由感知事件的 `Priority` 决定（Critical → 立即，High → 下个周期，Normal → 按需）。

### 2.4 Rust 结构定义

- **MemorySystem** — 包含 CoreMemory (L1)、RecallDatabase (L2 SQLite)、ArchivalDatabase (L3 向量库)、ContextBudget、Summarizer
- **CoreMemory** — Block 数组，每个 block 有 label/value/limit/read_only
- 代码位置: `memory/core_memory.rs`, `memory/recall_memory.rs`, `memory/budget.rs`

---

## 3. 已识别缺陷

### 3.1 崩溃时记忆损坏（Session 持久化问题的延伸）

**严重程度:** P0

如果 agentd 进程在 Core Memory 更新（`append`/`replace`/`rethink`）中途崩溃，可能出现 block 值部分写入或 Recall Memory 与 Core Memory 状态不一致。

**缓解方案：** 由 [Session 持久化模块](session-lifecycle.md) 统一解决。记忆系统侧需要：
- Core Memory 更新操作应原子化（先写 WAL，再 apply）
- Recall Memory 的 SQLite 本身支持事务
- 恢复时从最近检查点重建 Core Memory 状态

### 3.2 向量数据库选型未确定

**严重程度:** P2

L3 Archival Memory 的向量数据库在 ChromaDB、Qdrant、LanceDB 之间尚未最终选型。需要在 Phase 2 实现前完成 POC 对比，关注：嵌入式部署友好度、Rust binding 成熟度、查询延迟。

### 3.3 P1: 多 Agent 记忆隔离缺失

**严重程度:** P1

**问题描述：** 记忆系统的三层架构在单代理场景下运作良好，但在多代理场景下完全缺乏作用域隔离：

- **L1 Core Memory 全局共享且可写** — 所有子代理看到相同内容，子代理可修改影响所有其他代理
- **L2 Recall Memory 无代理标识** — 所有代理的历史记录在同一张表中，子代理中间推理过程污染全局召回空间
- **L3 Archival Memory 无归档标签** — 向量相似度检索无法区分知识来源
- **无任务级工作记忆** — 子代理没有临时存储中间结果的暂存空间

**参考来源：**
- Letta Per-Conversation Memory：每个对话独立的内存空间
- CrewAI Layered Scoped Memory：Agent-scoped / Task-scoped / Shared 三层作用域

---

## 4. 改进设计

### 4.1 原子化 Core Memory 更新

采用 WAL (Write-Ahead Log) 模式实现原子更新：先写 WAL entry，再 apply 到内存，最后标记已提交。read_only block 的更新被拒绝。

### 4.2 记忆恢复流程

从检查点 + WAL 恢复：加载最近检查点，重放未提交的 WAL entries 重建内存状态。

### 4.3 MemoryScope — 三级记忆作用域

解决 §3.3 多 Agent 记忆隔离缺失的问题。

#### 4.3.1 MemoryScope 定义

```rust
enum MemoryScope {
    /// 全局作用域 — 安全规则、用户偏好等共享知识
    /// 所有代理可读，仅父代理可写
    Global,
    /// 会话作用域 — 父代理的工作记忆 + 当前会话上下文
    /// 父代理可读写，子代理可读，子代理写入需审批
    Session,
    /// 代理作用域 — 单个代理的私有工作记忆
    /// 仅拥有者可读写，任务结束后可选保留或丢弃
    Agent(String), // agent_id
}
```

#### 4.3.2 Core Memory 作用域化

将 Core Memory 分区为不同作用域的记忆块。`ScopedMemoryBlock` 包含 scope, label, content, read_only。

子代理的系统提示词仅注入 Global + Session 作用域的记忆块，不注入其他子代理的 AgentScope。

#### 4.3.3 Recall Memory 作用域过滤

`RecallQuery` 包含 scope_filter 字段，子代理默认只能查询 Global + 自己的 AgentScope。查询 SessionScope 需父代理授权。

SQLite 表添加 `scope_type` 和 `scope_id` 列，带索引。

#### 4.3.4 Archival Memory 作用域标签

`ArchivalEntry` 的 metadata 包含 scope, agent_id, task_id, created_at。检索时可选按作用域过滤，子代理自动过滤为 Global + 自己的 AgentScope。

#### 4.3.5 任务级暂存空间（Scratchpad）

为每个子代理任务提供临时工作记忆：

`Scratchpad` 包含 agent_id, task_id, entries, retention。`RetentionPolicy` 枚举：Discard（任务结束后丢弃）、ArchiveToAgent（归档到 AgentScope）、ArchiveToSession（归档到 SessionScope，需审批）。

**写入控制策略：**
- 子代理写入 GlobalScope 被拒绝
- 写入 SessionScope 需父代理批准
- 写入 AgentScope 默认允许
- 现有单代理场景的行为不变，作用域默认为 Global（向后兼容）

---

## 5. 实现要点

| 项目 | 说明 |
|------|------|
| **Core Memory** | `agent-core/src/memory.rs` — MemoryBlock 结构 + 自编辑工具，参考 Letta `letta/schemas/block.py:67-68` |
| **Recall Memory** | `agent-core/src/memory.rs` — SQLite schema + 查询接口，参考 Letta `letta/schemas/memory.py:68-77` |
| **Archival Memory** | `agent-core/src/memory.rs` — 向量 DB wrapper，待选型 |
| **预算追踪** | `agent-core/src/memory.rs` — ContextBudget，参考 Letta `letta/schemas/memory.py:23-65` |
| **压缩器** | `agent-core/src/memory.rs` — Summarizer trait，本地便宜模型 + 云端 fallback |
| **WAL** | `agent-core/src/checkpoint.rs` — 与 Session 持久化共用 WAL |

---

## 6. 参考来源

| 来源 | 关键文件 | 借鉴内容 |
|------|----------|----------|
| Letta (MemGPT) | `letta/schemas/block.py:67-68` | MemoryBlock 定义 |
| Letta (MemGPT) | `letta/schemas/memory.py:68-77` | Memory 类（三级记忆容器） |
| Letta (MemGPT) | `letta/schemas/memory.py:23-65` | ContextWindowOverview（预算追踪） |
| Letta (MemGPT) | `letta/functions/function_sets/base.py:246-280` | `core_memory_append` / `core_memory_replace` |
| Letta (MemGPT) | `letta/services/summarizer/compact.py` | 便宜模型压缩实现 |
| Anthropic SDK | `lib/tools/_beta_runner.py:177` | `_check_and_compact` 上下文压缩触发 |

---

## Implementation Summary

**Code locations (all under `crates/agent-core/src/memory/`):**
- `core_memory.rs` — CoreMemory (L1) with block-based in-context memory and self-edit tools
- `recall_memory.rs` — RecallMemory (L2) with SQLite-backed conversation history
- `archival_memory.rs` — ArchivalMemory (L3) with `InMemoryArchival` (keyword search) and `VectorArchival` (vector-backed semantic search)
- `tools.rs` — Memory tools (core_memory_append/replace/recall_search etc.)
- `budget.rs` — ContextBudget (token budget tracking)
- `compressor/mod.rs` — `AdvancedCompressor` with token-budget tail protection and iterative summary generation
- `compressor/tail.rs` — `TailProtectionConfig` and `find_tail_cut()` — soft ceiling (1.5x budget), hard minimum (3 messages), boundary alignment (avoids splitting tool call/result pairs)
- `compressor/template.rs` — `SummaryTemplate` with `render()` (full summarization) and `render_iterative()` (incremental update of existing summary)
- `scope.rs` — `MemoryScope` (Global/Session/Agent), `ScopedCoreMemory` with `PendingWrite` approval flow, `Scratchpad` with `RetentionPolicy`, `ScopeFilter`/`ScopedRecallFilter` for scope-aware recall queries
- `pipeline/mod.rs` — `MemoryPipeline` orchestrating Phase 1 then Phase 2
- `pipeline/phase1.rs` — `Phase1Extractor` for parallel session extraction with secret redaction
- `pipeline/phase2.rs` — `Phase2Consolidator` for global consolidation into `raw_memories.md`
- `pipeline/state_db.rs` — `StateDatabase` for in-memory session tracking with lease/watermark
- `vector_store.rs` — `VectorStore` trait with QdrantVectorStore, LanceVectorStore implementations; `Embedder` trait with OpenAIEmbedder

**Key types/traits implemented:**
- `CoreMemory` — Block-based in-context memory with label/value/limit/read_only
- `MemoryBlock` — individual memory block definition
- `ContextBudget` — token budget tracking and compression trigger
- `AdvancedCompressor` — context compaction with token-budget tail protection, iterative summary via `SummaryTemplate`
- `MemoryScope` — 3-tier visibility scope (Global/Session/Agent) with per-agent read/write/request-write permission model
- `ScopedCoreMemory` — scope-enforcing wrapper over `CoreMemory` with `PendingWrite` approval queue
- `Scratchpad` — task-level ephemeral workspace with `RetentionPolicy` (Discard/ArchiveToAgent/ArchiveToSession)
- `MemoryPipeline` — two-phase memory consolidation (extraction + global merge)
- `Phase1Extractor` / `Phase2Consolidator` — pipeline stage implementations
- `StateDatabase` — session lifecycle tracking (claim/lease/watermark)
- Memory tools: core_memory_append, core_memory_replace, core_memory_rethink, recall_search, archival_memory_insert, archival_memory_search

**Planned (not started):**
- WAL-based atomic Core Memory updates
- Checkpoint-based memory recovery (WAL + checkpoint replay)

**Test coverage:**
- `scope.rs` — 15+ unit tests covering permission model, ScopedCoreMemory read/write/approve/reject, Scratchpad operations, ScopeFilter
- `compressor/mod.rs` — basic construction test
- `compressor/tail.rs` — 2 tests for short/long conversation tail cut
- `compressor/template.rs` — 2 tests for render/render_iterative
- `pipeline/mod.rs` — 5 tests including full integration test (Phase1 + Phase2 with tempdir)
- `archival_memory.rs` — tests in InMemoryArchival (via trait impl tests)

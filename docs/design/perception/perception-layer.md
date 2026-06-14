# 感知层与内核交互

> 从内核事件到用户行为，Agent 的全栈感知引擎。

**模块编号:** 04
**关联模块:** [认知引擎](../core/cognitive-engine.md) · [记忆系统](../core/memory-system.md) · [安全模型](../security/security-model.md) · [系统管理](system-management.md) · [FUSE 接口](fuse-interface.md)
**最后更新:** 2026-06-06

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| PerceptionEvent | ✅ Implemented | `perception/event.rs` | Event type definitions |
| PerceptionManager | ✅ Implemented | `perception/manager.rs` | Event routing and lifecycle |
| EventAggregator | ✅ Implemented | `perception/aggregator.rs` | Event deduplication and batching |
| ProcSource | ✅ Implemented | `perception/sources/proc_source.rs` | /proc filesystem monitoring |
| InotifySource | 🔶 Partial | `perception/sources/inotify_source.rs` | Polling-based, not real inotify |
| JournaldSource | ✅ Implemented | `perception/sources/journald_source.rs` | systemd journal reader |
| eBPF source | 🔶 Partial | `perception/sources/ebpf_source.rs` | Mock /proc fallback works; real eBPF ring buffer reading not implemented (`ebpf_source.rs:217`) |
| Network monitoring | 🔶 Partial | `perception/sources/ebpf_source.rs` | Passive /proc/net/dev reading; no packet-level monitoring |
| Perception-to-Engine feed | ✅ Implemented | `main.rs:146-171`, `engine.rs:120-167` | PerceptionBridge → injection_tx → engine.drain_perceptions() wired before each turn |

---

## 目录

- [1. 概述](#1-概述)
- [2. 当前设计](#2-当前设计)
  - [2.1 感知源](#21-感知源)
  - [2.2 感知事件模型](#22-感知事件模型)
  - [2.3 事件处理流水线](#23-事件处理流水线)
  - [2.4 感知到认知连接](#24-感知到认知连接)
  - [2.5 eBPF 程序管理](#25-ebpf-程序管理)
- [3. 已识别缺陷](#3-已识别缺陷)
  - [3.2 P1: 感知→认知背压机制缺失](#32-p1-感知认知背压机制缺失)
- [4. 改进设计](#4-改进设计)
  - [4.1 EventAggregator 概览](#41-eventaggregator-概览)
  - [4.2 内容哈希去重（轮询源专用）](#42-内容哈希去重轮询源专用)
  - [4.3 时间窗口去重](#43-时间窗口去重)
  - [4.4 批量折叠](#44-批量折叠)
  - [4.5 优先级提升（带指数衰减）](#45-优先级提升带指数衰减)
  - [4.6 通道版本管理（LangGraph 模式）](#46-通道版本管理langgraph-模式)
  - [4.7 Per-Source 可配置阈值](#47-per-source-可配置阈值)
  - [4.8 临界事件 At-Least-Once 交付](#48-临界事件-at-least-once-交付)
  - [4.10 有界事件队列与背压控制](#410-有界事件队列与背压控制)
- [5. 实现要点](#5-实现要点)
- [6. 参考来源](#6-参考来源)

---

## 1. 概述

感知层是 OS-Agent 与操作系统内核和系统服务之间的桥梁。它负责将内核级事件（eBPF tracepoint/kprobe）、用户态信号（/proc、/sys、inotify、journald、udev、D-Bus）以及环境感知（屏幕、音频、摄像头）统一为结构化的 `PerceptionEvent`，经过聚合、过滤和分发后注入认知引擎和记忆系统。

设计原则：
- **渐进式**：Phase 1 纯用户态（eBPF + D-Bus + FUSE + /proc），后续逐步引入内核模块加速
- **类型安全**：所有事件使用 Rust 枚举建模，编译期保证完备性
- **可观测**：每个事件携带来源、优先级和上下文元数据，全链路可追溯

---

## 2. 当前设计

### 2.1 感知源

感知引擎从三个维度采集系统信息：

**内核态感知（eBPF）**

| 领域 | 探测点 | 数据 |
|------|--------|------|
| 文件 | `tracepoint/syscalls/sys_enter_openat`, `sys_enter_write`, `kprobe/vfs_read`, `kprobe/vfs_write` | pid、path、flags |
| 进程 | `tracepoint/sched/sched_process_exec`, `sched_process_exit`, `sched_switch` | pid、comm、args |
| 网络 | `kprobe/tcp_connect`, `tcp_close`, `tracepoint/net/net_dev_queue` | src、dst、port |

数据通过 `bpf_ringbuf` 传输到用户态。

**用户态感知**

| 来源 | 采集方式 | 数据 |
|------|----------|------|
| `/proc` | 轮询 | 进程状态、CPU、内存 |
| `/sys` | 轮询 | 硬件状态、设备信息 |
| journald | 日志流 | 系统日志 |
| inotify | 事件驱动 | 文件系统变化 |
| udev | 事件驱动 | 设备热插拔 |
| D-Bus | 信号订阅 | 系统服务状态 |

**环境感知**

| 来源 | 工具 | 数据 |
|------|------|------|
| 屏幕 | screenshot + OCR | 当前 UI 内容 |
| 音频 | whisper.cpp | 语音识别 |
| 摄像头 | OpenCV | 图像识别 |
| 传感器 | GPIO/I2C（嵌入式） | 温度/湿度/光照 |

### 2.2 感知事件模型

**PerceptionEvent** — 感知事件统一模型，包含 id, timestamp, source (EventSource), category (EventCategory), priority (Low/Normal/High/Critical), data (EventData), context。
- 代码位置: `perception/event.rs`

**EventData 变体：** FileOpen, ProcessExec, TcpConnect, SystemLoad, DeviceEvent, LogMessage

### 2.3 事件处理流水线

```
eBPF ring buf ──┐
/proc poll ─────┼──▶ EventAggregator ──▶ EventFilter
inotify ────────┤      (合并/去重)       (规则过滤)
journald ───────┘           │                │
                            ▼                ▼
                  ┌─────────────────────────────┐
                  │  事件分发器 (EventDispatcher) │
                  │                             │
                  │  -> Core Memory 更新         │
                  │  -> 触发 Agent 推理          │
                  │  -> 写入 Recall Memory       │
                  │  -> 触发告警/通知            │
                  └─────────────────────────────┘
```

### 2.4 感知到认知连接

**感知→认知事件路由：**
- Critical 优先级：立即触发 Agent 推理
- High 优先级：加入待处理队列，下一个推理周期处理
- Normal/Low 优先级：存入 Recall Memory，按需检索

Core Memory 中的 `system_state` block 由感知引擎自动更新，包含 CPU、内存、焦点、网络、时间等系统状态信息。

### 2.5 eBPF 程序管理

| 阶段 | 能力 |
|------|------|
| Phase 1（现有） | 固定 eBPF 程序集合，BPF CO-RE 编译，agentd 启动时加载、退出时卸载 |
| Phase 2（扩展） | 动态加载/卸载，Agent 可请求新感知点，eBPF map 共享，tail call 链式处理 |

---

## 3. 已识别缺陷

### P2: 感知事件聚合策略

**问题：** 高频场景下（如批量文件操作触发大量 inotify 事件），Agent 会被事件淹没。当前 EventAggregator 的"合并/去重"仅有占位描述，缺少具体的聚合策略实现。

**影响：**
- 单次 `git checkout` 或 `make -j` 可产生数百个 inotify 事件
- 大量低价值事件挤占上下文窗口，降低 Agent 对关键事件的响应速度
- Core Memory 的 system_state block 被高频更新覆盖

**需要的策略：**
1. **时间窗口去重** — 同一文件 500ms 内的多次修改合并为一条事件
2. **批量折叠** — 连续同类事件折叠为摘要事件
3. **优先级提升** — 短时间内大量同类事件自动提升优先级

### 3.2 P1: 感知→认知背压机制缺失

**问题：** EventAggregator 的六种聚合策略全部作用于事件的**生产侧**，没有来自消费侧（认知引擎）的背压反馈。事件传递没有定义队列容量上限。

**影响：**

| 缺陷 | 描述 | 后果 |
|------|------|------|
| 无界事件队列 | 去重后仍有大量事件进入无容量限制的队列 | 持续高负载下内存泄漏 |
| 无每轮事件上限 | ReAct 每个 turn 可能注入 200+ 事件到 LLM 上下文 | 推理延迟递增 |
| 优先级提升累积 | High 事件堆积优先投递，Low 事件饿死 | Agent 对系统状态认知失真 |
| 临界事件队列无界 | CriticalEventQueue `pending` 无容量上限 | OOM 风险 |

**数据流图：**

```
eBPF ring buf ──┐
/proc poll ─────┼──▶ EventAggregator ──▶ EventFilter ──▶ EventDispatcher ──▶ CognitiveEngine
inotify ────────┤      (合并/去重)       (规则过滤)       (事件分发)          (推理循环)
journald ───────┘
                                ↑                              ↑
                          6 种聚合策略                     无容量限制
                          (生产侧)                       (消费侧)
```

---

## 4. 改进设计

### 4.1 EventAggregator 概览

`EventAggregator` 位于事件处理流水线的入口，内部维护六个协作策略：

```
原始事件 ──▶ ┌─────────────────────────────────────────────────────────┐
             │                    EventAggregator                       │
             │                                                          │
             │  ┌──────────┐  ┌──────────┐  ┌────────┐  ┌───────────┐ │
             │  │ Content  │  │ Dedup    │  │ Batch  │  │ Boost     │ │
             │  │ Hash     │  │ Window   │  │ Folder │  │ Checker   │ │
             │  │ Dedup    │  │ (per-src)│  │ (同类) │  │ (指数衰减)│ │
             │  └────┬─────┘  └────┬─────┘  └───┬────┘  └─────┬─────┘ │
             │       └────────────┼─────────────┼─────────────┘       │
             │                    ▼                                     │
             │         ┌──────────────────┐                            │
             │         │ Versioned Channel│  (LangGraph 模式)          │
             │         │ change detection │                            │
             │         └────────┬─────────┘                            │
             │                  ▼                                       │
             │    ┌──────────────────────────┐                         │
             │    │ Critical Event Pending Q │  (at-least-once)        │
             │    │ consume() on ACK         │                         │
             │    └──────────┬───────────────┘                         │
             │               ▼                                          │
             │      聚合后事件                                           │
             └──────────────────────────────────────────────────────────┘
```

设计借鉴：
- **Hermes `ToolCallGuardrailConfig`** — 分层阈值（warn/block）+ 签名哈希去重 + 成功时清除计数器
- **LangGraph `BaseChannel`** — `update()` 返回 bool 表示状态是否变化；版本号比较决定是否触发下游
- **LangGraph `LastValueAfterFinish`** — `consume()` 语义：读取后清除，防止重复处理

### 4.2 内容哈希去重（轮询源专用）

对于 `/proc`、`/sys` 等轮询源，时间窗口去重不够——如果系统状态稳定，每次轮询都产生不同的时间戳但数据内容完全相同。借鉴 Hermes 的 `_result_hash` 模式，对事件数据负载计算内容哈希，哈希匹配时直接抑制事件。

适用范围：`EventSource::Proc` 和 `EventSource::Sys`。事件驱动源（inotify、udev、D-Bus）不适用。

核心类型：`ContentHashDedup` 维护 `source_key -> 上次内容哈希` 映射，`should_emit()` 返回内容是否变化。

### 4.3 时间窗口去重

同一事件签名（source + category + 关键字段）在 `window_duration` 内只保留第一条，后续到达的事件更新时间戳和计数器，窗口到期后输出一条合并事件。

改进点：
- **per-source 配置**：不同事件源有不同的去重窗口（eBPF 100ms，D-Bus 2s）
- **有界 LRU 淘汰**：`dedup_map` 设上限（默认 4096 条目），满时按 `last_seen` 最早的淘汰

### 4.4 批量折叠

同一分类的事件在 `batch_interval` 内累积，超过 `batch_threshold` 条后折叠为一条摘要事件（如 "20 个文件写入 in /home/user/project/src/"）。

改进点：
- **目录感知分组**：文件事件按公共父目录分组后再折叠
- **per-source 阈值**：eBPF 来源的 batch_threshold 更高（50），D-Bus 来源更低（5）

### 4.5 优先级提升（带指数衰减）

如果同一分类在 `boost_window` 内产生的事件数超过 `boost_threshold`，自动将后续事件的优先级提升一级。

改进点：窗口过期时不硬重置为 0，而是将计数器右移（减半），防止在持续突发期间优先级在 boosted/normal 之间反复振荡。衰减周期为 `boost_window`，最大衰减 8 次。

### 4.6 通道版本管理（LangGraph 模式）

借鉴 LangGraph 的 `get_new_channel_versions()`，每个事件分类维护单调递增的版本号。下游消费者追踪自己最后处理的版本。分发器只在版本前进时投递事件。

`VersionedChannel` 核心操作：
- `update(category)` — 版本号+1
- `has_new_version(consumer, category)` — 消费者自上次读取以来是否有新版本
- `consume(consumer, category)` — 推进消费者游标

集成点：`EventDispatcher` 在投递前调用 `has_new_version()`，投递成功后调用 `consume()`。

### 4.7 Per-Source 可配置阈值

借鉴 Hermes 的 `ToolCallGuardrailConfig`，每种事件源使用独立的阈值配置。

`SourceProfile` 包含：dedup_window, batch_threshold, batch_interval, boost_threshold, content_hash_dedup

推荐默认值：

| 来源 | dedup_window | batch_threshold | batch_interval | boost_threshold | content_hash |
|------|-------------|-----------------|----------------|-----------------|-------------|
| eBPF | 100ms | 50 | 1s | 50 | false |
| /proc, /sys | 2s | 5 | 5s | 10 | **true** |
| inotify | 500ms | 10 | 2s | 20 | false |
| D-Bus | 2s | 5 | 3s | 10 | false |
| udev | 1s | 5 | 2s | 10 | false |
| journald | 1s | 20 | 3s | 30 | false |

### 4.8 临界事件 At-Least-Once 交付

借鉴 LangGraph `LastValueAfterFinish` 的 `consume()` 语义。`Priority::Critical` 事件在分发前写入持久化待处理队列。认知引擎必须通过 `acknowledge(event_id)` 确认消费。未确认的事件在超时后重试投递。

`CriticalEventQueue` 核心操作：
- `enqueue(event, now)` — 入队
- `acknowledge(event_id)` — 确认消费
- `reap_timeouts(now)` — 返回需要重试投递的事件列表；超过最大重试次数的事件降级为 High 优先级

### 4.10 有界事件队列与背压控制

为解决 §3.2 描述的背压缺失问题，在 EventAggregator 和 CognitiveEngine 之间引入有界通道。

#### 4.10.1 有界通道替代无界队列

`BoundedEventChannel` 使用 `tokio::sync::mpsc::channel(capacity)`，默认容量 256 个事件。溢出处理策略：
- **Critical** — 阻塞等待空间（永不丢弃）
- **High** — 丢弃并记录 metrics
- **Normal/Low** — 直接丢弃

`OverflowMetrics` 记录总丢弃数和按优先级分类的丢弃数。

#### 4.10.2 优先级感知队列

当通道满时，比较新事件和队列中最低优先级事件，而非简单丢弃新事件。`PriorityAwareQueue` 按优先级分桶（Low/Normal/High/Critical），满时高优先级事件可挤掉低优先级事件。

#### 4.10.3 每轮事件上限（Max Events Per Turn）

`CognitiveEngineConfig` 定义：
- `max_events_per_turn` — 默认 10
- `event_token_budget` — 默认 2000 tokens
- `drop_low_priority_on_overflow` — 默认 true

在 ReAct 循环的事件处理阶段：批量接收 → 按优先级排序 → 截断到上限 → 注入上下文（受 token 预算限制）。

#### 4.10.4 临界事件队列容量限制

`CriticalEventQueue` 增加 `max_capacity`（默认 64）和 `overflow_strategy`（DropOldest / BlockProducer / SpillToDisk）。

#### 4.10.5 监控和告警

`BackpressureMetrics` 追踪通道深度、溢出计数、每轮事件数、丢弃率、最老未处理事件年龄。告警规则：通道深度 > 80%、连续丢弃、最老事件 > 30s、drop_rate > 10%。

---

## 5. 实现要点

| 要点 | 说明 |
|------|------|
| **非阻塞** | `ingest()` 必须是 async 且非阻塞的，避免拖慢 eBPF ring buffer 消费 |
| **定时器** | `flush_expired_batches()` 和 `flush_critical_retries()` 需要由独立的 tokio task 周期调用 |
| **背压** | `output_tx` 通道满时应丢弃低优先级事件并记录 metrics |
| **可配置** | 所有时间窗口和阈值通过 `/etc/agent/agent.toml` 配置，支持 per-source profile 覆盖 |
| **可观测** | 聚合器导出计数器：events_ingested、events_deduped、events_batched、events_boosted、critical_retries |
| **BatchSummary 扩展** | `EventData` 枚举需新增 `BatchSummary { count, summary, sample_ids }` 变体 |
| **EventSource::Aggregator** | `EventSource` 枚举需新增 `Aggregator` 变体 |
| **EventData::hash_data_fields** | `EventData` 需实现 `hash_data_fields()` 方法 |
| **dedup_map 有界** | 去重表上限 `dedup_map_capacity`（默认 4096），LRU 淘汰时输出合并事件 |
| **临界事件持久化** | `CriticalEventQueue` 建议使用 WAL 或 SQLite 持久化 |
| **通道版本与认知引擎集成** | `EventDispatcher` 投递前调用 `has_new_version()`，投递后调用 `consume()` |

---

## 6. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| **eBPF** (`libbpf`, BPF CO-RE) | 内核态感知探针、ring buffer 传输、CO-RE 跨内核兼容 |
| **Hermes** | 签名哈希去重（`ToolCallSignature`）、`_result_hash` 内容哈希抑制、分层阈值配置、成功时清除计数器 |
| **LangGraph** | 通道版本调度——事件版本变化才触发处理；`update()` 返回 bool；`LastValueAfterFinish` 的 `consume()` 语义；`get_new_channel_versions()` 版本号比较 |
| **原始设计文档** | `docs/plans/2026-06-06-argos-design.md` §8 (historical reference, file has been removed) — 感知源分类、事件模型、事件处理流水线、eBPF 管理阶段 |

---

*模块版本: 0.1.0*
*源文档: `docs/plans/2026-06-06-argos-design.md` §8 (historical reference, file has been removed)*

---

## Implementation Summary

**Code locations:**
- `perception/event.rs` — PerceptionEvent type definitions, EventData variants
- `perception/manager.rs` — PerceptionManager (event routing and lifecycle)
- `perception/aggregator.rs` — EventAggregator (event deduplication and batching)
- `perception/sources/proc_source.rs` — ProcSource (/proc filesystem monitoring)
- `perception/sources/inotify_source.rs` — InotifySource (polling-based, not real inotify)
- `perception/sources/journald_source.rs` — JournaldSource (systemd journal reader)

**Key types/traits implemented:**
- `PerceptionEvent` — unified perception event model with id, timestamp, source, category, priority, data, context
- `PerceptionManager` — event routing and lifecycle management
- `EventAggregator` — event deduplication and batching pipeline
- `EventSource` — event source enumeration (Proc, Inotify, Journald, etc.)
- `EventData` — event data variants (FileOpen, ProcessExec, TcpConnect, SystemLoad, DeviceEvent, LogMessage)

**Planned (not started):**
- Real eBPF ring buffer reading (mock /proc fallback works, but no eBPF programs loaded)
- Backpressure control — bounded event queues, priority-aware queuing, per-turn event limits
- Content hash dedup for polling sources
- Versioned channel management
- Hardware sensors (GPU/disk SMART/temperature/ECC/battery)

**Test coverage:** No dedicated test suite documented for the perception layer.

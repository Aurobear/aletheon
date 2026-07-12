# Aletheon Current Architecture & Coupling Analysis

> 更新日期: 2026-07-12
> 基于 dev 分支 Clock 统一后（Phase 1 基础层改造进行中）
> 本文档反映真实代码状态，非设计建议

---

## 1. Crate 依赖图

```
                                     fabric (trait + 类型, leaf)
                                        │
                                     kernel (基础实现层)
                                        │
           ┌──────┬──────┬──────┬───────┼───────┬──────┬──────┐
           ▼      ▼      ▼      ▼       ▼       ▼      ▼      ▼
        cognit corpus dasein agora metacog mnemosyne executive
                                                         │
                                                     interact
                                                         │
                                                        bin
```

### 关键发现

| 属性 | 值 |
|------|-----|
| 循环依赖 | **无** — DAG 结构，fabric 是唯一叶子 |
| kernel 消费者 | **8 crates** — 从 1 个扩展到 8 个，成为真正的共享基础层（Phase 1 进行中） |
| executive 依赖数 | **7 workspace crates** + fabric — 仍然是集成 God Object |
| 时间统一 | **Clock trait 覆盖 87%** — 238 处直接时间调用降到 30 处（Phase 1 主体完成） |
| dasein 跨服务依赖 | **无** — 仅依赖 fabric + kernel，已完成解耦 ✓ |
| bin side-channel | **已消除** — 仅依赖 executive + interact ✓ |

### 变更说明 (2026-07-12 Clock 统一)

**发生的变化：** 7 个领域 crate（cognit、corpus、dasein、agora、metacog、mnemosyne、interact）从仅依赖 `fabric` 变为依赖 `fabric + kernel`。kernel 的 `Clock` trait 实现（`SystemClock`/`TestClock`）和 `Timer` 工具类被所有 crate 共享。

**未变化的部分：** kernel 的 ProcessTable、OperationTable、SupervisorTree、AdmissionController 等安全层/进程层服务仍然仅被 executive 使用。

**剩余 30 处直接时间调用** 分布在需要改 struct 字段类型的深层代码（drivers、watchdog、safe_mode），属于 Phase 1 的 follow-up。

---

## 2. 耦合分析

### 2.1 重度耦合: `executive` — 集成 God Object

`CoreSystems` 位于 `crates/executive/src/core/core_systems.rs`，已将字段组织为四个子结构体组：

```
CoreSystems
├── ports: ServicePorts
├── runtime: Arc<Mutex<AletheonExecutive>>
├── self_field: Arc<Mutex<SelfField>>
├── reflector: Reflector
├── pipeline: Arc<MorphogenesisPipeline>
├── debug_handler: Arc<DebugHandler>
├── debug_perf: Arc<PerfCounter>
├── cancel_token: CancellationToken
├── memory: MemoryGroup (6 fields)
│   ├── episodic
│   ├── recall
│   ├── core
│   ├── fact_store
│   ├── auto_memory
│   └── objective_store
├── security: SecurityGroup (5 fields)
│   ├── tool_runner
│   ├── storm_breaker
│   ├── approval_rx
│   ├── pending_approvals
│   └── session_approvals
├── corpus: CorpusGroup (5 fields)
│   ├── tools
│   ├── skill_loader
│   ├── skill_router
│   ├── hook_registry
│   └── hooks_config
└── session: SessionGroup (7 fields)
    ├── default_session_id
    ├── session_created_at
    ├── cached_prefix
    ├── memory_queue
    ├── context_window
    ├── config_prompt
    └── data_dir
```

**改善**: commit 10cd739 已将平铺字段重组为 Memory/Security/Corpus/Session 四个子结构体组。`DaemonTurnOrchestrator` 仍然镜像了 `CoreSystems` 的字段 + kernel primitives，构成两层 god object。

**改善方向**: 继续将具体字段迁移为 `Arc<dyn TraitOps>`。

### 2.2 低度耦合: `dasein` — 跨服务依赖已解耦 ✓

```
dasein ──► fabric           ✅ 合理 (protocol/types)
dasein ──► kernel           ✅ 新增 (Clock 统一, Phase 1)
dasein ──► corpus (具体类型)  ✅ 已移除
dasein ──► mnemosyne (具体类型) ✅ 已移除
```

`_Final(2).md` §12 的要求已满足: Dasein 不再直接依赖 Corpus 与 Mnemosyne 的具体实现。新增 `kernel` 依赖用于 `Arc<dyn Clock>`，替代直接 `chrono::Utc::now()` / `std::time::Instant::now()` 调用。

### 2.3 低度耦合: `bin` — Side-channel 已消除 ✓

```
bin/Cargo.toml 直接依赖:
  executive, interact
```

bin 以前依赖 6 个 crate (executive, kernel, interact, fabric, cognit, corpus)，现在仅依赖 `executive` + `interact`。`kernel`、`cognit`、`corpus` 的直接依赖已移除。bin 只能通过 `executive` 的公开 API 访问下游类型。kernel 的 Clock 通过 interact crate 间接获取。

`run_exec()` 函数使用 `ExecSessionBuilder` (shared factory) → `TurnService` 路径执行，不再绕过 executive。

### 2.4 低度耦合: `executive` 内部 — impl/ vs service/ 分裂

| 目录 | 文件数 | 定位 |
|------|--------|------|
| `service/` | ~12+ | "新"代码 — DaemonTurnOrchestrator, TurnService, ExecSessionBuilder |
| `impl/` | 80+ | "旧"代码 — daemon handlers, agents, automation, plugins |
| `core/` | ~28 | bootstrap + CoreSystems + session gateway |

`chat.rs` (482 行 zombie) **已删除** ✓。`bridge/mod.rs` (空模块) **已删除** ✓。逻辑已完全迁移到 `service/daemon_turn/`。

### 2.5 新增: `kernel` 基础层 — Phase 1 Clock 统一

kernel 从仅被 1 个 crate（executive）依赖扩展为被 **8 个 crate** 依赖。这是 Phase 1（Clock 统一）的成果。

**已完成 (87% 覆盖):**

| Crate | 时间调用 (替换前) | 替换后剩余 | Tests |
|-------|------------------|-----------|-------|
| agora | 1 | 0 | 53 pass |
| metacog | 10 | 0 | 17 pass |
| cognit | 31 | 0 | 302 pass |
| interact | 32 | 0 | 109 pass |
| corpus | 49 | 15 | 368 pass |
| mnemosyne | 57 | 4 | 173 pass |
| dasein | 58 | 18 | 291 pass |
| executive | 75 | ~45 | 1300+ pass |
| **合计** | **313** | **~82** | **全 pass** |

> 注：上表包含所有 crate（含 executive）。之前审计的 238 处是 7 个非-executive 领域 crate 的数字。剩余调用主要在需改 struct 字段类型（`Instant` → `MonoTime`）的深层代码。

**新增基础设施:**
- `fabric::wall_to_datetime(WallTime) → DateTime<Utc>` — chrono 类型转换桥
- `kernel::chronos::Timer::sleep(clock, dur)` — 基于 Clock 的 async sleep
- `kernel::chronos::Timer::timeout(clock, dur, fut)` — 基于 Clock 的 async timeout

**kernel 服务分层（目标架构，仅 Phase 1 实施）:**

```
基础层 (Foundation) — Phase 1 已实施:
  Clock   :: 统一时间源 (wall_now / mono_now) → 8 消费者 ✓
  Timer   :: async sleep / timeout → 8 消费者 ✓

进程层 (Process) — 仅 executive:
  ProcessTable   :: 进程生命周期 → 仅 executive
  OperationTable :: 操作树取消传播 → 仅 executive
  SupervisorTree :: OTP 监督树 → 仅 executive

安全层 (Security) — 仅 executive:
  AdmissionController :: 许可门控
  BudgetController    :: 配额管理
  ResourceLease       :: 资源租约
```

**未使用的 kernel 服务（仍然仅 executive）:** ProcessTable、OperationTable、SupervisorTree、AdmissionController、Budget、Lease。这些服务的跨 crate 集成需要独立论证（见设计文档 §7 后续方向）。

---

## 3. 当前架构总览

```text
┌─────────────────────────────────────────────────────────┐
│                    Aletheon Instance                     │
│                                                         │
│  ┌──────────────────────────────────────────────────┐   │
│  │              interact (TUI/CLI)                   │   │
│  └────────────────────┬─────────────────────────────┘   │
│                       │                                  │
│  ┌────────────────────▼─────────────────────────────┐   │
│  │                  executive                        │   │
│  │                                                  │   │
│  │  host/          core/           service/         │   │
│  │  ├ systemd.rs   ├ core_systems  ├ daemon_turn/   │   │
│  │  └ container.rs ├ runtime_core  │  ├ execute.rs  │   │
│  │                  ├ config/      │  ├ orchestrator│   │
│  │                  └ session_     │  ├ inject.rs   │   │
│  │                    gateway/     │  ├ lifecycle.rs│   │
│  │                  ├ sub_agent.rs │  └ post_phases │   │
│  │                  │              │                 │   │
│  │  impl/ (旧代码 80+ files)       tools/           │   │
│  │  ├ daemon/                     └ self_observe   │   │
│  │  ├ agents/                                      │   │
│  │  ├ automation/                                  │   │
│  │  ├ orchestration/                               │   │
│  │  └ plugins/                                     │   │
│  └────┬──────┬──────┬──────┬──────┬──────┬─────────┘   │
│       │      │      │      │      │      │              │
│  ┌────▼──┐ ┌─▼───┐ ┌─▼───┐ ┌─▼───┐ ┌─▼───┐ ┌─▼──────┐  │
│  │kernel │ │cognit│ │corpus│ │mnem │ │dasein│ │metacog │  │
│  │       │ │      │ │      │ │     │ │      │ │+ agora │  │
│  │proc   │ │config│ │tools │ │impl/│ │self  │ │        │  │
│  │oper   │ │harness│ │sec  │ │back │ │field │ │        │  │
│  │chronos│ │impl/ │ │drive│ │ends │ │      │ │        │  │
│  │space  │ │      │ │hook  │ │ops  │ │      │ │        │  │
│  │admiss │ │      │ │skill │ │     │ │      │ │        │  │
│  │supv   │ │      │ │      │ │     │ │      │ │        │  │
│  └───┬───┘ └──┬──┘ └──┬──┘ └──┬──┘ └──┬──┘ └───┬────┘  │
│      │        │       │       │       │        │        │
│  ┌───▼────────▼───────▼───────▼───────▼────────▼─────┐  │
│  │                    fabric                          │  │
│  │  contract/ dasein/ events/ include/ ipc/           │  │
│  │  kernel/ policy/ primitives/ types/                │  │
│  │  (CommunicationBus 统一事件总线)                     │  │
│  └───────────────────────────────────────────────────┘  │
│                                                         │
│  ┌──────────────────────────────────────────────────┐   │
│  │           External Execution Domains             │   │
│  │  Sandbox  │  Browser  │  Robot  │  GPU Worker   │   │
│  └──────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
```

---

## 4. P0 问题清单 (来自 `_Final(2).md` §21)

| # | P0 项 | 当前状态 | 具体 Gap |
|---|-------|---------|----------|
| 1 | **唯一 Turn Execution Path** | ✅ 改善 | daemon 路径使用 `DaemonTurnOrchestrator`→`TurnService`。`bin/main.rs run_exec()` 已改为使用 `ExecSessionBuilder` (shared factory)→`TurnService`。`chat.rs` zombie 已删除。三条路径已收敛为两条通过 `TurnService` 的路径（daemon + exec），exec 路径使用 `NoopTurnEventSink`（无事件流）。 |
| 2 | **拆 `handle_chat`** | ✅ 完成 | `chat.rs` (482 行 zombie) 已删除。`bridge/mod.rs` (空模块) 已删除。业务逻辑全部在 `DaemonTurnOrchestrator` 中。 |
| 3 | **修正 `SandboxFirst` fail-closed** | ✅ 完成 | `ProductionAdmissionController` 的 `sandbox_available` 字段默认为 `false`（通过 `with_sandbox_available(false)` 设置）。当 sandbox required 但 `sandbox_available == false` 时，返回 `AdmissionError::SandboxRequiredUnavailable` 硬错误，而非 warn 后继续。 |
| 4 | **统一 AgentId/OperationId/Error** | ✅ 完成 | `SubAgentSpawner` (`core/sub_agent.rs:118`) 有 `spawn_tracked`/`cancel`/`wait` 方法。`AgentKernel` + `AgentSupervisor` 存在。`orchestration::Agent` 标记为 `DEPRECATED`。Agent 概念已统一到 `SubAgentSpawner` + `AgentProcess` 体系。 |
| 5 | **SubAgent 真实执行与 wait/cancel** | ✅ 完成 | `SubAgentSpawner` 有 ID/状态追踪/CancellationToken。`cancel(id)`→bool, `wait(id, timeout)`→`SubAgentHandle`。 |
| 6 | **Dasein lived time vs Kernel Chronos 边界** | 🔄 进行中 | Phase 1 Clock 统一已实施：dasein 的 ChronosLayer 现在接受 `Arc<dyn Clock>`，34 处 `Utc::now()` 已替换为 `fabric::wall_to_datetime(clock.wall_now())`。剩余 18 处（watchdog、safe_mode、sorge、aggregator、FUSE sources）需 follow-up PR 处理 `Instant` → `MonoTime` 类型变更。 |
| 7 | **Agora version/proposal/commit** | ✅ 完成 | 完整 CAS: `propose(base_version)` 比较 `self.version`，不匹配返回 `VersionConflict`。`commit` 递增版本。`propose_full` 接受外部预构建 `AgoraProposal`。`AgoraCommit` 持久化支持 (`persistence.rs`)。 |

---

## 5. 僵尸代码清单

| 文件/目录 | 状态 | 行动 |
|-----------|------|------|
| `executive/src/impl/daemon/handler/chat.rs` | **已删除** ✓ | — |
| `executive/src/impl/daemon/handler/mod.rs` | `#[allow(dead_code)]` 已移除 | — |
| `executive/src/bridge/mod.rs` | **已删除** ✓ | — |
| `corpus/src/testing/` | **已删除** ✓ | — |
| `executive/src/impl/daemon/handler/tool_executor.rs` | `#![allow(dead_code)]` 仍存在 | 待清理 |

---

## 6. 依赖清理建议优先级

| Priority | 行动 | 影响 | 状态 |
|----------|------|------|------|
| **P0** | 删除 `chat.rs` zombie + `bridge/` 空模块 | 消除 dead code | ✅ 已完成 |
| **P0** | bin `run_exec()` 改为调用 `ExecSessionBuilder`→`TurnService` | 唯一执行路径 | ✅ 已完成 |
| **P1** | dasein 去掉 corpus/mnemosyne 具体依赖，改为 trait Port | 降低跨服务耦合 | ✅ 已完成 |
| **P1** | bin Cargo.toml 移除 kernel/cognit/corpus 直接依赖 | 消除 side-channel | ✅ 已完成 |
| **P1** | `execute.rs` 拆出 event 转换函数 (230 行) | 降低单文件复杂度 | ⚠️ 待处理 |
| **P1** | EventBus 双轨合并: 旧 EventBus trait 删除，`event_bridge.rs` 删除，`legacy_bridge.rs` 删除，统一为 `CommunicationBus` | 消除双轨 | ✅ 已完成 |
| **P2** | CoreSystems 直接字段 → `Arc<dyn TraitOps>` | 解耦 god object | ⚠️ 待处理 |
| **P2** | `impl/daemon/server.rs` DaemonHost 搬到 `host/` | 统一 host 抽象 | ⚠️ 待处理 |
| **P2** | `tool_executor.rs` 清理 `#![allow(dead_code)]` | 消除未使用代码提示 | ⚠️ 待处理 |
| **P3** | `impl/` 中旧代码归档或删除 | 消除 catch-all 目录 | ⚠️ 待处理 |

---

## 7. Crate 依赖矩阵

```
           fabric kernel cognit corpus mnemosyne dasein agora metacog executive interact
fabric       -      -      -      -      -       -     -     -       -         -
kernel       ✓      -      -      -      -       -     -     -       -         -
cognit       ✓      ✓      -      -      -       -     -     -       -         -
corpus       ✓      ✓      -      -      -       -     -     -       -         -
mnemosyne    ✓      ✓      -      -      -       -     -     -       -         -
dasein       ✓      ✓      -      -      -       -     -     -       -         -
agora        ✓      ✓      -      -      -       -     -     -       -         -
metacog      ✓      ✓      -      -      -       -     -     -       -         -
executive    ✓      ✓      ✓      ✓      ✓       ✓     ✓     ✓       -         -
interact     ✓      ✓      -      -      -       -     -     -       ✓         -
bin          -      -      -      -      -       -     -     -       ✓         ✓
```

✓ = 依赖, - = 无依赖

**关键变化 (相比 2026-07-12 初期):**

| Crate | 之前 | 现在 | 状态 |
|-------|------|------|------|
| dasein | fabric + corpus + mnemosyne | fabric + kernel | ✅ 解耦 + Clock |
| bin | fabric + kernel + cognit + corpus + executive + interact | executive + interact | ✅ side-channel 消除 |
| kernel 消费者 | 1 (仅 executive) | **8 crates** | 🔄 Phase 1 Clock 统一 |
| cognit | 仅 fabric | fabric + kernel | ✅ Clock 统一 |
| corpus | 仅 fabric | fabric + kernel | ✅ Clock 统一 |
| mnemosyne | 仅 fabric | fabric + kernel | ✅ Clock 统一 |
| agora | 仅 fabric | fabric + kernel | ✅ Clock 统一 |
| metacog | 仅 fabric | fabric + kernel | ✅ Clock 统一 |
| interact | fabric + corpus | fabric + corpus + kernel | ✅ Clock 统一 |

---

## 8. Phase 1 Clock 统一 — 实施状态

> 设计文档: `docs/plans/2026-07-12-kernel-foundation-layer-design.md`
> 实施计划: `docs/plans/2026-07-12-clock-unification-plan.md`

### 8.1 目标

将 kernel 的 `Clock` trait（`wall_now` / `mono_now`）作为全仓库的**唯一时间源**，替代各处直接调用的 `std::time::Instant::now()`、`chrono::Utc::now()`、`SystemTime::now()`、`tokio::time::sleep`、`tokio::time::timeout`。

收益：确定性时间测试（`TestClock::advance()` + `Timer::sleep`）、消除 flaky tests、统一时间语义。

### 8.2 已完成 (87% 覆盖)

**基础设施 (PR-0):**
- `fabric::wall_to_datetime(WallTime) → DateTime<Utc>` — chrono 类型转换桥
- `kernel::chronos::Timer::sleep(clock, dur)` — 基于 Clock 的 async sleep
- `kernel::chronos::Timer::timeout(clock, dur, fut)` — 基于 Clock 的 async timeout

**逐 crate 注入 (PR-1~8):**

| PR | Crate | 注入方式 | 替换前 | 替换后 | Tests |
|----|-------|---------|--------|--------|-------|
| 1 | agora | `Workspace::new(session_id, clock)` | 1 | 0 | 53 pass |
| 2 | metacog | `DefaultMetaRuntime` → pipeline → leaf | 10 | 0 | 17 pass |
| 3 | cognit | `CognitCore::new(clock)` → 子模块 | 32 | 0 | 302 pass |
| 4 | interact | `TuiApp` + ACLX context | 41 | 0 | 109 pass |
| 5 | corpus | `ToolContext.clock` + `ToolRegistry` | 60 | 15 | 368 pass |
| 6 | mnemosyne | `MemoryPipeline` → 所有 backend | 57 | 4 | 173 pass |
| 7 | dasein | `SelfField::new(clock)` → 子层 | 63 | 18 | 291 pass |
| 8 | executive | `ServicePorts.clock` → handler/host | 75 | ~45 | 1300+ pass |

### 8.3 剩余工作 (~82 处直接调用)

这些调用分布在需要改 struct 字段类型（`Instant` → `MonoTime`）的深层代码：

| 模块 | 文件 | 原因 |
|------|------|------|
| corpus drivers | `boot.rs`, `clipboard_x11.rs`, `android.rs` | 平台驱动层，需要 struct 重构 |
| corpus sandbox | `executor.rs` (测试计时), `socket_approval.rs` | 测试中使用真 `Instant` 测量并行度 |
| dasein watchdog | `watchdog.rs`, `safe_mode.rs` | `last_beat: Mutex<Instant>` → `Mutex<MonoTime>` |
| dasein event loop | `sorge.rs`, `aggregator.rs` | select! 宏中使用 `tokio::time::sleep` |
| dasein sources | `inotify_source.rs`, `dispatcher.rs` | FUSE 感知层 |
| executive impl/ | `kernel/supervisor.rs`, `agent/process.rs`, 等 | 旧代码层，`Instant` 字段类型变更 |
| mnemosyne | `tools.rs`, `core_memory/mod.rs` | 耗时测量 + 时间戳 |

预估：1-2 个 follow-up PR 可完成全部替换。

### 8.4 kernel 服务使用现状

| 服务 | 消费者 | 备注 |
|------|--------|------|
| `Clock` (SystemClock/TestClock) | 8 crates ✅ | Phase 1 已实施 |
| `Timer` (sleep/timeout) | 8 crates ✅ | 基于 Clock |
| `ProcessTable` | executive only | 进程生命周期管理 |
| `OperationTable` | executive only | 操作树取消传播 |
| `SupervisorTree` | executive only | OTP 监督树 |
| `InMemorySpaceManager` | executive only | 上下文空间隔离 |
| `ProductionAdmissionController` | executive only | 许可门控 |
| `InMemoryBudgetController` | executive only | 配额管理 |
| `InMemoryResourceLeaseManager` | executive only | 资源租约 |

### 8.5 设计文档中的后续方向

Phase 1 完成后，kernel 已作为所有 crate 的依赖。后续方向见设计文档 §7 和本文 §9。

---

## 9. 地址空间与进程 — 架构分析

> 分析日期: 2026-07-12
> 基于 dev 分支 Phase 1 Clock 统一后
> 动机: kernel 不仅提供时间服务，也提供空间管理和进程管理，但三者目前严重割裂

### 9.1 概念模型

Aletheon 的"地址空间"不是传统的 OS 虚拟内存页表，而是**基于 capability 的上下文空间**：每个进程有一个 `SpaceId`，指向 `InMemorySpaceManager` 中的 `ContextSpace`。一个 ContextSpace 绑定了一组"区域"（session、agora workspace、memory view、artifact、world projection）+ 一个私有的 key-value overlay。

类比 Linux：`task_struct.mm` → `mm_struct` → `vm_area_struct[]`。Aletheon 里是 `ProcessRecord.space` → `ContextSpace` → `ContextBinding[]`。

### 9.2 当前使用现状

**SpaceManager 的消费者只有 1 个 crate（executive），而且使用方式不完整：**

| 操作 | 设计 | 实际 |
|------|------|------|
| `fork_space(parent, owner)` | spawn 进程时继承父空间 bindings（写权限降级为只读） | **从未在生产线调用** |
| `attach_region(space, binding)` | 为进程空间绑定 session/agora/artifact 等区域 | 仅 `execute_turn` 使用，且绑定的是临时 `turn_space` 而非 `process.space` |
| `set_overlay(space, key, value)` | 进程私有数据写入 | 仅 `execute_turn` 使用 |
| `release(space)` | 进程退出时清理空间 | **不存在此方法** |

### 9.3 两层"空间"概念割裂

| 维度 | `SpaceId(Uuid)` (kernel) | `AgoraSpaceId(String)` (agora) |
|------|---------------------------|----------------------------------|
| 类型 | UUID 强类型 | 裸 String 包装 |
| 归属 | `ProcessRecord.space` | `Workspace.session_id` |
| 管理器 | `InMemorySpaceManager` (fork/attach/overlay) | `AgoraRegistry` (`HashMap<String, Workspace>`) |
| 绑定方式 | `ContextBinding::Agora(agora_id, version)` | 不知道自己被绑定了 |
| 消费者 | executive turn handler | agora ops |

两者只在 `execute_turn` 中通过 `ContextBinding::Agora(AgoraSpaceId(sess_id), version)` 连接一次。而且连接的不是进程的真实 `process.space`，而是每个 turn 临时创建的 `turn_space`。

### 9.4 agora 的 nil-uuid 问题

agora 的 workspace 使用了 `fabric::ProcessId`，但**每一处都是零值占位符**：

| 位置 | 字段 | 值 |
|------|------|-----|
| `workspace/mod.rs:94` | `AgoraProposal.author` | `ProcessId(uuid::Uuid::nil())` |
| `workspace/mod.rs:232` | `claims` 插入值 | `ProcessId(uuid::Uuid::nil())` |
| `ops/mod.rs:672,697,723,761` | 测试代码 | 全部 nil |
| `persistence/mod.rs:74` | `AgoraCommit.author` | `ProcessId(uuid::Uuid::nil())` |

agora 完全不知道"哪个进程在操作我"。proposal 的 author、commit 的 author、claims 的 owner — 全部是 `Uuid::nil()`。这意味着：
- claims（共享对象锁）无法真正验证"谁持有"
- commit 审计日志无法追溯到真实进程
- CAS 冲突检测只知道"版本变了"但不知道"被谁变了"

### 9.5 进程与空间的结合问题

**ProcessTable::spawn() 创建 SpaceId 但不注册到 SpaceManager：**

`kernel/src/process/table.rs:139` 生成 `SpaceId::new()` 写入 `ProcessRecord.space`，但**从不调用 `InMemorySpaceManager::fork_space()` 或 `attach_region()`**。SpaceManager 只在 lazy 访问时才知道这个 SpaceId 存在。

**每次 daemon turn 泄露一个 Space：**

`executive/src/service/daemon_turn/execute.rs:311` 每 turn 创建临时 `SpaceId::new()`，attach bindings + set overlay 后丢弃。`InMemorySpaceManager` 没有 `release()` 方法，空间条目永久积累。

**正确的模型应该是：**

```
ProcessTable::spawn(spec)
  ├── child_pid = ProcessId::new()
  ├── child_space = space_manager.fork_space(parent_pid.space, child_pid)
  │     // 继承父空间 bindings（写权限降级为只读）
  │     // 子空间初始 overlay 为空
  └── ProcessRecord { pid: child_pid, space: child_space, ... }

agora commit:
  ├── proposal.author = 真实 ProcessId（不是 nil）
  ├── claims.owner = 真实 ProcessId
  └── commit.author = 真实 ProcessId

daemon turn:
  ├── 使用 process.space（不是临时 turn_space）
  ├── turn 结束后保留 overlay（不泄露）
  └── agora commit 时版本写入 ContextBinding::Agora(space, new_version)
```

### 9.6 `SpaceManager` trait 缺失的方法

当前 trait 只有 2 个方法：

```rust
pub trait SpaceManager: Send + Sync {
    async fn fork_space(&self, parent: SpaceId, owner: ProcessId) -> Result<SpaceId>;
    async fn attach_region(&self, space: SpaceId, binding: ContextBinding) -> Result<()>;
}
```

明显缺失（仅在具体类型 `InMemorySpaceManager` 上存在或完全不存在）：

| 方法 | 需要 | 当前状态 |
|------|------|---------|
| `release(space)` | 进程退出时清理空间、释放绑定 | ❌ 不存在 |
| `lookup(space)` | 查询空间状态 | ⚠️ 仅 `InMemorySpaceManager::get_space()` |
| `set_overlay(space, k, v)` | 写入进程私有数据 | ⚠️ 仅 `InMemorySpaceManager::set_overlay()` |
| `get_overlay(space, k)` | 读取进程私有数据 | ❌ 不存在 |
| `list_spaces()` | 调试/监控 | ❌ 不存在 |
| `update_binding(space, binding)` | 更新 agora 版本等 | ❌ 不存在 |

### 9.7 影响评估

| 问题 | 严重度 | 影响 |
|------|--------|------|
| `fork_space` 从未被调用 | 🔴 高 | 进程间无空间继承，每个进程都是孤立空间 |
| `turn_space` 泄露 | 🔴 高 | 每个 turn 永久积累一个 ContextSpace，长期运行 OOM |
| agora nil-uuid | 🟡 中 | claims 无意义，审计不可追踪，但功能可用 |
| `SpaceManager` trait 不完整 | 🟡 中 | 无法基于 trait 做 release/查询/绑定更新 |
| 其他 crate 零感知 | 🟢 低 | 需要空间的 crate（agora）暂时不需要完整 SpaceManager |

### 9.8 修复建议优先级

| 优先级 | 修复 | 影响 |
|--------|------|------|
| **P0** | `SpaceManager` trait 加 `release(space)` 方法 | 修复内存泄露 |
| **P0** | `execute_turn` 复用 `process.space` 而非创建临时 `turn_space` | 修复内存泄露 + 语义正确 |
| **P1** | `ProcessTable::spawn()` 改调用 `space_manager.fork_space()` | 进程间空间继承 |
| **P1** | agora 的 nil-uuid → 接受 `Arc<dyn ProcessManager>` 或传入真实 ProcessId | claims 有意义 |
| **P2** | `SpaceManager` trait 补全 lookup / update_binding / get_overlay | trait 可以作为抽象边界使用 |
| **P2** | `ContextBinding::Agora` 版本号在 commit 时自动更新 | 绑定版本与实际 workspace 版本一致 |

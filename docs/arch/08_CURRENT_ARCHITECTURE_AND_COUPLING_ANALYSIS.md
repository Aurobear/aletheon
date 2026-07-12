# Aletheon Current Architecture & Coupling Analysis

> 生成日期: 2026-07-12
> 基于 dev 分支 commit 1b12a59
> 本文档反映真实代码状态，非设计建议

---

## 1. Crate 依赖图

```
                    ┌──────────┐
                    │    bin   │
                    └────┬─────┘
                         │ (直接依赖 2 crates: executive + interact
                         │  side-channel 已消除)
                         │
              ┌──────────┤
              ▼          ▼
         ┌────────┐ ┌──────────┐
         │interact│ │metacog   │──────► fabric
         └───┬────┘ └──────────┘
             │
             ▼
         ┌──────────────────────────────────┐
         │           executive              │
         │  (依赖全部 7 workspace crates,     │
         │   仍然是集成 God Object)           │
         └──┬───┬───┬───┬───┬───┬───┬──────┘
            │   │   │   │   │   │   │
     ┌──────┘   │   │   │   │   │   └──────┐
     ▼          ▼   ▼   ▼   ▼   ▼          ▼
┌────────┐ ┌──────┐ ┌──────┐ ┌────────┐ ┌──────┐
│ kernel │ │cognit│ │corpus│ │mnemosyne│ │dasein│
└───┬────┘ └──┬───┘ └──┬───┘ └───┬─────┘ └──┬───┘
    │         │        │         │          │
    │         │        │         │          │ (仅 fabric)
    │         │        │         │          │ dasein 已解耦:
    │         │        │         │          │ 不再依赖 corpus
    │         │        │         │          │ 不再依赖 mnemosyne
    ▼         ▼        ▼         ▼          ▼
         ┌──────────────────────┐
         │        fabric        │  (leaf crate)
         │ contract/ dasein/    │
         │ events/  include/    │
         │ ipc/     kernel/     │
         │ policy/  primitives/ │
         │ types/               │
         └──────────────────────┘
```

### 关键发现

| 属性 | 值 |
|------|-----|
| 循环依赖 | **无** — DAG 结构，fabric 是唯一叶子 |
| executive 依赖数 | **7 workspace crates** + fabric — 仍然是集成 God Object |
| dasein 跨服务依赖 | **无** — 仅依赖 fabric，已完成解耦 ✓ |
| bin side-channel | **已消除** — 仅依赖 executive + interact ✓ |
| metacog 隔离 | **良好** — 仅依赖 fabric |
| agora 隔离 | **良好** — 仅依赖 fabric |

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
dasein ──► corpus (具体类型)  ✅ 已移除
dasein ──► mnemosyne (具体类型) ✅ 已移除
```

`_Final(2).md` §12 的要求已满足: Dasein 不再直接依赖 Corpus 与 Mnemosyne 的具体实现。`crates/dasein/Cargo.toml` 中仅保留 `fabric` 作为 workspace 依赖。

### 2.3 低度耦合: `bin` — Side-channel 已消除 ✓

```
bin/Cargo.toml 直接依赖:
  executive, interact
```

bin 以前依赖 6 个 crate (executive, kernel, interact, fabric, cognit, corpus)，现在仅依赖 `executive` + `interact`。`kernel`、`cognit`、`corpus` 的直接依赖已移除。bin 只能通过 `executive` 的公开 API 访问下游类型。

`run_exec()` 函数使用 `ExecSessionBuilder` (shared factory) → `TurnService` 路径执行，不再绕过 executive。

### 2.4 低度耦合: `executive` 内部 — impl/ vs service/ 分裂

| 目录 | 文件数 | 定位 |
|------|--------|------|
| `service/` | ~12+ | "新"代码 — DaemonTurnOrchestrator, TurnService, ExecSessionBuilder |
| `impl/` | 80+ | "旧"代码 — daemon handlers, agents, automation, plugins |
| `core/` | ~28 | bootstrap + CoreSystems + session gateway |

`chat.rs` (482 行 zombie) **已删除** ✓。`bridge/mod.rs` (空模块) **已删除** ✓。逻辑已完全迁移到 `service/daemon_turn/`。

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
| 6 | **Dasein lived time vs Kernel Chronos 边界** | ✅ 明确 | 概念已清晰分离，dasein 不再依赖 kernel。 |
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
cognit       ✓      -      -      -      -       -     -     -       -         -
corpus       ✓      -      -      -      -       -     -     -       -         -
mnemosyne    ✓      -      -      -      -       -     -     -       -         -
dasein       ✓      -      -      -      -       -     -     -       -         -
agora        ✓      -      -      -      -       -     -     -       -         -
metacog      ✓      -      -      -      -       -     -     -       -         -
executive    ✓      ✓      ✓      ✓      ✓       ✓     ✓     ✓       -         -
interact     ✓      -      -      -      -       -     -     -       ✓         -
bin          -      -      -      -      -       -     -     -       ✓         ✓
```

✓ = 依赖, - = 无依赖

**关键变化 (相比 2026-07-12 初期快照)**:

| Crate | 之前 | 现在 | 状态 |
|-------|------|------|------|
| dasein | fabric + corpus + mnemosyne | fabric only | ✅ 已解耦 |
| bin | fabric + kernel + cognit + corpus + executive + interact | executive + interact | ✅ side-channel 已消除 |

**理想状态已达成**: bin 只依赖 executive + interact, dasein 只依赖 fabric。

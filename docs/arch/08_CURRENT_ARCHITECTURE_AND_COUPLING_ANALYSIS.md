# Aletheon Current Architecture & Coupling Analysis

> 生成日期: 2026-07-12
> 基于 dev 分支 commit b14788b
> 本文档反映真实代码状态，非设计建议

---

## 1. Crate 依赖图

```
                    ┌──────────┐
                    │    bin   │
                    └────┬─────┘
                         │ (直接依赖 6 crates: executive, kernel, interact,
                         │  fabric, cognit, corpus — 有 side-channel)
                         │
              ┌──────────┼──────────┐
              ▼          ▼          ▼
         ┌────────┐ ┌───────┐ ┌──────────┐
         │interact│ │  bin  │ │metacog   │──────► fabric
         └───┬────┘ └───┬───┘ └──────────┘
             │          │
             ▼          ▼
         ┌──────────────────────────────────┐
         │           executive              │
         │  (依赖全部 8 crates, God Object)   │
         └──┬───┬───┬───┬───┬───┬───┬──────┘
            │   │   │   │   │   │   │
     ┌──────┘   │   │   │   │   │   └──────┐
     ▼          ▼   ▼   ▼   ▼   ▼          ▼
┌────────┐ ┌──────┐ ┌──────┐ ┌────────┐ ┌──────┐
│ kernel │ │cognit│ │corpus│ │mnemosyne│ │dasein│
└───┬────┘ └──┬───┘ └──┬───┘ └───┬─────┘ └──┬───┘
    │         │        │         │          │
    │         │        │         │          ├────────► corpus
    │         │        │         │          ├────────► mnemosyne
    │         │        │         │          │
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
| executive 依赖数 | **8 crates** — 全部依赖，是集成 God Object |
| dasein 跨服务依赖 | **corpus + mnemosyne** — 唯一的跨服务具体依赖 |
| bin side-channel | **6 crates** — 绕过 executive 直接访问 kernel/cognit/corpus |
| metacog 隔离 | **良好** — 仅依赖 fabric |
| agora 隔离 | **良好** — 仅依赖 fabric |

---

## 2. 耦合分析

### 2.1 重度耦合: `executive` — 集成 God Object

`CoreSystems` 位于 `crates/executive/src/core/core_systems.rs:33-69`，共 *31 个子系统引用*：

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

**问题**: `DaemonTurnOrchestrator` (53 行 struct) 又镜像了 `CoreSystems` 的字段 + kernel primitives。两层 god object。

**改善方向**: 继续将具体字段迁移为 `Arc<dyn TraitOps>` (commit 10cd739 已将部分字段分组)

### 2.2 中度耦合: `dasein` — 跨服务具体依赖

```
dasein ──► fabric           ✅ 合理 (protocol/types)
dasein ──► corpus (具体类型)  ❌ 应改为 trait Port
dasein ──► mnemosyne (具体类型) ❌ 应改为 trait Port
```

`_Final(2).md` §12 明确要求: Dasein 不应直接依赖 Corpus 与 Mnemosyne 的具体实现。

### 2.3 中度耦合: `bin` — Side-channel 访问

```
bin/Cargo.toml 直接依赖:
  executive, kernel, interact, fabric, cognit, corpus
```

bin 可以通过两条路径访问同一类型：`executive::re_export` 和 `cognit::original`。导致不确定哪个是 canonical import path。

### 2.4 低度耦合: `executive` 内部 — impl/ vs service/ 分裂

| 目录 | 文件数 | 定位 |
|------|--------|------|
| `service/` | 12 | "新"代码 — DaemonTurnOrchestrator, TurnService |
| `impl/` | 80+ | "旧"代码 — daemon handlers, agents, automation, plugins |
| `core/` | 28 | bootstrap + CoreSystems + session gateway |

`chat.rs` (482 行) 标记为 `#[allow(dead_code)]` 和 `#[deprecated]`，但文件仍在。逻辑已迁移到 `service/daemon_turn/`，旧文件是 zombie code。

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
│  │                    gateway/     │  └ post_phases │   │
│  │                                │                 │   │
│  │  impl/ (旧代码 80+ files)       tools/           │   │
│  │  ├ daemon/                     └ self_observe   │   │
│  │  ├ agents/                                      │   │
│  │  ├ automation/            bridge/ (空, vestigial)│   │
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
│  │space  │ │bridge│ │hook  │ │ops  │ │      │ │        │  │
│  │admiss │ │test  │ │skill │ │     │ │      │ │        │  │
│  │supv   │ │      │ │      │ │     │ │      │ │        │  │
│  └───┬───┘ └──┬──┘ └──┬──┘ └──┬──┘ └──┬──┘ └───┬────┘  │
│      │        │       │       │       │        │        │
│  ┌───▼────────▼───────▼───────▼───────▼────────▼─────┐  │
│  │                    fabric                          │  │
│  │  contract/ dasein/ events/ include/ ipc/           │  │
│  │  kernel/ policy/ primitives/ types/                │  │
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
| 1 | **唯一 Turn Execution Path** | ⚠️ 部分 | 三套路径: `service/daemon_turn/execute.rs` (canonical) + `bin/main.rs run_exec()` (独立) + `impl/daemon/handler/chat.rs` (zombie)。只有 daemon 路径是完整的。exec 路径绕过 Admission/Session/Event streaming |
| 2 | **拆 `handle_chat`** | ⚠️ 部分 | 业务逻辑已迁移到 `DaemonTurnOrchestrator`，但 `chat.rs` (482 行 zombie) 没删。`impl/daemon/handler/mod.rs` 还标着 `#[allow(dead_code)]` |
| 3 | **修正 `SandboxFirst` fail-closed** | ⚠️ 部分 | 部分路径 warn 后继续，不是真正的 fail-closed |
| 4 | **统一 AgentId/OperationId/Error** | ⚠️ 部分 | 有 AgentId/OperationId，但 `SubAgentSpawner`、`orchestration::Agent`、`AgentProcess` 概念未统一 |
| 5 | **SubAgent 真实执行与 wait/cancel** | ⚠️ 部分 | `SubAgentSpawner` 有 ID/状态/CancellationToken，但缺少结构化 ExitStatus、supervision tree |
| 6 | **Dasein lived time vs Kernel Chronos 边界** | ✅ 明确 | 概念已清晰分离 |
| 7 | **Agora version/proposal/commit** | ⚠️ 部分 | propose/commit 有，但 version CAS 不完整 |

---

## 5. 僵尸代码清单

| 文件/目录 | 状态 | 行动 |
|-----------|------|------|
| `executive/src/impl/daemon/handler/chat.rs` | 482 行 zombie, `#[allow(dead_code)]` | 删除 |
| `executive/src/impl/daemon/handler/mod.rs` | `#[allow(dead_code)]` + deprecated 注释 | 清理 |
| `executive/src/bridge/mod.rs` | 空文件, 仅 `//! Bridge module` | 删除 |
| `corpus/src/testing/` | 已删除 ✓ | — |

---

## 6. 依赖清理建议优先级

| Priority | 行动 | 影响 |
|----------|------|------|
| **P0** | 删除 `chat.rs` zombie + `bridge/` 空模块 | 消除 dead code |
| **P0** | bin `run_exec()` 改为调用 `DaemonTurnOrchestrator` 或共享 `TurnService` | 唯一执行路径 |
| **P1** | dasein 去掉 corpus/mnemosyne 具体依赖，改为 trait Port | 降低跨服务耦合 |
| **P1** | bin Cargo.toml 移除 kernel/cognit/corpus 直接依赖 | 消除 side-channel |
| **P1** | `execute.rs` 拆出 event 转换函数 (230 行) | 降低单文件复杂度 |
| **P2** | CoreSystems 直接字段 → `Arc<dyn TraitOps>` | 解耦 god object |
| **P2** | `impl/daemon/server.rs` DaemonHost 搬到 `host/` | 统一 host 抽象 |
| **P3** | `impl/` 中旧代码归档或删除 | 消除 catch-all 目录 |

---

## 7. Crate 依赖矩阵

```
           fabric kernel cognit corpus mnemosyne dasein agora metacog executive interact
fabric       -      -      -      -      -       -     -     -       -         -
kernel       ✓      -      -      -      -       -     -     -       -         -
cognit       ✓      -      -      -      -       -     -     -       -         -
corpus       ✓      -      -      -      -       -     -     -       -         -
mnemosyne    ✓      -      -      -      -       -     -     -       -         -
dasein       ✓      -      -      ✓      ✓       -     -     -       -         -
agora        ✓      -      -      -      -       -     -     -       -         -
metacog      ✓      -      -      -      -       -     -     -       -         -
executive    ✓      ✓      ✓      ✓      ✓       ✓     ✓     ✓       -         -
interact     ✓      -      -      -      -       -     -     -       ✓         -
bin          ✓      ✓      ✓      ✓      -       -     -     -       ✓         ✓
```

✓ = 依赖, - = 无依赖

**理想状态**: bin 只依赖 executive + interact, dasein 只依赖 fabric

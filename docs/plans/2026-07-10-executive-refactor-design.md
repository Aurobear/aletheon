# Executive Refactor Design

> 基于 RFC-010~013，将 Runtime 收缩为 Executive，建立 6 个子系统边界，
> 引入 Communication Fabric，按三组逐步执行。

**创建日期:** 2026-07-10
**状态:** 已确认

---

## 1. 目标架构（Group C 完成后）

```
┌─────────────────────────────────────────────────────────────┐
│                    aletheon-bin (入口)                        │
│               DaemonHost / SystemdHost / OneShot              │
└──────────────────────┬──────────────────────────────────────┘
                       │
┌──────────────────────┴──────────────────────────────────────┐
│                  executive (原 runtime)                       │
│  Lifecycle │ Scheduler │ Supervisor │ Resource │ Authority   │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │              CoreSystems (trait-based)                   │ │
│  │  cognit_ops │ dasein_ops │ mnemosyne_ops │ corpus_ops   │ │
│  └─────────────────────────────────────────────────────────┘ │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────────┐  │
│  │ Gateway  │  │ Session  │  │ Harness  │  │ Supervisor  │  │
│  │(json-rpc)│  │ Manager  │  │  Runner   │  │  (health)   │  │
│  └──────────┘  └──────────┘  └──────────┘  └────────────┘  │
└─────────────────────────────────────────────────────────────┘
                       │ Communication Fabric
          ┌────────────┼────────────┬────────────┐
          ▼            ▼            ▼            ▼
┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐
│ cognit   │ │ dasein   │ │mnemosyne │ │ corpus   │
│ Planner  │ │ Identity │ │ Episodic │ │ Tool     │
│ Reasoner │ │ Boundary │ │ Semantic │ │ Sandbox  │
│ Verifier │ │ Care     │ │ Replay   │ │ MCP      │
│ Reflector│ │ Narrative│ │ Decay    │ │ Driver   │
│ Learner  │ │ Goal     │ │Associate │ │ Skill    │
└──────────┘ └──────────┘ └──────────┘ └──────────┘
                              ┌──────────┐
                              │ metacog  │
                              │ Genome   │
                              │ Mutation │
                              │ Evaluate │
                              └──────────┘

fabric (原 base): Envelope / Command/Query/Event/Stream / PubSub / Mailbox
interact (不变): TUI / CLI / ACIX
```

**关键变化 vs 当前代码：**
- `RequestHandler` 消失，Executive 只持有 `CoreSystems`（4 个 trait 对象）
- 各子系统之间通过 `fabric::CommunicationBus` 通信，不再直接 `Arc<Mutex<T>>`
- ReAct 循环变成 `Harness` trait（可插拔的认知流水线）
- Memory 子系统拥有自己的完整生命周期（consolidation、decay、replay）

---

## 2. Crate 重命名

| 当前 | 新名称 | Cargo.toml name | 目录 |
|------|--------|-----------------|------|
| `base` | fabric | `fabric` | `crates/fabric/` |
| `memory` | mnemosyne | `mnemosyne` | `crates/mnemosyne/` |
| `runtime` | executive | `executive` | `crates/executive/` |
| `corpus` | — | 不变 | 不变 |
| `dasein` | — | 不变 | 不变 |
| `cognit` | — | 不变 | 不变 |
| `metacog` | — | 不变 | 不变 |
| `interact` | — | 不变 | 不变 |

---

## 3. 分组执行计划

### Group A: 边界建立（Phase 0 + 1）

**目标:** 建立 trait 约束，RequestHandler 字段降到 ≤10。

#### Phase 0 — 四项架构约束（来自 RFC-010）

```
1. 状态归属唯一 — 每种状态只有一个 owner subsystem
2. Executive 不直接持有 Memory / Tool / LLM
3. 所有核心模块通过 trait + message 通信
4. Composition Root 放在 aletheon-bin
```

#### Phase 0 — 定义 4 个 trait 接口

放在 `base` crate（Group A 期尚未重命名），新文件 `base/src/ops.rs`（Group C 后变成 `fabric/src/ops.rs`）：

```rust
// fabric/src/ops.rs

#[async_trait]
pub trait CognitOps: Send + Sync {
    async fn build_context(&self, session: &SessionId) -> Result<Context>;
    async fn reason(&self, ctx: &Context, goal: &Goal) -> Result<Plan>;
    async fn reflect(&self, outcome: &Outcome) -> Result<Reflection>;
}

#[async_trait]
pub trait DaseinOps: Send + Sync {
    async fn review(&self, intent: &Intent, ctx: &Context) -> Result<Verdict>;
    async fn narrate(&self, event: &NarrativeEvent);
    async fn snapshot(&self) -> Result<DaseinSnapshot>;
}

#[async_trait]
pub trait MnemosyneOps: Send + Sync {
    async fn recall(&self, query: &str, limit: usize) -> Result<Vec<MemoryBlock>>;
    async fn store(&self, block: &MemoryBlock) -> Result<()>;
    async fn compose_prompt_block(&self, session: &SessionId) -> Result<String>;
    async fn consolidate(&self) -> Result<()>;
}

#[async_trait]
pub trait CorpusOps: Send + Sync {
    async fn execute(&self, tool: &ToolCall, ctx: &ExecContext) -> Result<ToolResult>;
    async fn list_tools(&self) -> Result<Vec<ToolDef>>;
    async fn run_hooks(&self, event: &HookEvent) -> Result<Vec<HookResult>>;
}
```

#### Phase 1 — CoreSystems + 收缩 RequestHandler

```rust
// runtime/src/core/core_systems.rs (Group A 期，Group C 后变成 executive/src/core/core_systems.rs)

pub struct CoreSystems {
    pub cognit: Arc<dyn CognitOps>,
    pub dasein: Arc<dyn DaseinOps>,
    pub mnemosyne: Arc<dyn MnemosyneOps>,
    pub corpus: Arc<dyn CorpusOps>,
}
```

**RequestHandler 收缩前→后：**

| 类别 | 收缩前（36 字段） | 收缩后（~10 字段） | 去向 |
|------|-----------------|-----------------|------|
| **子系统** | self_field, episodic_memory, recall_memory, core_memory, tools, tool_runner, reflector, fact_store, auto_memory, pipeline, skill_loader, skill_router, agent_registry, agent_loader, objective_store | — | → `CoreSystems` trait impl |
| **会话** | sessions, default_session_id, session_created_at, session_gateway, state | sessions, session_gateway | keep |
| **通信** | bus, event_bus, notify_tx, approval_rx, pending_approvals | bus, notify_tx | keep (精简) |
| **守护** | llm, model_router, data_dir, context_window, started_at, active_connections, cancel_token, daemon_cancel_token | llm, started_at, active_connections, cancel_token | keep (host 层) |
| **安全/运维** | storm_breaker, session_approvals, hooks_config, debug_handler, debug_perf | — | → corpus_ops / host |
| **死代码** | checkpoint_store, event_bus (dead), agent_loader (parked), agent_registry (parked) | 删除 | — |
| **配置类** | config_prompt, cached_prefix, memory_queue, hooks_config | — | → 各自的 subsystem |

**收缩后：**

```rust
pub struct RequestHandler {
    /// Subsystem interfaces — 所有认知工作委托到这里
    subsystems: CoreSystems,
    /// 多会话注册
    sessions: Arc<Mutex<HashMap<String, SessionState>>>,
    /// Session gateway
    session_gateway: Arc<SessionGateway>,
    /// Communication bus (out-of-band notifications)
    bus: Arc<CommunicationBus>,
    /// 默认 LLM provider (过渡期 — 最终迁入 CognitOps)
    llm: Arc<dyn LlmProvider>,
    /// 客户端 push channel
    notify_tx: Option<mpsc::Sender<String>>,
    /// 活跃连接数
    active_connections: Arc<AtomicUsize>,
    /// Daemon 启动时间
    started_at: Instant,
}
```

≤10 字段，符合 RFC-010 要求。

---

### Group B: 模块迁移（Phase 2 + 3 + 4 + 5）

**目标:** 将业务逻辑从 runtime 迁入对应 subsystem crate。

#### Phase 2 — Memory 迁入 Mnemosyne

```
迁移:
  runtime::impl::memory::CoreMemory       → mnemosyne::core_memory
  runtime::impl::memory::RecallMemory     → mnemosyne::recall
  runtime::impl::memory::FactStore        → mnemosyne::fact_store
  runtime::impl::memory::AutoMemory       → mnemosyne::auto_memory
  runtime::impl::memory::compressor       → mnemosyne::compressor
  runtime::impl::memory::memory_pipeline  → mnemosyne::pipeline

executive 端变更:
  handle_chat 中调用 mnemosyne_ops.compose_prompt_block() 替代直接操作 CoreMemory
  handle_chat 中调用 mnemosyne_ops.store() 替代直接操作 RecallMemory

mnemosyne crate 结构 (post-migration):
  crates/mnemosyne/src/
  ├── core/           # Mnemosyne trait, MemoryBlock, SessionId
  ├── impl/
  │   ├── episodic/   # EpisodicMemory (SQLite)
  │   ├── semantic/   # SemanticMemory
  │   ├── procedural/ # ProceduralMemory
  │   ├── self_memory/
  │   ├── core_memory/    # ← from runtime
  │   ├── recall/         # ← from runtime
  │   ├── fact_store/     # ← from runtime
  │   ├── auto_memory/    # ← from runtime
  │   ├── compressor/     # ← from runtime
  │   └── pipeline/       # ← from runtime
  └── bridge/         # MnemosyneOps trait impl
```

#### Phase 3 — ReAct → Cognit（Harness 体系）

```
迁移:
  runtime::core::react_loop → cognit::harness::linear::LinearCognitiveHarness

Harness trait (base crate → Group C 后变成 fabric):
  pub trait CognitiveHarness: Send + Sync {
      async fn run(
          &self,
          input: &str,
          llm: &dyn LlmProvider,
          tools: &[ToolDef],
          executor: &dyn ToolExecutor,
      ) -> Result<(String, TurnMetrics)>;
  }

初始实现 — LinearCognitiveHarness:
  等价于当前 ReAct，但遵循 RFC-012 的节点化设计:
    Goal → Context → Planner → Reasoner → Executor → Verifier → Reflector

executive 端变更:
  handle_chat 中不再自己构建 ReActLoop，改为:
    let harness = LinearCognitiveHarness::new(config);
    harness.run(input, &llm, &tool_defs, &executor).await
```

#### Phase 4 — Skill / Hook / Tool 迁入 Corpus

```
迁移:
  runtime::impl::skills          → corpus::skill
  runtime::impl::skill_router    → corpus::skill::router
  runtime::impl::hooks           → corpus::hook
  runtime::impl::agent_loader    → corpus::agent (parked, 迁过去保持 parked)
  runtime::core::storm_breaker   → corpus::security::storm_breaker

executive 端变更:
  handle_chat 中 skill matching → corpus_ops.match_skills(input)
  handle_chat 中 hook execution → corpus_ops.run_hooks(event)
```

#### Phase 5 — Gateway 分离

```
Gateway（留在 executive/daemon）:
  server.rs          — Unix socket 监听
  handler/mod.rs     — JSON-RPC dispatch（精简后）
  handler/connection.rs
  handler/format.rs
  handler/session_routing.rs
  model_router.rs
  debug_handler.rs

Executive 核心:
  core/runtime_core.rs    — 生命周期
  core/orchestrator.rs    — AletheonRuntime（通过 CoreSystems 编排）
  core/session_gateway.rs
  impl/daemon/session_manager.rs
```

Group B 完成后，`RequestHandler` 变成一个纯 JSON-RPC dispatcher，每个方法只委托给对应 trait。

---

### Group C: 命名收官（Phase 6 + Crate 重命名）

**目标:** 最终命名对齐，文档更新。

#### Phase 6 — `runtime` → `executive`

等所有业务逻辑迁出后，runtime 只负责 RFC-010 定义的 6 个职责：

```
executive 职责 (RFC-010):
  Lifecycle      — RuntimeCore::bootstrap / shutdown
  Scheduler      — AletheonRuntime::process (编排 harness 执行)
  Supervisor     — 健康检查、超时、中断
  Resource       — 会话管理、连接计数、取消令牌
  Communication  — CommunicationBus 持有和分配
  Authority      — 权限分发、approval gate
```

#### Crate 重命名执行步骤

```
1. 全仓替换 import 路径:
   use base::*       → use fabric::*
   use memory::*     → use mnemosyne::*
   use runtime::*    → use executive::*

2. 更新所有 Cargo.toml [package.name]:
   crates/base/Cargo.toml     → name = "fabric"
   crates/memory/Cargo.toml   → name = "mnemosyne"
   crates/runtime/Cargo.toml  → name = "executive"

3. 更新依赖方 Cargo.toml:
   all crates: base → fabric, memory → mnemosyne, runtime → executive

4. git mv:
   crates/base    → crates/fabric
   crates/memory  → crates/mnemosyne
   crates/runtime → crates/executive

5. cargo build --workspace 确认编译通过
6. cargo test --workspace 确认测试通过
7. cargo fmt + cargo clippy 确认无警告

8. 更新 docs/:
   docs/design/README.md               — Crate-to-目录映射表
   docs/design/architecture-overview.md — 模块总览表
   docs/architecture/architecture-doc-2026-07-10.md — 架构文档
```

---

## 4. 当前代码基准

### 当前 Crate 依赖图

```
base (leaf)
  +-- cognit
  +-- corpus
  +-- memory
  +-- metacog
  +-- interact (depends on corpus+base)
  +-- dasein (depends on cognit+corpus+memory+base)
  +-- runtime (depends on ALL: base, cognit, corpus, memory, dasein, metacog)
        +-- aletheon-bin (depends on runtime+interact)
```

### 关键痛点

| 痛点 | 现状 | 目标 |
|------|------|------|
| `RequestHandler` | 36 字段 God Object | ≤10 字段，纯 JSON-RPC dispatcher |
| `chat.rs` handle_chat | ~1100 行，一个方法做 SELF/memory/skill/hook/routing/Dasein/ReAct/approval/auto-memory/reflection/evolution | 委托给 CoreSystems trait |
| 无 trait 抽象 | memory/tools/communication 都是 `Arc<Mutex<T>>` 直接访问 | 全部通过 ops trait 通信 |
| `CommunicationBus` adoption 不完整 | 有 dead `event_bus` 字段，chat 里直接 lock | `CoreSystems` trait + bus 通信 |
| `base` / `memory` 命名 | `base` 太泛化，`memory` 太普通 | `fabric` / `mnemosyne` 语义准确 |

---

## 5. 不变更范围

- `interact` crate — CLI/TUI/ACIX 不在 RFC-010~013 范围内，结构和命名不变
- `metacog` crate — 已对齐 RFC-011，不在本次迁移范围内（但 train 实现会通过 `CoreSystems` trait 对接）
- `aletheon-bin` — 入口点不变，只是 Composition Root 位置调整
- `docs/architecture/` 下的 RFC 文档 — 只读参考，不迁移
- 后续 RFC（Harness 详细设计、Capability 体系、Agent 模型、Long-term Goals 等）— 不在本次范围

---

## 6. 验证标准（每组完成后）

### Group A 验证
- [x] 4 个 ops trait 编译通过
- [x] `CoreSystems` struct 编译通过
- [x] `RequestHandler` 字段 ≤ 10
- [x] `cargo build --workspace` 通过
- [x] `cargo test --workspace` 通过
- [x] `cargo clippy --workspace` 无新警告

### Group B 验证
- [x] `mnemosyne` crate 包含所有迁移的 memory 模块
- [x] `LinearCognitiveHarness` 替代 ReActLoop
- [x] `corpus` crate 包含所有迁移的 skill/hook/tool 模块
- [x] Gateway 和 Executive 代码分离
- [x] `cargo build --workspace` 通过
- [x] `cargo test --workspace` 通过
- [x] `cargo clippy --workspace` 无新警告

### Group C 验证
- [x] `grep -r "use base::" crates/` 返回空
- [x] `grep -r "use memory::" crates/` 返回空
- [x] `grep -r "use runtime::" crates/` 返回空
- [x] `cargo build --workspace` 通过
- [x] `cargo test --workspace` 通过
- [x] docs/ 中所有链接更新

---

## 7. 参考文档

| 文档 | 内容 |
|------|------|
| [RFC-010](docs/architecture/RFC-010-Executive-Refactor.md) | Executive 设计原则、禁止持有的事物 |
| [RFC-011](docs/architecture/RFC-011-Core-Subsystems.md) | 6 个子系统边界、各自拥有的状态 |
| [RFC-012](docs/architecture/RFC-012-Communication-Harness.md) | Communication Fabric (C/Q/E/S) + Harness 模型 |
| [RFC-013](docs/architecture/RFC-013-Refactor-Roadmap.md) | 6 Phase 施工路线图 |
| [gpt.md](docs/architecture/gpt.md) | RFC 体系评论 + Primitive 概念 |
| [architecture-doc-2026-07-10](docs/architecture/architecture-doc-2026-07-10.md) | 当前代码架构完整文档 |
| [architecture-overview](docs/design/architecture-overview.md) | 架构总览 + 数据流 |

---

*文档版本: 1.0.0*
*创建日期: 2026-07-10*

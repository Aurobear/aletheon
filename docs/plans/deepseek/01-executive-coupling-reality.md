# Executive Crate 耦合现状 — 代码级验证

## 概述

逐行扫描 `crates/executive/src/` 下关键模块，对比架构文档中关于 "Executive 已实现 trait-based 解耦" 的声明。

**结论：部分外層模块确实实现了 trait-based 解耦，但核心编排层存在严重的 concrete type 泄漏。文档低估了耦合的实际程度。**

---

## 1. DomainPorts — 清洁，确认

**文件:** `crates/executive/src/core/domain_ports.rs:8-13`

`DomainPorts` 仅持 4 个 `Arc<dyn Trait>` 字段，零 concrete store，零 `Mutex`：

```rust
pub struct DomainPorts {
    pub agora: Arc<dyn AgoraOps>,
    pub metacog: Arc<dyn metacog::MetacogService>,
    pub corpus: Arc<dyn corpus::CorpusService>,
    pub cognition: Arc<dyn CognitiveSessionFactory>,
}
```

**验证结果：100% trait-based，完全符合架构文档的目标。** 正确实现了 "ports" 模式，与 `KernelRuntime` 完全分离。

---

## 2. TurnPipeline — 半具体耦合

**文件:** `crates/executive/src/service/turn_pipeline.rs:42-59`

`TurnPipeline` 共 14 个字段，**7 个是具体类型，2 个包裹 `Arc<Mutex<...>>`**：

| 字段 | 类型 | 类别 | 行号 |
|------|------|------|------|
| `session_gateway` | `Arc<SessionGateway>` | **具体** | 42 |
| `notify_tx` | `Arc<Mutex<Option<mpsc::Sender<String>>>>` | **具体 + Mutex** | 43 |
| `clock` | `Arc<dyn Clock>` | trait | 44 |
| `agora` | `Option<Arc<dyn AgoraOps>>` | trait | 45 |
| `kernel` | `Arc<KernelRuntime>` | **具体** | 47 |
| `current_scope` | `Arc<Mutex<Option<OperationScope>>>` | **具体 + Mutex** | 48 |
| `daemon_cancel_token` | `Option<CancellationToken>` | **具体** | 49 |
| `context_assembler` | `Arc<ContextAssembler>` | **具体** | 51 |
| `canonical_sessions` | `Arc<SessionService>` | **具体** | 52 |
| `post_turn_projection` | `Arc<dyn PostTurnProjection>` | trait | 53 |
| `runtime_ports` | `Arc<TurnRuntimePorts>` | **具体** | 54 |
| `cognitive_sessions` | `Arc<dyn CognitiveSessionFactory>` | trait | 55 |
| `conscious_core` | `Option<Arc<dyn ConsciousTurnPort>>` | trait | 56 |

所有字段都是 `pub(crate)`，不泄漏到 crate 外部。但 `TurnPipeline::run()` 方法（行 274-293）直接调用 `self.kernel.inspect_process()`、`self.kernel.upsert_space_binding()` 等具体方法，绕过了任何 trait 边界。

**验证结果：架构文档标记 Turn-Path 收敛为 "完成"，但这仅是 ReAct 层面的收敛。TurnPipeline 自身的具体类型耦合未被文档提及。**

---

## 3. DaemonTurnOrchestrator — 纯 God Object

**文件:** `crates/executive/src/service/daemon_turn/orchestrator.rs:22-30`

**7/7 字段全部是具体类型，零 trait 对象：**

| 字段 | 类型 | 行号 |
|------|------|------|
| `kernel` | `Arc<KernelRuntime>` | 22 |
| `notify_tx` | `Arc<Mutex<Option<mpsc::Sender<String>>>>` | 23 |
| `main_agent_process_id` | `Arc<Mutex<Option<ProcessId>>>` | 24 |
| `turn_token` | `Arc<Mutex<Option<CancellationToken>>>` | 25 |
| `pipeline` | `Arc<TurnPipeline>` | 27 |
| `coordinator` | `Arc<TurnCoordinator>` | 28 |
| `session_service` | `Arc<SessionService>` | 29 |

其中 4/7 字段包裹 `Arc<Mutex<...>>`。

**公开 API 泄漏:** `notify_tx()` 方法（行 45-47）返回 `&Arc<Mutex<Option<mpsc::Sender<String>>>>` — 向所有调用方暴露裸 mutex handle。

**验证结果：这是 daemon 路径中最深的 concrete-coupling 热点。文档正确指出了 "两层 god object"，但未列出具

体字段的严重程度。**

---

## 4. KernelRuntime 接口 — 文档自述不实

**文件:** `crates/kernel/src/runtime.rs`

文档声称 "Callers receive immutable typed snapshots/results rather than table or lock handles"（行 25-27）。

**所有字段确实是私有的** — 这一点正确。但三个公开方法返回 `Arc<具体可变类型>`：

| 方法 | 返回类型 | 行号 |
|------|----------|------|
| `budget_controller()` | `Arc<InMemoryBudgetController>` | 205 |
| `lease_manager()` | `Arc<InMemoryResourceLeaseManager>` | 209 |
| `mailbox_service()` | `Arc<InProcessMailboxService>` | 189 |

这些不是 "immutable snapshots" — 调用方获得的是对共享可变状态的直接引用。

**内部 Mutex 数量:** 8 个 `Mutex` 字段 + 2 个通过 getter 间接暴露。

**验证结果：私有字段的纪律被公开方法削弱。文档声称 "immutable snapshots" 对这三个 getter 不成立。**

---

## 5. TurnRuntimeResources — 最严重的泄漏

**文件:** `crates/executive/src/service/turn_runtime_ports.rs:105-135`

```rust
pub(crate) struct TurnRuntimeResources {
    // 17 个字段，全部具体类型
    // 8 个包裹 Mutex
    ...
}
```

`pub(crate)` 可见性意味着 executive crate 内部任意模块都可以直接访问 — 包括 `bootstrap/`、`impl/`、`service/` 下的所有代码。

**验证结果：这是 executive crate 内 concrete type 泄漏最严重的位置。文档未单独提及此结构体。**

---

## 6. Bootstrap 组合根 — 仍然分散

**声明:** `bootstrap/mod.rs:5` — "Construction code in this module is the only production code allowed to know the concrete implementations"

**实际:** `crates/executive/src/impl/daemon/bootstrap/mod.rs` 仅 36 行，是一个 thin-shell wrapper。真正的组合根在 `crates/executive/src/impl/daemon/bootstrap/request.rs`，共 **1391 行**，直接实例化 `SystemClock`、`SelfField`、`CoreMemory`、`RecallMemory`、`FactStore`、`ObjectiveStore`、`SessionStore`、`DebugHandler`、`ToolRegistry`、`ToolRunnerWithGuard`、`ModelRouter`、`MorphogenesisPipeline` 等 30+ 具体类型。

**验证结果：文档声明 "construction knowledge is confined to one module" 是误导性的 — bootstrap/mod.rs 是 thin-shell，真正的组合根散布在 5 个子模块中。**

---

## 7. GovernedCapabilityInvoker — 清洁，确认

**文件:** `crates/executive/src/service/governed_capability.rs:105-127`

`GovernedCapabilityInvoker` 仅持 `Arc<dyn CapabilityInvoker>`、`Arc<dyn TurnAuthorityProvider>`、`Option<Arc<dyn GovernedActionLoop>>`。100% trait objects。无 concrete store，无 `Mutex`。

`CapabilityRuntimeFactory`（行 190-213）构建 `DefaultCapabilityInvoker` 并包装为 `GovernedCapabilityInvoker`，所有输入参数都是 `Arc<dyn Trait>`。

**验证结果：完全 trait-clean，是正确的能力执行边界。文档描述准确。**

---

## 8. daemon_react.rs — 清洁，确认

**文件:** `crates/executive/src/service/daemon_react.rs:17-27`

`DaemonStreamingTurnContext<F>` 泛型于工具执行闭包 `F`。`submit_streaming_daemon_turn()`（行 30-75）构建 `DaemonTurnServices<F>` 实现 `TurnServices` trait，完全 trait-based。

**验证结果：这是 executive 中最干净的模块。无 concrete store 访问，无 Mutex 暴露。**

---

## 9. 文档更新状态 (2026-07-17)

本报告中的发现已同步到 `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md`（顶部新增 "Code-Reality Update (2026-07-17)" 章节），包括：
- TurnPipeline/DaemonTurnOrchestrator/TurnRuntimeResources 合计 38+ concrete 字段、14 Mutex
- KernelRuntime "immutable snapshots" 声明修正
- Bootstrap "confined to one module" 修正为 1391 行 request.rs 跨 5 子模块
- CodexRuntime→PiRuntime、AgentHarness/AgentRuntime scaffold 标记

原始计划内容完整保留，仅前置代码实际状态说明。

---

## 总结表

| 模块 | 状态 | 具体字段 | Mutex 包装 | 对外泄漏 |
|------|------|----------|------------|----------|
| `DomainPorts` | ✅ 清洁 | 0 | 0 | 无 |
| `GovernedCapabilityInvoker` | ✅ 清洁 | 0 | 0 | 无 |
| `daemon_react.rs` | ✅ 清洁 | 0 | 0 | 无 |
| `TurnPipeline` | 🔴 耦合 | 7/14 | 2 | `pub(crate)` |
| `DaemonTurnOrchestrator` | 🔴 耦合 | 7/7 | 4/7 | 公开 `Arc<Mutex<>>` |
| `TurnRuntimeResources` | 🔴 耦合 | 17/17 | 8 | `pub(crate)` |
| `KernelRuntime` (getters) | 🟡 部分 | 3 暴露 | 0 (通过 `Arc`) | 公开 `Arc<具体类型>` |
| `Bootstrap` | 🟡 分散 | 30+ | — | 跨 5 文件 |

**结论：Executive 的耦合程度比架构文档描述的更严重。文档宣称的 trait-based 架构仅在外层（DomainPorts、GovernedCapabilityInvoker、daemon_react）成立；核心编排层（TurnPipeline、DaemonTurnOrchestrator、TurnRuntimeResources）存在 38+ 个具体字段和 14 个 Mutex 包装的 concrete type 泄漏。**

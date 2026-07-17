# Kernel / Application Layer Separation — 内核层与应用层分离

> **Status:** Design / Proposed（未实现；实现需另行批准）
>
> **Date:** 2026-07-17
>
> **Baseline:** `dev` HEAD
>
> **一句话:** `aletheon-kernel` 在**crate 与依赖图层面已经是干净的机制层**（无策略、无环、只依赖 `fabric`）。真正没做完的不是"拆分"，而是"封边"——应用层仍通过具体类型 getter 和 kernel 子模块直连 kernel 内部。本计划把边界收成"只经 fabric trait"，并用 CI 永久锁定。

---

## 1. 现状：分层已成，边界未封

### 1.1 kernel 是什么（`crates/kernel/`）
- 依赖：仅 `fabric`（+ tokio/anyhow/tracing）——`kernel/Cargo.toml`。
- 模块（`kernel/src/lib.rs:3-10`）：`admission / capability / chronos / operation / process / space / supervision / runtime`。
- `KernelRuntime`（`runtime.rs:28-47`）= 唯一跨表生命周期句柄，**私有**拥有：`Clock`、`InMemorySpaceManager`、`ProcessTable`、`OperationTable`、`SupervisorTree`、`InProcessMailboxService`、`AdmissionController`、`InMemoryBudgetController`、`InMemoryResourceLeaseManager` 及若干内存映射。
- 公开面：进程生命周期（`spawn_process:308`、`terminate_process:463`、`signal_process:440`、`inspect_process:640`）、操作生命周期（`submit_operation:662`、`succeed/fail/cancel:730-756`）、预算预留（213-302）、监督（`supervise:304`）、空间（766-811）。实现 fabric trait `ProcessManager`（829）、`OperationManager`（848）。
- **判定：kernel 已经是"纯机制"** —— 进程/操作表、admission/budget/lease、chronos、supervision、mailbox 传输，全部 domain-neutral，无策略。**没有东西需要"搬进"kernel。**

### 1.2 依赖方向：干净、无环
- 7 个 crate 依赖 kernel：`agora / dasein / mnemosyne / executive / metacog / corpus / cognit`。
- kernel **不依赖任何应用 crate**，只依赖 `fabric`（叶子，持共享契约）。
- 图为有向无环：`fabric ← kernel ← {domain crates, executive}`。**耦合来自具体类型泄漏，不是依赖环。**

### 1.3 边界破口（本计划要封的三处，均已验证）

| # | 破口 | 证据 | 后果 |
|---|------|------|------|
| B1 | **getter 泄漏具体可变类型** | `mailbox_service()` → `Arc<InProcessMailboxService>`（`runtime.rs:189`）；`budget_controller()` → `Arc<InMemoryBudgetController>`（`:205`）；`lease_manager()` → `Arc<InMemoryResourceLeaseManager>`（`:209`）。而 `:26` doc 声称 "immutable typed snapshots"——**不实**，是共享可变 `Arc` | 应用层绑死实现；无法替换/测试；文档说谎 |
| B2 | **应用层直连 kernel 子模块**（绕过 facade） | executive 直接 `use aletheon_kernel::chronos::{SystemClock,SystemTimer,TestClock}`（`core/runtime_core.rs:28`、`host/mod.rs:18` 等多处）、`::operation::OperationScope`（`daemon_turn/execute.rs:75`）、`::capability::ToolExecutor`（`mcp_embedded.rs:239`） | 内部结构成了公共 API；kernel 内部一改就波及应用层 |
| B3 | **应用侧聚合句柄泄漏具体类型** | `TurnRuntimeResources`（`turn_runtime_ports.rs:105-128`）19 个 `pub(crate)` 字段、~17 具体类型、~7-8 个 `Mutex`；`DaemonTurnOrchestrator`（`daemon_turn/orchestrator.rs:22-30`）7/7 具体、`notify_tx():45` 直接返回 `&Arc<Mutex<Option<...>>>` | 应用层内部同样"裸露"，与 kernel 破口同源 |

> B3 的 `DaemonTurnOrchestrator` 实为"漏抽象的聚合句柄"而非体量意义的 God Object（本体仅 86 行 / 5 方法，逻辑已拆到 `execute.rs`/`helpers.rs`/`lifecycle.rs`）。

---

## 2. 澄清：三种"分离"别混淆

用户说的"kernel 和应用层分开"，需与已有工作区分：

| 分离维度 | 层级 | 现状 |
|----------|------|------|
| **进程/运行时分离**（system-core vs user runtime） | 部署/进程 | **已完成**（见 `2026-07-17-multi-user-runtime-m0-m2.md`：`system_core_runtime.rs`、`user_runtime/`、systemd core/user 单元） |
| **crate/依赖分离**（mechanism vs policy） | 编译期 | **已成**（§1.2，无环） |
| **边界封装**（只经 trait 交互） | 类型/API | **未完成 ← 本计划** |

本计划只做第三种：**封边**。

---

## 3. 目标边界（设计）

```text
        APPLICATION (policy / orchestration)
   executive turn pipeline · DaemonTurnOrchestrator · AletheonExecutive
   · model routing · storm · self-policy · sessions · bootstrap composition
        │  只允许经由 ▼ 这些 fabric trait 与 kernel 交互
   ┌────┴───────────────────────────────────────────────────────────┐
   │  FABRIC contracts (leaf crate)                                   │
   │   已存在:  ProcessManager · OperationManager · Clock · Timer     │
   │            AdmissionController · SpaceManager · MailboxService    │
   │   需新增:  BudgetController(trait) · LeaseManager(trait)          │← B1 根因
   └────┬───────────────────────────────────────────────────────────┘
        │  KernelRuntime 实现这些 trait；getter 返回 Arc<dyn Trait>
        ▼
        KERNEL (mechanism)  process/operation/admission/budget/lease/chronos/supervision/mailbox
```

**核心原则**：应用层**只**能看到 `fabric` 里的 trait，**永远看不到** `aletheon_kernel::{chronos,operation,capability,...}` 的具体类型。

---

## 4. 变更设计（分阶段，全部只写设计）

### K1 — 补齐缺失的 fabric trait（B1 根因）
- **根因**：`mailbox_service` 能返回 `Arc<dyn MailboxService>`（trait 已在 `fabric/src/ipc/mailbox.rs`），但 budget/lease **在 fabric 里根本没有 trait**，所以只能返回具体类型。
- **做什么**：在 `fabric` 新增 `BudgetController` 与 `LeaseManager` 两个 trait（对齐现有 `AdmissionController`/`SpaceManager` 的风格，放 `fabric/src/include/`）。`KernelRuntime` 的 `InMemory*` 实现它们。
- **验收 AC-K1.1**：`fabric` 中存在 `BudgetController`/`LeaseManager` trait，`KernelRuntime` 内部类型 `impl` 之。

### K2 — getter 返回 trait 对象（B1）
- **做什么**：`mailbox_service()`/`budget_controller()`/`lease_manager()`（`runtime.rs:189/205/209`）改为返回 `Arc<dyn MailboxService>` / `Arc<dyn BudgetController>` / `Arc<dyn LeaseManager>`。同步修正 `runtime.rs:26` 的 doc（要么真做成 immutable snapshot，要么如实描述为"共享句柄经 trait 暴露"）。
- **验收 AC-K2.1**：kernel 公共 getter 无一返回 `Arc<具体可变类型>`；文档与实现一致。

### K3 — 应用层改走 facade / fabric（B2）
- **做什么**：把 executive 里对 `aletheon_kernel::{chronos,operation::OperationScope,capability::ToolExecutor}` 的直接 `use` 换成 `fabric` 对应契约（`Clock`/`Timer` 已在 fabric；`OperationScope`/`ToolExecutor` 若需跨界，应在 fabric 有对应类型或 re-export）。`TurnPipeline::run()` 对 `kernel.inspect_process()`/`upsert_space_binding()` 的调用保留（这些是 facade 公共面，合法），但不得触达子模块内部类型。
- **验收 AC-K3.1**：`crates/executive` 中不再出现 `use aletheon_kernel::{chronos|operation|capability|process|space|admission}::…` 的**子模块**直连（只允许 `use aletheon_kernel::{KernelRuntime}` 顶层 facade + `use aletheon_fabric::…` 契约）。

### K4 — CI 封边（把 K1–K3 永久锁定）
- **做什么**：在架构 fitness gate（`scripts/architecture-check.sh`，已存在 485 行 + allowlist 回归防护）新增两条删除门：
  1. 禁止 `crates/executive/**` 出现 `aletheon_kernel::{chronos,operation,capability,process,space,admission,supervision}::`（子模块直连）。
  2. 禁止 kernel 公共 API 返回 `Arc<Concrete>` 跨边界（对齐既有 `2026-07-15-architecture-coupling-optimization-plan.md:1106` 的目标"no public `Arc<Mutex<ConcreteDomainStore>>` crosses a domain boundary"）。
- **验收 AC-K4.1**：新增门在 CI 生效；故意引入一处子模块直连会被 CI 拒绝。

### K5 — 应用侧聚合句柄收口（B3，可选/后续）
- **做什么**：`TurnRuntimeResources` 的 19 个 `pub(crate)` 具体字段收敛为按能力分组的 trait 视图；`DaemonTurnOrchestrator::notify_tx()` 不再裸露 `Arc<Mutex<…>>`，改为方法。此项属应用层**内部**整洁，与 kernel 边界正交，风险更高、收益更"架构美学"，建议放最后并单独评审。
- **验收 AC-K5.1**：应用层内部无跨模块 `pub(crate)` 具体可变句柄泄漏（可用 fitness gate 度量）。

---

## 5. 分批与风险
```text
批次 1:  K1 → K2   补 trait + getter 返回 trait 对象（编译期收敛，风险低，测试守护）
批次 2:  K3 → K4   应用层改走 facade + CI 封边（防回归）
批次 3:  K5        应用侧聚合句柄收口（可选，单独评审）
```

**里程碑文件（已拆分）** —— 本文是总设计；下列为逐里程碑可执行详单（触及文件、任务分解、验收、依赖）：
- 批次 1 → [`2026-07-17-kernel-k1-k2-fabric-traits-detailed-plan.md`](./2026-07-17-kernel-k1-k2-fabric-traits-detailed-plan.md)
- 批次 2 → [`2026-07-17-kernel-k3-k4-facade-and-ci-detailed-plan.md`](./2026-07-17-kernel-k3-k4-facade-and-ci-detailed-plan.md)
- 批次 3 → [`2026-07-17-kernel-k5-aggregate-handle-detailed-plan.md`](./2026-07-17-kernel-k5-aggregate-handle-detailed-plan.md)

**不变量**
1. 依赖图保持无环、kernel 不依赖应用 crate。
2. 每步后全量测试（2,766）绿；行为不变（纯类型/边界重构）。
3. 不因封边而在 kernel 里塞入任何策略——kernel 保持 domain-neutral。

**风险**
- trait 对象化可能触及热路径的动态分发开销 → 缓解：budget/lease/mailbox 非高频热路径，影响可忽略；若某处确证性能敏感，保留具体类型但仍以 trait 暴露公共面。
- K3 改动面广（多处 `use`）→ 缓解：机械替换 + 编译器兜底 + CI 门锁定，逐 crate 推进。

---

## 6. 与已有文档的关系
| 文档 | 关系 |
|------|------|
| `2026-07-15-architecture-coupling-optimization-plan.md` | **本计划的母计划**；§7.1 Authority map（`:772`）、目标（`:1106`）即三处破口的出处。本文是其在 kernel/app 边界上的可执行细化 |
| `docs/arch/Aletheon_MacroKernel_Architecture_Final.md` / `CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md` | 宏内核架构与耦合分析背景 |
| `2026-07-17-multi-user-runtime-m0-m2.md` | 已完成**进程级** core/user 分离；本文做**类型级**边界封装，二者不同层、互补 |
| `01-executive-coupling-reality.md` | 提供 executive 侧耦合的代码级证据 |

---

## 7. 完成定义（DoD）
1. `fabric` 有 `BudgetController`/`LeaseManager` trait，kernel 实现之（K1）。
2. kernel 公共 getter 全部返回 `Arc<dyn Trait>`，`runtime.rs:26` doc 与实现一致（K2）。
3. `crates/executive` 不再直连 `aletheon_kernel` 子模块，只经顶层 facade + fabric 契约（K3）。
4. CI fitness gate 有两条新删除门锁定上述边界（K4）。
5. 依赖图仍无环、全量测试绿、行为不变（不变量）。
6. （可选）应用侧聚合句柄收口（K5）。

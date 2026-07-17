# Kernel K1–K2 — 补齐 fabric trait + getter 返回 trait 对象（detailed plan）

> **Status:** Design only（不含实现代码；实现需另行批准）
> **Parent:** `2026-07-17-kernel-application-layer-separation-plan.md` §4 K1–K2 / §5 批次 1
> **批次:** 1（编译期收敛，风险低，测试守护）
> **目标:** 消除 kernel getter 的具体类型泄漏（破口 B1）——补齐缺失的 fabric trait，让公共 getter 返回 `Arc<dyn Trait>`。

## 触及文件（锚点）
- `crates/kernel/src/runtime.rs:189` — `mailbox_service()` → `Arc<InProcessMailboxService>`
- `crates/kernel/src/runtime.rs:205` — `budget_controller()` → `Arc<InMemoryBudgetController>`
- `crates/kernel/src/runtime.rs:209` — `lease_manager()` → `Arc<InMemoryResourceLeaseManager>`
- `crates/kernel/src/runtime.rs:26` — doc 声称 "immutable typed snapshots"（不实，需改）
- `crates/fabric/src/ipc/mailbox.rs` — `MailboxService` trait（已存在）
- `crates/fabric/src/include/` — 新增 `BudgetController` / `LeaseManager` trait（对齐 `AdmissionController`/`SpaceManager` 风格）

## 任务分解（TDD）
### K1 补 trait（B1 根因）
1. **T1** 在 `fabric/src/include/` 新增 `BudgetController` trait（预算预留/释放接口，抽自 `InMemoryBudgetController` 公共面）。
2. **T2** 新增 `LeaseManager` trait（资源租约接口，抽自 `InMemoryResourceLeaseManager`）。
3. **T3** kernel 的 `InMemory*` 类型 `impl` 这两个 trait。

### K2 getter 返回 trait 对象
4. **T4** `mailbox_service()` 返回 `Arc<dyn MailboxService>`；`budget_controller()` 返回 `Arc<dyn BudgetController>`；`lease_manager()` 返回 `Arc<dyn LeaseManager>`。
5. **T5** 修正 `runtime.rs:26` doc：要么真做成 immutable snapshot，要么如实描述为"经 trait 暴露的共享句柄"。
6. **T6** 全量测试（2,766）绿；依赖图仍无环。

## 验收（来自父计划）
- **AC-K1.1** `fabric` 存在 `BudgetController`/`LeaseManager` trait，`KernelRuntime` 内部类型 impl 之。
- **AC-K2.1** kernel 公共 getter 无一返回 `Arc<具体可变类型>`；文档与实现一致。

## 不变量 / 风险
- 依赖图保持无环、kernel 不依赖应用 crate。
- 行为不变（纯类型/边界重构）。
- 动态分发开销：budget/lease/mailbox 非高频热路径，影响可忽略。

## 依赖
- 无（封边起点）。

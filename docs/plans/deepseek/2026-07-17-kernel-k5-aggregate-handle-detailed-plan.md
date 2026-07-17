# Kernel K5 — 应用侧聚合句柄收口（detailed plan）

> **Status:** Design only（不含实现代码；实现需另行批准）
> **Parent:** `2026-07-17-kernel-application-layer-separation-plan.md` §4 K5 / §5 批次 3
> **批次:** 3（可选/后续；应用层内部整洁，与 kernel 边界正交，风险更高，单独评审）
> **目标:** 收敛应用层内部的具体类型/可变句柄泄漏（破口 B3）——`TurnRuntimeResources` 与 `DaemonTurnOrchestrator`。

## 触及文件（锚点）
- `crates/executive/src/.../turn_runtime_ports.rs:105-128` — `TurnRuntimeResources`：19 个 `pub(crate)` 字段、~17 具体类型、~7-8 `Mutex`
- `crates/executive/src/.../daemon_turn/orchestrator.rs:22-30` — `DaemonTurnOrchestrator`：7/7 具体字段
- `crates/executive/src/.../daemon_turn/orchestrator.rs:45` — `notify_tx()` 裸露 `&Arc<Mutex<Option<mpsc::Sender>>>`

## 任务分解（TDD）
1. **T1** `TurnRuntimeResources` 的具体字段按能力分组，收敛为 trait 视图（减少 `pub(crate)` 具体可变句柄跨模块泄漏）。
2. **T2** `notify_tx()` 不再返回裸 `Arc<Mutex<…>>`，改为方法封装（send/subscribe 语义）。
3. **T3** fitness gate 度量：应用层内部无跨模块 `pub(crate)` 具体可变句柄泄漏。

## 验收（来自父计划）
- **AC-K5.1** 应用层内部无跨模块 `pub(crate)` 具体可变句柄泄漏（可用 fitness gate 度量）。

## 不变量 / 风险
- 与 kernel 边界正交，属应用层内部整洁；收益偏"架构美学"，风险更高。
- 不改行为；全量测试绿。
- **建议放最后并单独评审**（不阻塞 K1–K4 的 kernel 封边收益）。

## 依赖
- 建议在 **K1–K4** 之后（先把 kernel 边界封住，再收应用侧内部）。

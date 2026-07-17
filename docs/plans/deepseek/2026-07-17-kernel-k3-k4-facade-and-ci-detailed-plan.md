# Kernel K3–K4 — 应用层改走 facade + CI 封边（detailed plan）

> **Status:** Design only（不含实现代码；实现需另行批准）
> **Parent:** `2026-07-17-kernel-application-layer-separation-plan.md` §4 K3–K4 / §5 批次 2
> **批次:** 2（改 import 面 + 防回归）
> **目标:** 让应用层只经顶层 facade + fabric 契约与 kernel 交互（破口 B2），并用 CI 永久锁定。

## 触及文件（锚点）
- `crates/executive/src/core/runtime_core.rs:28` — `use aletheon_kernel::chronos::{SystemClock,SystemTimer,TestClock}`
- `crates/executive/src/.../host/mod.rs:18` — 同上 chronos 直连
- `crates/executive/src/.../daemon_turn/execute.rs:75` — `use aletheon_kernel::operation::OperationScope`
- `crates/executive/src/.../mcp_embedded.rs:239` — `use aletheon_kernel::capability::ToolExecutor`
- `crates/fabric/src/include/` — `Clock`/`Timer` 已在 fabric；`OperationScope`/`ToolExecutor` 若需跨界应在 fabric 有对应类型或 re-export
- `scripts/architecture-check.sh` — 现有 485 行 fitness gate + allowlist 回归防护（加新删除门）

## 任务分解（TDD）
### K3 改走 facade
1. **T1** 把 executive 对 `aletheon_kernel::chronos::*` 的直连换成 fabric 的 `Clock`/`Timer` 契约。
2. **T2** `OperationScope` / `ToolExecutor`：若确需跨界，在 fabric 提供对应类型或 re-export，executive 改用之。
3. **T3** 保留 `TurnPipeline::run()` 对 `kernel.inspect_process()`/`upsert_space_binding()` 的调用（facade 公共面，合法），但不得触达子模块内部类型。

### K4 CI 封边
4. **T4** fitness gate 新增删除门 #1：禁止 `crates/executive/**` 出现 `aletheon_kernel::{chronos,operation,capability,process,space,admission,supervision}::`（子模块直连）。
5. **T5** 新增删除门 #2：禁止 kernel 公共 API 返回 `Arc<Concrete>` 跨边界（对齐母计划 `2026-07-15-architecture-coupling-optimization-plan.md:1106`）。
6. **T6** 故意引入一处子模块直连 → CI 拒绝（门有效性测试）。

## 验收（来自父计划）
- **AC-K3.1** `crates/executive` 不再出现 `aletheon_kernel` 子模块直连；只用顶层 facade + fabric 契约。
- **AC-K4.1** 两条新删除门在 CI 生效，违规被拒。

## 不变量 / 风险
- 改动面广（多处 `use`）→ 机械替换 + 编译器兜底 + CI 门锁定，逐 crate 推进。
- 行为不变；依赖图仍无环。

## 依赖
- **K1 + K2**（fabric 契约与 trait-对象 getter 必须先就位，K3 才有 fabric 类型可用）。

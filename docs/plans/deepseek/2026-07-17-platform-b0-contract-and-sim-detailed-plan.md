# Platform B0 — 硬件契约与仿真后端（detailed plan）

> **Status:** Design only（不含实现代码；实现需另行批准）
> **Parent:** `2026-07-17-platform-driver-hardware-control-plan.md` §3.4 B0
> **批次:** B0（零真机、可 CI；B 线起点）
> **目标:** 定义实际硬件控制的 trait 契约 + 类型 + 仿真后端，让上层与 CI 在无真机时即可开发、测试。

## 触及文件（锚点）
- `crates/fabric/src/include/body.rs:73` — `BodyRuntime::execute(Action)`（唯一集成缝，复用）
- 新增 `crates/corpus/src/drivers/hardware/mod.rs`（或独立 `crates/embodiment/`）— `EffectorDriver` / `SensorDriver` trait
- 新增 `crates/corpus/src/drivers/hardware/types.rs` — `JointState`/`JointCommand`/`SensorFrame`/`PdoMap`/`HardwareCaps`（与 UI 原语 `drivers/types.rs` 分离）
- 参照现有 mock 模式：`drivers/display/mod.rs:24`、`drivers/input/mod.rs:37`

## 任务分解（TDD）
1. **T1** 定义 `EffectorDriver`（`command(JointCmd/IoCmd)` + `capabilities() -> HardwareCaps`）与 `SensorDriver`（`read(SensorFrame)` / `subscribe`），均 `Send + Sync`。
2. **T2** 新增 `hardware/types.rs`（关节/传感/PDO/能力类型），不复用 UI 原语。
3. **T3** 实现 `MockHardware`（仿真后端）：可回放确定性 `SensorFrame`，接受 `JointCommand`。
4. **T4** 让 `BodyRuntime` 可接仿真后端：一个 `Action` → `MockHardware` → 回读 `SensorFrame`。
5. **T5** `capabilities()` 诚实返回仿真能力表（供上层/诊断查询）。

## 验收（来自父计划）
- **AC-B0.1** `BodyRuntime` 接仿真后端后，一个 `Action` 能驱动 `MockHardware` 并回读 `SensorFrame`；CI 无真机通过。

## 不变量 / 风险
- **诚实的能力表**：仿真后端 `capabilities()` 必须如实反映，不谎报（上层据此决策）。
- 仿真优先：无真机也能验证上层逻辑，避免硬件成为唯一验证路径。
- 契约放 `fabric`（与 `BodyRuntime` 同层），后端放 corpus/embodiment，遵守依赖方向。

## 依赖
- 无（B 线起点）。与 `kernel/application 分离` 计划协同：新 trait 放 fabric。

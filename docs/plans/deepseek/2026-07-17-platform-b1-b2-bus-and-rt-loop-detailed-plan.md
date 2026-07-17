# Platform B1–B2 — 单总线打通 + 确定性 RT 回路（detailed plan）

> **Status:** Design only（不含实现代码；实现需另行批准）
> **Parent:** `2026-07-17-platform-driver-hardware-control-plan.md` §3.4 B1–B2
> **批次:** B1（单总线，非实时）→ B2（确定性 RT 回路）
> **目标:** 用一条最易验证的总线实现首个真实后端，再引入确定性控制回路把它接入周期读写。

## 触及文件（锚点）
- `crates/corpus/src/drivers/io/mod.rs:1` — 当前空 TODO stub（低层设备绑定天然落点）
- `crates/corpus/src/drivers/proc/mod.rs:1` — 当前空 TODO stub
- `crates/corpus/src/drivers/hardware/`（B0 建立）— 新增总线后端 + RT 回路模块
- 依赖候选：`socketcan`（CAN）/ `serialport`（serial）—— B1 建议选 serial 或 CAN（最易验证）

## 任务分解（TDD）
### B1 单总线
1. **T1** 选定总线（建议 serial 或 CAN），实现一个真实 `EffectorDriver`/`SensorDriver` 后端，非实时、请求-响应级。
2. **T2** 后端 `capabilities()` 如实反映该总线可用能力；不可用时可诊断降级（非静默失败）。
3. **T3** 后端与 `MockHardware` 共用同一 trait，测试可切换真机/仿真。

### B2 RT 回路
4. **T4** 引入确定性控制线程：独立 OS 线程 + `SCHED_FIFO` / `PREEMPT_RT`，**不在 tokio async 上下文内**。
5. **T5** async 世界（daemon/turn pipeline）与 RT 回路之间用**无锁 ring buffer + 命令/状态双缓冲**通信；RT 内**零分配 / 零加锁 / 零 `.await`**。
6. **T6** 把 B1 后端接入周期回路，先低频（100Hz）：周期读传感 → 计算 → 写执行器。

## 验收（来自父计划）
- **AC-B2.1** RT 回路以设定频率运行，抖动（jitter）在目标阈值内；回路内零分配 / 零 `.await`（可用隔离子模块或审查佐证）。
- **AC-B*.2** 硬件动作经过权限治理，无绕过。

## 不变量 / 风险
- **RT 与 async 严格解耦**：违反即引入不可预测延迟（参考 `dataflow` skill 关注的 100Hz↔1kHz 边界与帧丢失）。
- 一条总线打通即里程碑，勿一次上多总线。

## 依赖
- **B0**（trait + 类型 + 仿真必须先就位）。

# Platform B3(+B4) — EtherCAT/PDO 现场总线 + fail-safe（detailed plan）

> **Status:** Design only（不含实现代码；实现需另行批准）
> **Parent:** `2026-07-17-platform-driver-hardware-control-plan.md` §3.4 B3–B4
> **批次:** B3（现场总线 + 高频 + 硬安全）→ B4（可选 ROS 桥）
> **目标:** 引入 EtherCAT master + PDO 映射，把控制回路提升到 1kHz，并加硬件级 fail-safe（急停/看门狗/限位）。

## 触及文件（锚点）
- `crates/corpus/src/drivers/hardware/`（B0/B2 建立）— 新增 EtherCAT master + PDO 映射
- `crates/corpus/src/drivers/hardware/types.rs` — 扩展 `PdoMap`
- RT 回路模块（B2 建立）— 提频 + 看门狗/急停
- 依赖候选：`ethercrab` / SOEM 绑定；B4：ROS2 后端实现 `BodyRuntime`（对齐 `fabric/.../body.rs:71` 预留）

## 任务分解（TDD）
### B3 现场总线
1. **T1** EtherCAT master + PDO 映射；把执行器/传感映射到 `JointCommand`/`SensorFrame`。
2. **T2** 控制回路提频至 1kHz（沿用 B2 的无锁 ring + 双缓冲，仍零分配）。
3. **T3** **硬件 fail-safe**：看门狗超时 / 急停触发 → 执行器进入 fail-safe（断电或回中）；限位保护。
4. **T4** fail-safe 路径有确定性测试（可在仿真后端注入超时/急停）。

### B4（可选）ROS 桥
5. **T5** 实现 `BodyRuntime` 的 ROS2 后端，与 ROS 生态互操作（仅在需要时）。

## 验收（来自父计划）
- **AC-B3.1** 看门狗超时 / 急停触发时，执行器进入 fail-safe，且该路径有测试。
- **AC-B*.2** 硬件动作全部经过权限治理，无绕过。

## 不变量 / 风险
- **硬件 fail-safe 优先于一切软件逻辑**：急停/看门狗/限位是硬约束，**不受**意识层软否决或工具层影响。
- 意识层（conscious-core R3）可**收紧**硬件动作，但不得替代 fail-safe。
- 1kHz 下 RT 纪律更严：任何分配/加锁/`.await` 都可能致命。

## 依赖
- **B1 + B2**（总线后端 + RT 回路必须先稳定）。

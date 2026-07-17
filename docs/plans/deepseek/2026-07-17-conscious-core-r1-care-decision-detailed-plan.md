# Conscious-Core R1 — 让 `determine_action()` 进入闭环（detailed plan）

> **Status:** Design only（不含实现代码；实现需另行批准）
> **Parent:** `2026-07-17-conscious-core-engineering-plan.md` §5 R1 / §7 批次 1
> **批次:** 1（低风险，不改行为，只增信号）
> **目标:** 让 Sorge 的 `CareAction` 决策成为闭环一等信号，可被 Agora 竞争/广播消费。

## 触及文件（锚点）
- `crates/dasein/src/dasein/reducer.rs:408-417` — `ScheduledReflection` 分支（当前不调 `determine_action`）
- `crates/dasein/src/dasein/care_structure.rs:184-213` — `determine_action()`（现只在单测可达）
- `crates/dasein/src/dasein/*`（SelfSignal 定义处）— 新增 `CareDecision(CareAction)` 变体 + serde
- `crates/agora/.../conscious_core_coordinator.rs:404-446` — 复用 `submit_dasein_candidates`（`emitted`→`Concern` 映射，勿新建通道）

## 任务分解（TDD，先写测试意图）
1. **T1** 定义 `SelfSignal::CareDecision(CareAction)` 变体及其序列化；确认与现有 ledger 事件类型解耦（**不**新增持久化事件）。
2. **T2** 在 `reducer.rs:408-417` 的 `ScheduledReflection` 分支调用 `self.care.determine_action(...)`，将结果 `emit` 为 `CareDecision`。
3. **T3** 验证 `CareDecision` 经 `submit_dasein_candidates` 映射为 Agora `Concern` 并参与竞争/广播（不改映射代码，仅走既有通道）。
4. **T4** 可观察性：`emit` 时结构化记录 `care_decision=<variant>`，供 `aletheon_diagnose` / 广播快照读取。

## 验收（来自父计划）
- **AC-R1.1** 给定触发 `Negate` 的 care 状态，一次 reflection cycle 后，Agora 广播候选集合包含由 `CareDecision(Negate)` 派生的 Concern。
- **AC-R1.2** `determine_action()` 存在生产调用点（不再"仅测试可达"）。
- **AC-R1.3** replay 一段历史事件，最终状态与未加 R1 前**逐字节一致**。

## 不变量 / 风险
- **不持久化 `CareDecision`**：它是派生信号，写入 `SelfLedger` 会破坏 checksum chain 与 replay 确定性（AC-R1.3 守护）。
- 意识核心未点火时行为不变（降级安全）。

## 依赖
- 无（可独立开工，是三步中的起点）。

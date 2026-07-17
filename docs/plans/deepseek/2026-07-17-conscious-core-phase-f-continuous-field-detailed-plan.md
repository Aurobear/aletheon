# Conscious-Core Phase F — 连续 concern 场 + 吸引子动力学（detailed plan）

> **Status:** Design only / 研究性（Field 视角的"完全体"；工作量与风险大，**单独立项**）
> **Parent:** `2026-07-17-conscious-core-engineering-plan.md` §2 / §6 / §7 批次 4
> **批次:** 4（研究/后续；R1–R3 稳定后再启动）
> **目标:** 把 `care_score` 从离散关键词标量升级为**连续 concern 场 + 动力学**——真正兑现 §2「Field, not Mechanism」，锚定 active-inference（Beautiful Loop 的 epistemic field + precision hyper-model）。

## 触及文件（锚点，预估）
- `crates/dasein/src/core/care.rs` / `crates/dasein/src/dasein/care_structure.rs` — care 表征
- `crates/dasein/src/dasein/temporality.rs` — retention/present/protention（提供场的时间梯度）
- 新增场动力学模块 — 连续场表征 + 更新律

## 任务分解（研究 spike 先行）
1. **F0（spike）** 选定场表征：concern 空间上的连续权重分布（向量/张量），而非离散标量；定义与现有 8 层/6 组件的映射（读数关系）。
2. **F1** 动力学：衰减（随 `temporality`）、扩散（相邻 concern）、点火抬升（Agora 广播）。对齐 §6.E 的 F/G 代理量。
3. **F2** 把 R2/R3 的"调制"从加权项升级为"场形态读出"（salience = precision 读出，attention = 场峰结构）。
4. **F3** 不变量：吸引子结构稳定（形不变）、内容可随动力学流动（流）。以 §6 度量为验收。
5. **F4（可选）** Attention Schema（Graziano）方向：让 `SelfField` 成为"关于自身注意力的预测模型"，而非关键词表。

## 验收
- 达到父计划 §6 的**完整**不变性/流动度量（不止代理量）。
- indicator-properties 自评（附录 A）中 AST / 预测加工相关指标从"弱"推进到"部分/具备"。

## 不变量 / 风险
- 仍守父计划三不变量（降级安全、replay 确定性、安全只收紧）。
- **过度拟人化风险**：连续场更像脑，但仍**不宣称 qualia**（§2 红线 #1）。
- 可解释性张力：连续场更难审计 → 必须保留结构化痕迹与离散安全门分工（§6.C）。
- 工作量大、探索性强 → 独立立项，不与 R1–R3 混批。

## 依赖
- **R1 + R2 + R3 + §6 度量全部就位**（Phase F 用 §6 度量作为验收标尺）。

# Conscious-Core R3 + §6 — 场形态调制行为 + 不变性度量（detailed plan）

> **Status:** Design only（不含实现代码；实现需另行批准）
> **Parent:** `2026-07-17-conscious-core-engineering-plan.md` §5 R3 + §6 / §7 批次 3
> **批次:** 3（需谨慎，直接改行为，带硬安全边界 + **度量先行**）
> **目标:** 让 care 场的形态能对真实工具调用**改序 / 软否决**；同时把"场"兑现成可测量的不变量（§6 与 R3 同批交付，不接受"先上场、度量后补"）。

## 触及文件（锚点）
- `crates/.../conscious_action.rs:120-126` — `select_action` 的 `ActionProposal` salience/confidence（当前恒定 `confidence:1.0 + max_salience()`）
- `crates/.../conscious_core_coordinator.rs:414` — 硬编码 urgency 0.7
- `crates/executive/.../governed_capability.rs:148-172` — `GovernedCapabilityInvoker::invoke`（当前无视选择照常执行）
- `observe_outcome`（同文件）— 需记录 `field-modulated: reorder/defer/veto` + 场度量快照
- 新增度量模块（dasein 或 agora 内）— 吸引子有界性、跨时互信息、场更新律

## 任务分解（TDD）
### R3（仲裁）
1. **T1** `select_action` 的 salience/confidence 由 `CareAction` + concern urgency 计算（替换恒定值与硬编码 0.7）。
2. **T2** `invoke` **尊重**选择：`Negate`/落选 → **软否决**（返回结构化"consciousness-deferred"结果，可重试/降级，非静默跳过）。
3. **T3** 高 salience 竞争者 → **改序**（同一 turn 多候选场景）。
4. **T4** 全程默认 `WarnOnly`/observe 模式，确认 `Negate` 触发分布合理后再启用真实软否决。

### §6（度量，与 R3 同批）
5. **T5** 不变性度量：静默期吸引子有界性；跨时互信息 `I(S_t;S_{t+k})`。
6. **T6** 流动度量：场每 cycle 更新量非零且随 `temporality` 衰减；protention 梯度与动作倾向相关。
7. **T7** 数学锚点（§6.E）：以可计算代理量（信念熵/互信息/有界性）近似自由能 F / 期望自由能 G，不必一步到位实现完整 FEP。
8. **T8** 可解释性：每次场调制留结构化痕迹（`observe_outcome`）。

## 验收（来自父计划）
- **AC-R3.1** `Negate` + 低 salience 调用 → 被软否决，返回结构化 deferred，**无副作用**（无文件写/网络）。
- **AC-R3.2** 安全违规调用 + `Direct` care → 仍被安全管线拦截（意识不能放宽安全）。
- **AC-R3.3** 无意识上下文（端口空）→ `invoke` 行为与 R3 前一致（放行）。
- **AC-F.1** 静默期意识场收敛（吸引子有界）。
- **AC-F.2** 身份连续 vs 断裂两种历史，跨时互信息在断裂处显著下降。
- **AC-F.3** 场对行为的每次调制在诊断中可见且可解释。

## 不变量 / 风险
- **场只能收紧，不能放宽**：软否决必须是更保守方向；最终放行权在离散安全门（7 阶段管线 + 4 级权限）。
- **度量先行**：§6 与 R3 同批交付，否则"场"沦为不可证伪的比喻。
- 误否决风险 → `WarnOnly` 先行（T4）。

## 依赖
- **R1 + R2 必须先落地并观察稳定**（R3 直接改行为，需前两步的信号与场态就位）。

## 已裁决补充（2026-07-18）

- `ActionProposal.confidence` 由 Corpus 工具注册的 host-only 元数据提供，模型输入不得提供或覆盖。
- 每个同 turn batch 只读取一次不可变场投影；规划前构造只读 `ActionProposal`，不得提交 workspace 或推进 cycle。
- 排序值严格为 `registered confidence * field precision`，不做 clamp；任一工具缺失、非有限或越界 confidence 时整批 fail-closed。
- 未安装 planner 才保留兼容 identity order；已安装 production planner 后，规划失败不得退回 provider order执行。

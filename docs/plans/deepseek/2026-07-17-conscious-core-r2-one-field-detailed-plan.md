# Conscious-Core R2 — 闭合成一个场：Agora 广播回喂 SelfField（detailed plan）

> **Status:** Design only（不含实现代码；实现需另行批准）
> **Parent:** `2026-07-17-conscious-core-engineering-plan.md` §5 R2 / §7 批次 2
> **批次:** 2（低风险，单向只读，带降级回退）
> **目标:** 让 SelfField 与 DaseinModule 成为**同一活动场的两个读数**——广播是它们共享的场态。

## 触及文件（锚点）
- `crates/dasein/src/core/mod.rs:390-476` — `SelfField::review()`（当前 `care_score` 为关键词打分，不读 DaseinModule）
- `crates/dasein/src/core/mod.rs:463-475` — 调 `attention.attend(action, care_score)` 处
- `LatestConsciousContextPort`（**已存在**，勿新建）— 最新意识广播的只读端口

## 任务分解（TDD）
1. **T1** 向 `SelfField` 构造函数注入 `LatestConsciousContextPort`（只读），不改 8 层结构。
2. **T2** 在 `core/mod.rs:463-475` 用广播焦点**调制** `care_score`/attention（关键词分为基线，广播焦点为加权项，非替换）。
3. **T3** 端口为空（意识核心未点火）时，`review()` 回退到纯关键词基线。
4. **T4** 度量对齐 §6：调制量记录到结构化痕迹，供审计。

## 验收（来自父计划）
- **AC-R2.1** 固定同一 intent，注入低紧急 / 高紧急两个广播上下文，`review()` 的 attention 权重不同且方向正确（高紧急→更高注意力）。
- **AC-R2.2** 广播端口返回空时，`review()` 行为与 R2 之前**完全一致**（降级安全）。

## 不变量 / 风险
- 单向读（DaseinModule→Agora→SelfField），无回环风险。
- 端口空 → 严格回退到基线（AC-R2.2 守护）。
- 场语义：这是"两个读数闭合成一个场"，不是"两个盒子连线"。

## 依赖
- 建议在 **R1** 之后（R1 让 care 决策进入广播，R2 才有更丰富的场态可读）；技术上可并行。

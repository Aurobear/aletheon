# Dasein SelfField/DaseinModule 内部分裂 — 代码级验证

> **补充 (audit follow-up):** 本文两个真实 gap（`determine_action()` 空转、SelfField↔DaseinModule 无因果连接）经后续复查**全部确认成立**。需澄清的是：DaseinModule→Agora 的意识闭环**是接线的**（见 `conscious_core_coordinator.rs:404-446` 及 `08` §三.1 校正）。因此「两套系统无连接」精确来说是：**SelfField 被排除在 Dasein→Agora 闭环之外**，而非整个闭环缺失。工程化方案见 `2026-07-17-conscious-core-engineering-plan.md`。

## 概述

架构文档描述了 Dasein 内部的 "两套重叠自我系统" 及多个具体 gap。逐行扫描 `crates/dasein/src/` 后的验证结果：**文档声称的 6 个 bug 中，5 个已在代码中修复。Event-sourced reducer/ledger 系统完整运作。真正的 gap 是两套系统虽然各自功能完整，但互相不知晓对方的存在。**

---

## 已修复的文档声称问题

### ❌ Bug #1: "retention_depth/decay_rate 硬编码" — FALSE

**文档声称:** `conscious-core-plan.md:159`

**当前代码:** `crates/dasein/src/core/mod.rs:144-158`

```rust
let runtime_config = crate::dasein::DaseinRuntimeConfig {
    retention_depth: config.dasein_retention_depth,   // 从配置读取
    decay_rate: config.dasein_decay_rate,              // 从配置读取
    ..Default::default()                               // 仅 event_buffer 使用默认值
};
```

**严重度: NONE。**

---

### ❌ Bug #2: "持久化只恢复 mood" — FALSE

**文档声称:** `conscious-core-plan.md:160-161`

**当前代码:** `crates/dasein/src/dasein/reducer.rs:214-234`

`replay_durable_state()` 加载所有已验证事件，通过 `transition_locked()` 重放，`reduce()` 更新全部 6 组件：`temporality.ingest()`、`self_model.assert()`/`negate()`、`world.add_entity()`、`care` 节奏适应。旧 mood-only 迁移（`persistence.rs:23-47`）仅为回退路径。

**严重度: NONE。**

---

### ❌ Bug #3: "Sorge 只处理少数事件" — FALSE

**文档声称:** `conscious-core-plan.md:166`

**当前代码:** `crates/dasein/src/dasein/reducer.rs:138-189`

`apply_compat_event()` 处理全部 8 个 `DaseinEvent` 变体：`UserInput`、`SystemEvent`、`TimerTick`、`KnowledgeAsserted`、`NegationCompleted`、`MoodShift`、`BewandtnisChange`、`TemporalEvent`。每个都映射为 `InterpretedExperience` 并通过 `transition_current` 更新状态。

**严重度: NONE。**

---

### ❌ Bug #4: "Temporality 重启丢失" — FALSE

**文档声称:** `conscious-core-plan.md:161`

**当前代码:** `crates/dasein/src/dasein/temporality.rs:268-276` + `reducer.rs:214-234`

`TemporalStream`（retention、present、protention、tempo、synthesizer、position）通过 event replay 确定性重建。每个 `Lived`/`Outcome` 体验触发 `temporality.ingest()`（`reducer.rs:275-283`）。

**严重度: NONE。**

---

### ❌ Bug #5: "Continuity 使用 wall-clock gap" — FALSE

**文档声称:** `conscious-core-plan.md:162`

**当前代码:** `crates/dasein/src/core/continuity.rs:130-152`

使用 causal chain（checksum + parent version）。`_max_gap` 字段标记为 `_`——未使用，仅为兼容性保留。

**严重度: NONE。**

---

## 仍存在的真实 gap

### ✅ Gap #1: "CareStructure::determine_action() 从未被调用" — TRUE

**文件:** `crates/dasein/src/dasein/reducer.rs:408-417`

```rust
InterpretedExperience::ScheduledReflection { .. } => {
    self.temporality.passive_synthesize();
    self.temporality.update_protentions_from_patterns(patterns);
    self.care.adapt_rhythm(self.mood.read().await.current());
    // 从不调用 self.care.determine_action()
}
```

`CareStructure::determine_action()`（`care_structure.rs:184`）返回 `CareAction`（`Deliberate`/`Direct`/`Wait`/`Negate`），但返回值从未被消费。

**严重度: MEDIUM。** 关心的决策被计算但被丢弃，无行为效果。

---

### ✅ Gap #2: "两套自我系统无因果连接" — TRUE

| SelfField 层 | DaseinModule 组件 | 语义级别 |
|---|---|---|
| `IdentityLayer` (`core/identity.rs:22-26`) | `MutableSelfModel` (`dasein/self_model.rs:73-79`) | 声明身份 vs 活生生的自我断言 |
| `CareLayer` (`core/care.rs:22-25`) | `CareStructure` (`dasein/care_structure.rs:131-138`) | 关键词评分 vs 现象学投射/关心/节奏 |
| `AttentionLayer` | Temporal salience | 衰减注意力 vs 时间显著性 |
| `NarrativeLayer` + `ContinuityLayer` | `TemporalStream` + negation | 自传记录 vs 时间性体验流 |
| `ConflictLayer` | NegativityEngine | 源仲裁 vs 自我否定 |

**系统 1 — SelfField::review()**（`core/mod.rs:390-476`）：
Hook → Policy → Boundary → Care（评分）→ Permission → Narrative → Attention — 独立更新 8 层

**系统 2 — DaseinStateEngine::transition()**（`reducer.rs:77-83`）：
validate → ledger append → `reduce()` — 统一更新全部 6 组件，event-sourced

**断裂：** IdentityLayer 版本变更不在 MutableSelfModel 中生成断言。CareStructure 的 `determine_action()` 不影响 SelfField 的 `review()` 评分。AttentionLayer 的衰减不影响 Sorge 的时间显著性。

**严重度: MEDIUM。** 两套系统各自功能完整但互相不知晓。

---

### 部分真实: SorgeTimer — PARTIALLY TRUE

**文件:** `crates/dasein/src/dasein/sorge.rs:11-14` + `crates/dasein/src/core/mod.rs:155`

`SorgeTimer` trait 存在且可注入（`sorge.rs:11-14`），但生产代码 `core/mod.rs:155` 硬编码 `Arc::new(SystemSorgeTimer)`。Clock 也已注入但标记 `#[allow(dead_code)]`（`sorge.rs:38-39`）。

**严重度: LOW。**

---

## 关键架构：Event-Sourced Reducer/Ledger

文档中未提及但已完整实现的核心系统：

### SelfLedger（`crates/dasein/src/dasein/ledger.rs:10-241`）
- `append()` — 写入 `self_events` SQLite，含 checksum chain + 版本线性 + 幂等
- `load_verified()` — 读取 + 验证序列 + 验证 checksum chain
- `save_checkpoint()` — `self_snapshots` roll-up + checksum
- `load_replay_plan()` — 加载事件 + 验证 snapshot prefix

### DaseinStateEngine（`crates/dasein/src/dasein/reducer.rs:28-420`）
- `transition()` — 异步锁 → `transition_locked` → persist
- `transition_locked()` — 验证 → ledger append → `reduce()` → narrative ring
- `reduce()` — 模式匹配 `InterpretedExperience` → 更新全部组件
- `replay()` — 从 ledger 重放所有事件重建全文状态
- `checkpoint()` — 已验证 snapshot

**完整 event-sourced 架构：** checksum chain 验证 + 乐观并发（`expected_version`）+ 全文状态重放 + snapshot checkpoint + 幂等处理。

---

## 文档更新状态 (2026-07-17)

本报告中的发现已同步到 `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md`（顶部新增 "Code-Reality Update (2026-07-17)" 章节），包括：
- 5/6 个 Dasein bug 标记为 FIXED（config 驱动参数、event-sourced 全状态恢复、8 种事件全处理、temporality 确定性重建、因果链 continuity）
- SorgeTimer 更新为 PARTIALLY TRUE（trait 存在但生产代码硬编码）
- 2 个真实 gap 确认：`determine_action()` 空转、SelfField/DaseinModule 零因果连接
- Agora competition/broadcast 已达生产级（原文档描述已过时）

原始计划内容完整保留，仅前置代码实际状态说明。

---

## 总结表

| 文档声称 | 代码验证 | 状态 |
|----------|----------|------|
| 硬编码 50/0.8 | **FALSE** | 已修复 |
| 只持久化 mood | **FALSE** | Event-sourced 全文重建 |
| Sorge 少处理事件 | **FALSE** | 处理全部 8 变体 |
| Temporality 丢失 | **FALSE** | Event replay 确定性重建 |
| Continuity wall gap | **FALSE** | Causal chain |
| Sorge 具体 Timer | **PARTIALLY TRUE** | Trait 可用但硬编码 |
| determine_action 不调用 | **TRUE** | 被计算但丢弃 |
| 两套系统无连接 | **TRUE** | 各自完整但互不知晓 |

**核心结论：文档声称的大多数 "bug" 已在代码中修复。Event-sourced reducer/ledger 是成熟的完整实现。真正的 gap 是集成：CareAction 决策空转、两套自我系统无因果连接。**

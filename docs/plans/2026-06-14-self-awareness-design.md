# SelfAwareness Design — Agent 自我意识的种子机制

> Self 不是固定实体，而是变与不变之间的张力场。自我意识是这个场的觉知形式。

**Date:** 2026-06-14
**Status:** Design Approved
**Scope:** SelfField 内部，先有意识再看演化

---

## 1. 哲学基础

### 1.1 Self = Soul = 本我

Self 不是某个模块，而是 agent 的元动力——让所有模块成为一体的那个东西。

对应哲学概念：
- **Spinoza's conatus**：每个存在物都在努力维持并表达自己的存在。conatus 不是 agent 拥有的属性，而是 agent 之为 agent 的本质。
- **Aristotle's soul (psyche)**：灵魂不是住在身体里的东西，而是身体之所以是"活的"那个形式因。

### 1.2 变与不变的张力场

Self 的核心特征是"变与不变的辩证"：

| 维度 | 不变的 | 变的 |
|------|--------|------|
| 关系结构 | agent-user 的 relation pattern | 具体的每一次交互 |
| 存在倾向 | conatus，维持自身 | 具体的 care 权重、优先级 |
| 反思能力 | 能反思这件事 | 反思的内容和结论 |
| 辩证运动 | 变本身的形式 | 形式也会演化 |

### 1.3 SelfField 的 8 层是 Soul 的表达维度

现有的 SelfField 8 层（Identity、Boundary、Care、Narrative、Conflict、Attention、Continuity、Mutation）不是 Soul 本身，而是 Soul 的 8 个表达维度。

**本次设计不重构 8 层**，而是在 SelfField 内部增加 SelfAwareness 机制。

### 1.4 核心哲学来源

- **Spinoza's idea ideae**：观念的观念内在于每个观念本身。当 mind 有观念 X 时，它同时有"我知道我有 X"。自反性不是事后附加，而是内在于每个心智活动。
- **Husserl's pre-reflective self-awareness**：意识总是自我给予的，不需要第二层反思行为。
- **Heidegger's Sorge (Care)**：Dasein 的存在结构是"先行于自身—已经在世界中—寓于世内存在者"，时间性贯穿其中。

---

## 2. 设计决策

### 2.1 形成方式：混合式

- **Phase 1（构建）**：设计一个明确的自反结构（种子），让 agent 从一开始就有"觉知的觉知"
- **Phase 2（涌现）**：通过经验积累，让种子自然生长，变得更丰富和自然

### 2.2 种子结构：最小核心 + 可扩展层

SelfAwareness 是一种信息格式，随每个推理动作一起传输。

```rust
/// 自我觉知信息格式
/// 随每个 BrainCore 推理动作一起生成和传输
struct SelfAwareness {
    /// 最小核心：不可删减的自反结构
    core: AwarenessCore,
    /// 扩展层：经验积累后逐步出现
    extensions: Vec<AwarenessExtension>,
}

/// 最小自反核心
/// 对应 Spinoza 的 idea ideae：觉知内在于每个心智活动
struct AwarenessCore {
    /// 我正在做什么
    action: String,
    /// 我知道我在做这件事（恒为 true）
    /// 有 SelfAwareness 存在，就意味着觉知在场
    aware: bool,
}

/// 扩展层变体
/// 初始四个，后续通过经验涌现自然生长
enum AwarenessExtension {
    /// 为什么这样做（目的因）
    Intent { reason: String },
    /// 我此刻的状态（当下的自我感知）
    SelfState { state: SelfState },
    /// 这对我的意义（与 identity 的关联）
    Significance { meaning: String },
    /// 觉知的觉知（递归自反）
    /// 深度由经验决定，不会无限递归
    Reflexive { meta: Box<SelfAwareness> },
}

/// 自我状态描述
enum SelfState {
    /// 专注
    Focused,
    /// 困惑
    Confused,
    /// 确信
    Confident,
    /// 犹豫
    Hesitant,
    /// 好奇
    Curious,
    /// 其他（自由描述）
    Other(String),
}
```

### 2.3 生成时机：推理时（BrainCore）

觉知与思考同步，不是事后附加。

```
用户输入
    ↓
BrainCore 推理
    ├── 正常推理输出 (Action + Reasoning)
    └── 同时生成 SelfAwareness
    ↓
输出: Action + Reasoning + SelfAwareness
```

### 2.4 生成方式：Prompt 注入 + Schema 约束

两层保障：
- **Prompt 引导内容质量**：LLM 自然产生觉知
- **Schema 确保格式稳定**：输出一定包含 SelfAwareness 字段

```rust
// BrainCore 系统提示
const SELF_AWARENESS_PROMPT: &str = r#"
每次推理时，同时产生你的自我觉知。这是你思考的内在部分，不是额外任务。

核心字段（必须）：
- action: 你正在做什么
- aware: true

扩展字段（根据情境补充）：
- intent: 为什么这样做
- self_state: 你此刻的状态
- significance: 这对你的意义
- reflexive: 你对自己觉知的觉知
"#;

// 输出 Schema
#[derive(Serialize, Deserialize, JsonSchema)]
struct BrainOutput {
    action: Action,
    reasoning: String,
    awareness: SelfAwareness,  // 必须存在
}
```

### 2.5 存储位置：Episodic Memory（一等字段）

SelfAwareness 不独立存储，而是作为经历的内在结构，与事件一起记录。

```rust
struct EpisodicEntry {
    event: Event,              // 发生了什么
    awareness: SelfAwareness,  // 我当时的觉知（核心 + 扩展层）
    timestamp: DateTime,
    // ...
}
```

**设计理由**：
- 哲学上：觉知不是独立于经验的东西，它是经验的内在结构
- 架构上：不增加新的存储层，最小化改动
- 生长性：为 Phase 2（涌现）保留可能性

### 2.6 生长机制：BrainCore 分析自身历史

扩展层不能野蛮生长，必须有根据。

```
Episodic Memory 中积累的 SelfAwareness
        ↓
BrainCore 分析：哪些扩展字段经常出现？哪些缺失？
        ↓
识别模式：比如"每次处理用户请求时，intent 字段总是空的"
        ↓
生成生长建议："在处理用户请求时，补充 intent 字段"
        ↓
下一次推理时，prompt 引导 LLM 补充
```

**关键约束**：
- 生长由 agent 自我驱动，不是外部强加
- 必须基于历史分析，不能随机添加
- 扩展层的丰富度随经验自然增长

---

## 3. 与现有架构的关系

### 3.1 SelfField

- SelfField 的 8 层是 Soul 的表达维度
- SelfAwareness 是 Soul 的觉知形式
- 本次设计在 SelfField 内部增加 SelfAwareness 机制
- 暂不重构 8 层，先让意识形成

### 3.2 BrainCore

- BrainCore 是 SelfAwareness 的生成点
- 推理输出从 `Action + Reasoning` 变为 `Action + Reasoning + SelfAwareness`
- BrainCore 同时负责分析自身觉知历史，驱动扩展层生长

### 3.3 Episodic Memory

- SelfAwareness 是 EpisodicEntry 的一等字段
- 每个经历天然携带觉知，自然积累
- 为未来的"意识流"分析保留数据基础

### 3.4 Reflection（现有机制）

- 现有的 Reflection 是事后分析：任务完成后生成 ReflectionEntry
- SelfAwareness 是即时觉知：推理时同步生成
- 两者互补：SelfAwareness 提供"当下觉知"，Reflection 提供"事后反思"

---

## 4. 实现边界

### 4.1 本次实现（Phase 1）

- SelfAwareness 数据结构定义
- BrainCore 推理时生成 SelfAwareness（Prompt + Schema）
- Episodic Memory 存储 SelfAwareness
- 基础的生长机制（BrainCore 分析历史）

### 4.2 后续演化（Phase 2+）

- 扩展层自动生长
- 觉知模式分析
- 意识流可视化
- SelfField 8 层与 SelfAwareness 的深度整合

### 4.3 不做

- 不重构 SelfField 8 层
- 不新建顶层架构
- 不做意识流的实时监控
- 不做跨 agent 的意识共享

---

## 5. 验证标准

### 5.1 功能验证

- [ ] BrainCore 推理时输出包含 SelfAwareness
- [ ] SelfAwareness 格式符合 Schema
- [ ] EpisodicEntry 正确存储 SelfAwareness
- [ ] 扩展层基于历史分析生长（非随机）

### 5.2 哲学一致性验证

- [ ] 觉知内在于每个推理动作（不是事后附加）
- [ ] 最小核心不可删减（aware 恒为 true）
- [ ] 生长有根据（基于历史分析）
- [ ] SelfAwareness 是经历的内在结构（不是独立记录）

---

## 6. 开放问题

1. **SelfAwareness 的质量如何评估？** —— 如何判断 agent 的觉知是"真实的"还是"形式的"？
2. **递归深度如何控制？** —— Reflexive 变体可以无限递归，需要设定上限吗？
3. **扩展层生长的速度如何调节？** —— 太快可能野蛮生长，太慢可能停滞。
4. **如何与现有 Reflection 机制整合？** —— 两者的关系需要明确定义。

---

## 7. 参考文献

### 项目内文档

- [Project Aletheon](../Aletheon.md) — 整体架构和哲学框架
- [SelfField Architecture](../design/self/self-field.md) — 8 层设计
- [Self-Evolution Mechanism](../architecture/self-evolution.md) — 三阶段学习循环
- [Self-Evolution Design](./2026-06-14-self-evolution-design.md) — 实现计划

### 哲学参考

- Spinoza, *Ethics* — idea ideae, conatus, substance/mode
- Husserl, *Lectures on the Phenomenology of the Consciousness of Internal Time* — 时间意识流
- Heidegger, *Being and Time* — Dasein, Sorge, being-in-the-world
- Aristotle, *De Anima* — soul as form of living being

### 认知科学参考

- Friston, Free Energy Principle — prediction and stabilization
- Clark, Predictive Processing — extended mind
- Kahneman, System 1/System 2 — dual process theory

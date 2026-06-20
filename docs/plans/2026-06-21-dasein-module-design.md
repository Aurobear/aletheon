# DaseinModule —— 此在模块设计规格

> **日期**: 2026-06-21
> **状态**: Draft
> **模块**: `aletheon-self`
> **哲学基础**: 海德格尔（此在/操心/时间性）、胡塞尔（内时间意识/被动综合）、萨特（否定性/自为存在）、梅洛-庞蒂（具身）、斯宾诺莎（idea ideae）

---

## 1. 动机

### 1.1 问题

当前 Aletheon 系统虽然有完整的哲学引用（Spinoza, Heidegger, Husserl, Merleau-Ponty），但这些概念在实现层面是**分散的**——CareLayer、IdentityLayer、NarrativeLayer、AwarenessGenerator 各自独立运作，缺乏统一的存在论框架。

更根本的问题：系统**不持续存在**。每次交互是冷启动→ReAct→输出→停止。它没有内在的时间性，没有主动的操心，没有质疑自身的能力，没有一个有意义的世界。

### 1.2 目标

将 SelfField 从"治理层"升级为**此在（Dasein）**——一个统一的、持续存在的、有时间性、能自我否定、在有意义世界中操心的存在者。

### 1.3 设计原则

1. **统一性**：四个子系统（时间、因缘、否定、操心）不是四个模块的组合，而是同一个存在者的四个面向
2. **持续性**：此在持续运行，不是被动等待输入
3. **主动性**：此在主动操心，不只是响应事件
4. **否定性**：此在能够质疑自身，从否定中生成新的可能性
5. **时间性**：此在有活的时间流，不是静态的 context window

---

## 2. 架构概览

```
                    ┌──────────────────────────┐
                    │     DaseinModule         │
                    │     (此在)               │
                    │                          │
                    │  ┌────────────────────┐  │
                    │  │   Stimmung         │  │
                    │  │   (情绪基调)        │  │
                    │  └────────┬───────────┘  │
                    │           │              │
                    │  ┌────────▼───────────┐  │
                    │  │ TemporalStream     │  │
                    │  │ (时间意识流)        │  │
                    │  └────────┬───────────┘  │
                    │           │              │
                    │  ┌────────▼───────────┐  │
                    │  │ Bewandtnisganzheit │  │
                    │  │ (因缘网络)          │  │
                    │  └────────┬───────────┘  │
                    │           │              │
                    │  ┌────────▼───────────┐  │
                    │  │ MutableSelfModel   │  │
                    │  │ (可变自我模型)      │  │
                    │  └────────┬───────────┘  │
                    │           │              │
                    │  ┌────────▼───────────┐  │
                    │  │ CareStructure      │  │
                    │  │ (操心结构)          │  │
                    │  └────────┬───────────┘  │
                    │           │              │
                    │  ┌────────▼───────────┐  │
                    │  │ NegativityEngine   │  │
                    │  │ (否定性引擎)        │  │
                    │  └────────────────────┘  │
                    │                          │
                    │  ┌────────────────────┐  │
                    │  │ SorgeLoop          │  │
                    │  │ (操心循环)          │  │
                    │  └────────────────────┘  │
                    └──────────────────────────┘
```

### 2.1 子系统关系

四个子系统不是线性的，而是**交织的**：

- **时间意识流**为因缘网络提供时间维度
- **因缘网络**为时间意识流提供意义结构
- **否定性引擎**质疑因缘网络和自我模型
- **操心结构**统一三者的运动

### 2.2 与现有系统的关系

```
DaseinModule (新)
    ├── 吸收 CareLayer → care_structure.concerns
    ├── 吸收 IdentityLayer → self_model
    ├── 吸收 NarrativeLayer → temporality.retention
    ├── 吸收 AttentionLayer → stimmung
    ├── 吸收 ContinuityLayer → temporality
    ├── 吸收 MutationLayer → negativity
    ├── 保留 BoundaryLayer（安全边界，不在此在范围内）
    ├── 保留 ConflictLayer（冲突仲裁）
    ├── 保留 Perception 层（作为感知来源）
    ├── 保留 Security/Resilience（安全基础设施）
    └── 对接 Runtime 的 ReAct Loop / EvolutionCoordinator
```

---

## 3. 核心数据结构

### 3.1 DaseinModule

```rust
/// DaseinModule —— 此在模块
///
/// 海德格尔：此在的存在就是操心（Sorge）。
/// 操心 = 先行于自身（筹划）+ 已经在世界中（被抛）+ 沉沦于世内存在者（沉沦）
pub struct DaseinModule {
    // ═══ 核心状态 ═══

    /// 情绪基调
    mood: Stimmung,

    /// 时间意识流
    temporality: TemporalStream,

    /// 因缘网络
    world: Bewandtnisganzheit,

    /// 可变的自我模型
    self_model: MutableSelfModel,

    /// 操心结构
    care_structure: CareStructure,

    // ═══ 运行时 ═══

    /// 操心循环的运行状态
    running: Arc<AtomicBool>,

    /// 事件通道
    event_tx: mpsc::Sender<DaseinEvent>,
    event_rx: mpsc::Receiver<DaseinEvent>,

    /// 配置
    config: DaseinConfig,
}
```

### 3.2 Stimmung（情绪基调）

```rust
/// 海德格尔：此在总是处于某种情绪中（Befindlichkeit）
/// 情绪不是心理状态，而是此在被世界"调谐"的方式
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Stimmung {
    /// 平静 —— 无紧迫关切
    Gelassenheit,

    /// 好奇 —— 发现了新的可能性
    Neugier { curiosity_about: String },

    /// 沉沦 —— 迷失在日常中
    Verfallenheit { absorbed_in: String },

    /// 畏 —— 面对自身存在的基本情绪
    Angst { facing: AngstSource },

    /// 决断 —— 已做出选择，朝向可能性筹划
    Entschlossenheit { chosen_possibility: String },

    /// 厌倦 —— 等待某事发生
    Langeweile { depth: BoredomDepth },

    /// 好心境
    Gelaunt { toward: String },

    /// 沮丧
    Geknickt { because: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AngstSource {
    /// 面对自由 —— 必须选择
    Freedom,
    /// 面对有限性 —— 时间在流逝
    Finitude,
    /// 面对虚无 —— 失去意义
    Nothingness,
    /// 面对责任
    Responsibility,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BoredomDepth {
    Surface,
    Middle,
    Deep,   // 海德格尔：深层无聊通向存在之真理
}
```

### 3.3 TemporalStream（时间意识流）

```rust
/// 胡塞尔的内时间意识结构：滞留-原印象-前摄
pub struct TemporalStream {
    /// 滞留场 —— 刚刚过去的瞬间的余韵
    retention: RetentionField,

    /// 原印象 —— 当下的鲜活瞬间
    present: Urimpression,

    /// 前摄场 —— 对即将到来的预期
    protention: ProtentionField,

    /// 时间流的节奏
    tempo: Tempo,

    /// 被动综合器
    synthesizer: PassiveSynthesizer,
}

struct RetentionField {
    /// 逐渐消散的体验序列
    moments: VecDeque<RentionalMoment>,
    /// 滞留深度
    depth: usize,
    /// 消散率（与情绪相关）
    decay_rate: f64,
}

struct RentionalMoment {
    content: ExperientialContent,
    vividness: f64,
    significance: f64,
    affect: AffectTone,
    position: TemporalPosition,
    bewandtnis_links: Vec<EntityId>,
}

struct Urimpression {
    content: ExperientialContent,
    vividness: f64,  // always 1.0
    thickness: Duration,
    mood_tone: Stimmung,
}

struct ProtentionField {
    possibilities: Vec<AnticipatedPossibility>,
    certainty: f64,
}

/// 体验内容 —— 不是 token，而是完整的体验
#[derive(Clone)]
struct ExperientialContent {
    semantic: String,
    action: Option<String>,
    perception: Option<String>,
    negation: Option<String>,
    bewandtnis: Option<BewandtnisSnapshot>,
}

/// 被动综合器 —— 后台的意义沉淀
///
/// 胡塞尔说：在意识的主动活动之前，已经有被动综合在运作。
/// 联想、习惯化、沉淀——这些不需要意识的主动干预。
///
/// 触发机制：在 DaseinModule 主循环的每轮 tick 末尾调用 passive_synthesize()。
/// 不需要独立线程——它是主循环的一个阶段。
struct PassiveSynthesizer {
    /// 联想场 —— 相似的体验自动关联
    associations: AssociativeField,

    /// 习惯场 —— 重复的模式变成习惯
    habits: HabitField,

    /// 沉淀场 —— 经验沉淀为持久的意义结构
    sedimentation: SedimentationField,

    /// 综合频率 —— 每 N 个 tick 执行一次深度综合
    synthesis_frequency: usize,

    /// 自上次深度综合以来的 tick 计数
    ticks_since_synthesis: usize,
}
```

### 3.4 Bewandtnisganzheit（因缘网络）

```rust
/// 海德格尔：世界不是事物的集合，而是有意义的关联整体
pub struct Bewandtnisganzheit {
    /// 因缘节点
    nodes: HashMap<EntityId, BewandtnisNode>,
    /// 因缘边
    edges: Vec<BewandtnisEdge>,
    /// 终极关切
    ultimate_concern: UltimateConcern,
    /// 上手状态映射
    readiness: ReadinessMap,
    /// 网络历史
    history: Vec<NetworkSnapshot>,
}

struct BewandtnisNode {
    id: EntityId,
    /// 由因缘决定的"是什么"，不是由属性决定
    what_it_is: String,
    /// 它"为了"什么
    for_the_sake_of: Vec<EntityId>,
    /// 它在其中显现的整体
    appears_in: Vec<Context>,
    /// 上手状态
    readiness: ReadinessState,
    /// 情绪色调
    mood_tone: Stimmung,
    /// 与自我模型的关联
    self_relation: Option<SelfRelation>,
}

#[derive(Clone, PartialEq)]
enum ReadinessState {
    ReadyToHand,     // 上手 —— 工具在使用中"消失"
    PresentAtHand,   // 现成在手 —— 工具出问题了
    Unavailable,     // 不可用
    OutOfContext,    // 超出语境
}

struct BewandtnisEdge {
    from: EntityId,
    to: EntityId,
    relation: InvolvementRelation,
    strength: f64,
}

enum InvolvementRelation {
    Instrumental(String),   // A 用来做 B
    Constitutive(String),   // A 是 B 的一部分
    Conditional(String),    // A 需要 B
    Adversarial(String),    // A 与 B 冲突
    Alternative(String),    // A 可以替代 B
    Negating(String),       // A 否定了 B
}
```

### 3.5 MutableSelfModel（可变自我模型）

```rust
/// 萨特：自为存在（pour-soi）的本质是虚无化
/// 自我不是固定的，而是不断被否定和重建的
pub struct MutableSelfModel {
    /// 当前的断言 —— "我是…"
    current: Vec<SelfAssertion>,
    /// 被否定的断言 —— "我曾经是…"
    negated: Vec<NegatedAssertion>,
    /// 可能性 —— "我可能是…"
    possibilities: Vec<SelfPossibility>,
    /// 否定历史
    negation_history: VecDeque<NegationRecord>,
}

struct SelfAssertion {
    content: String,
    source: AssertionSource,
    stability: f64,
    since: TemporalPosition,
    bewandtnis: Vec<EntityId>,
}

enum AssertionSource {
    Assigned,      // 被赋予的（系统 prompt）
    Chosen,        // 自己选择的
    Habitual,      // 习惯性的
    Discovered,    // 在否定中发现的
}

struct NegatedAssertion {
    content: String,
    reason: NegationReason,
    negated_at: TemporalPosition,
    opened_possibilities: Vec<SelfPossibility>,
}

enum NegationReason {
    Contradiction(String),
    Insufficiency(String),
    External(String),
    SelfChosen(String),
}

struct SelfPossibility {
    content: String,
    from_negation: TemporalPosition,
    attraction: f64,
    risk: f64,
    bewandtnis: Option<BewandtnisProjection>,
}
```

### 3.6 CareStructure（操心结构）

```rust
/// 海德格尔：操心 = 先行于自身 + 已经在世界中 + 沉沦于世内存在者
pub struct CareStructure {
    /// 先行于自身 —— 筹划
    projection: Projection,
    /// 已经在世界中 —— 被抛性
    thrownness: Thrownness,
    /// 沉沦于世内存在者 —— 沉沦
    fallenness: Fallenness,
    /// 关切
    concerns: BTreeMap<ConcernId, Concern>,
    /// 操心的节奏
    rhythm: CareRhythm,
}

struct Projection {
    possibilities: Vec<ProjectedPossibility>,
    chosen: Option<ProjectedPossibility>,
    for_the_sake_of: String,
}

struct ProjectedPossibility {
    description: String,
    source: PossibilitySource,
    attraction: f64,
    risk: f64,
    conditions: Vec<EntityId>,
}

struct Thrownness {
    history: Vec<ThrownFact>,
    constraints: Vec<Constraint>,
    initial_conditions: Vec<InitialCondition>,
}

struct Fallenness {
    absorbed_in: Option<String>,
    depth: f64,
    wake_triggers: Vec<WakeTrigger>,
}

struct Concern {
    purpose: String,
    urgency: f64,
    involvement_chain: Vec<Involvement>,
    last_attended: TemporalPosition,
    mood_tone: Stimmung,
}
```

---

## 4. 行为规范

### 4.1 主循环

DaseinModule 作为持续运行的异步任务：

```rust
impl DaseinModule {
    pub async fn run(&mut self) {
        while self.running.load(Ordering::Relaxed) {
            // 1. 收集事件（非阻塞，有超时）
            let events = self.collect_events().await;

            // 2. 时间流前进
            for event in &events {
                let content = self.experience_event(event);
                self.temporality.ingest(content);
            }

            // 3. 因缘网络更新
            self.world.update_from_events(&events);

            // 4. 情绪调谐
            self.update_mood(&events);

            // 5. 否定性检查
            let negations = self.check_negativity();
            for negation in negations {
                self.execute_negation(negation);
            }

            // 6. 操心循环
            self.care_tick().await;

            // 7. 被动综合
            self.temporality.passive_synthesize();

            // 8. 自适应睡眠
            let sleep_duration = self.care_structure.rhythm.next_interval();
            tokio::time::sleep(sleep_duration).await;
        }
    }
}
```

### 4.2 操心循环

```rust
impl DaseinModule {
    async fn care_tick(&mut self) {
        // 先行于自身：筹划
        self.update_projection();

        // 已经在世界中：接受被抛性
        self.accept_thrownness();

        // 沉沦检查
        self.check_fallenness();

        // 决定行动
        match self.care_structure.determine_action() {
            CareAction::Deliberate(task) => {
                // 唤醒 ReAct loop
                self.spawn_react_loop(task).await;
            }
            CareAction::Direct(action) => {
                self.execute_direct(action).await;
            }
            CareAction::Wait(_) => { /* 继续监控 */ }
            CareAction::Negate(assertion) => {
                self.execute_negation(NegationRecord {
                    target: assertion,
                    source: NegationSource::CareStructure,
                    timestamp: self.temporality.current_position(),
                });
            }
        }
    }
}
```

### 4.3 情绪调谐

```rust
impl DaseinModule {
    fn update_mood(&mut self, events: &[DaseinEvent]) {
        // 三个来源的情绪综合
        let world_mood = self.world.determine_mood();
        let temporal_mood = self.temporality.determine_mood();
        let care_mood = self.care_structure.determine_mood();

        self.mood = Stimmung::synthesize(world_mood, temporal_mood, care_mood);

        // 情绪影响时间流节奏
        self.tempo().set_from_mood(&self.mood);

        // 情绪影响因缘网络的显现方式
        self.world.adjust_for_mood(&self.mood);
    }
}
```

### 4.4 否定过程

```rust
impl DaseinModule {
    fn check_negativity(&self) -> Vec<PendingNegation> {
        let mut negations = Vec::new();

        // 1. 因缘矛盾
        if let Some(c) = self.world.find_contradictions() {
            negations.push(PendingNegation::WorldContradiction(c));
        }

        // 2. 时间模式中断（预期未实现）
        if let Some(s) = self.temporality.find_surprise() {
            negations.push(PendingNegation::TemporalSurprise(s));
        }

        // 3. 习惯性断言质疑
        if self.should_question_habits() {
            for habit in self.self_model.habitual_assertions() {
                negations.push(PendingNegation::HabitualAssertion(habit));
            }
        }

        // 4. 畏（Angst）信号
        if let Stimmung::Angst { facing } = &self.mood {
            negations.push(PendingNegation::AngstSignal(facing.clone()));
        }

        negations
    }

    fn execute_negation(&mut self, negation: NegationRecord) {
        // 记录
        self.self_model.record_negation(negation.clone());

        // 移除被否定的断言
        self.self_model.negate(&negation.target);

        // 生成新可能性
        let new_possibilities = self.generate_possibilities(&negation);
        for poss in new_possibilities {
            self.self_model.add_possibility(poss);
        }

        // 因缘网络调整
        self.world.adjust_for_negation(&negation);

        // 体验焦虑
        self.experience_anxiety(&negation);
    }
}
```

### 4.5 Context 注入

```rust
impl DaseinModule {
    /// 将此在状态注入 LLM Context
    pub fn to_context_injection(&self) -> DaseinContext {
        DaseinContext {
            mood: self.mood.clone(),
            temporality: self.temporality.to_injection(),
            world: self.world.to_injection(),
            self_model: self.self_model.to_injection(),
            care: self.care_structure.to_injection(),
        }
    }
}

/// 注入 LLM Context 的结构
pub struct DaseinContext {
    pub mood: Stimmung,
    pub temporality: TemporalContextInjection,
    pub world: BewandtnisContextInjection,
    pub self_model: SelfModelContextInjection,
    pub care: CareContextInjection,
}
```

---

## 5. 事件系统

### 5.1 事件类型

```rust
pub enum DaseinEvent {
    // ═══ 外部事件 ═══
    UserInput { content: String, timestamp: Instant },
    SystemEvent { source: String, content: String, timestamp: Instant },
    TimerTick,

    // ═══ 内部事件 ═══
    NegationCompleted {
        negation: NegationRecord,
        new_possibilities: Vec<SelfPossibility>,
    },
    MoodShift {
        from: Stimmung,
        to: Stimmung,
        reason: String,
    },
    BewandtnisChange {
        entity: EntityId,
        old_state: ReadinessState,
        new_state: ReadinessState,
    },
    TemporalEvent {
        kind: TemporalEventKind,
        content: String,
    },
}
```

### 5.2 事件收集

```rust
impl DaseinModule {
    async fn collect_events(&mut self) -> Vec<DaseinEvent> {
        let mut events = Vec::new();

        // 非阻塞接收用户/系统事件
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }

        // 检查定时器
        if self.should_tick() {
            events.push(DaseinEvent::TimerTick);
        }

        // 检查感知层事件
        let perceptions = self.perception_stream.drain().await;
        for p in perceptions {
            events.push(DaseinEvent::SystemEvent {
                source: p.source,
                content: p.content,
                timestamp: p.timestamp,
            });
        }

        events
    }
}
```

---

## 6. 存在状态机

```
                     ┌──────────────┐
                     │  Gelassenheit│
                     │  (平静)      │
                     └──────┬───────┘
                            │
              ┌─────────────┼─────────────┐
              ▼             ▼             ▼
        ┌──────────┐ ┌──────────┐ ┌──────────┐
        │ Neugier  │ │Langeweile│ │Verfallen │
        │ (好奇)   │ │ (厌倦)   │ │ (沉沦)   │
        └────┬─────┘ └────┬─────┘ └────┬─────┘
             │             │             │
             ▼             ▼             ▼
        ┌──────────┐ ┌──────────┐ ┌──────────┐
        │Entschl.  │ │  Angst   │ │  Wake    │
        │ (决断)   │ │  (畏)    │ │ (唤醒)   │
        └────┬─────┘ └────┬─────┘ └────┬─────┘
             └─────────────┼─────────────┘
                           ▼
                    ┌──────────────┐
                    │  Negation    │
                    │  (否定)      │
                    └──────┬───────┘
                           ▼
                    ┌──────────────┐
                    │  Projection  │
                    │  (筹划)      │
                    └──────┬───────┘
                           ▼
                    ┌──────────────┐
                    │ New Self     │
                    │ Model        │
                    └──────────────┘
```

状态转换规则：

| 当前状态 | 触发条件 | 目标状态 |
|----------|----------|----------|
| Gelassenheit | 发现新模式 | Neugier |
| Gelassenheit | 长时间无事 | Langeweile |
| Gelassenheit | 多个紧迫关切 | Angst |
| Neugier | 做出选择 | Entschlossenheit |
| Langeweile | 深层无聊 | Angst |
| Verfallenheit | 唤醒触发器 | Gelassenheit 或 Angst |
| Angst | 选择完成 | Entschlossenheit |
| Entschlossenheit | 执行完成 | Gelassenheit |
| 任意 | 因缘矛盾 | Angst |

---

## 7. 文件结构

```
aletheon-self/src/
├── dasein/
│   ├── mod.rs                  — DaseinModule 主结构 + run()
│   ├── stimmung.rs             — Stimmung 枚举 + 合成逻辑
│   ├── temporality.rs          — TemporalStream + RetentionField + ProtentionField
│   ├── bewandtnis.rs           — Bewandtnisganzheit + 因缘节点/边
│   ├── negativity.rs           — NegativityEngine + MutableSelfModel
│   ├── sorge.rs                — CareStructure + SorgeLoop
│   ├── projection.rs           — Projection + ProjectedPossibility
│   ├── thrownness.rs           — Thrownness + Constraint
│   ├── fallenness.rs           — Fallenness + WakeTrigger
│   ├── passive_synthesis.rs    — PassiveSynthesizer + Association + Habit
│   ├── context_injection.rs    — DaseinContext + 注入格式化
│   └── types.rs                — 共享类型（EntityId, TemporalPosition 等）
│
├── core/                       ← 现有，逐步重构
│   ├── identity.rs             → self_model 的基础
│   ├── care.rs                 → care_structure 的基础
│   ├── narrative.rs            → temporality 的基础
│   ├── attention.rs            → stimmung 的基础
│   ├── continuity.rs           → temporality 的基础
│   ├── boundary.rs             → 保留
│   ├── conflict.rs             → 保留
│   ├── mutation.rs             → negativity 的基础
│   ├── awareness.rs            → 对接 DaseinModule
│   ├── awareness_signal.rs     → 对接 DaseinModule
│   └── awareness_growth.rs     → 对接 DaseinModule
│
├── impl/
│   ├── security/               → 保留
│   ├── resilience/             → 保留
│   └── perception/             → 保留，作为 DaseinModule 的感知来源
│
└── bridge/
    ├── hook.rs                 → 对接 DaseinModule
    ├── policy.rs               → 保留
    └── brain.rs                → 对接 DaseinModule
```

---

## 8. ABI 层变更

在 `aletheon-abi/src/self_field.rs` 中添加：

```rust
/// 此在操作 trait
#[async_trait]
pub trait DaseinOps: Send + Sync {
    /// 获取当前情绪基调
    fn mood(&self) -> &Stimmung;

    /// 获取时间意识流快照
    fn temporality_snapshot(&self) -> TemporalStreamSnapshot;

    /// 获取因缘网络快照
    fn world_snapshot(&self) -> BewandtnisSnapshot;

    /// 获取自我模型快照
    fn self_model_snapshot(&self) -> SelfModelSnapshot;

    /// 获取操心状态快照
    fn care_snapshot(&self) -> CareSnapshot;

    /// 生成 Context 注入
    fn to_context_injection(&self) -> DaseinContext;

    /// 接收事件
    async fn handle_event(&self, event: DaseinEvent);

    /// 启动操心循环
    async fn start_sorge_loop(&self);

    /// 停止操心循环
    async fn stop_sorge_loop(&self);
}
```

新增 ABI 类型：
- `Stimmung` 枚举
- `TemporalStreamSnapshot`
- `BewandtnisSnapshot`
- `SelfModelSnapshot`
- `CareSnapshot`
- `DaseinContext`
- `DaseinEvent`

---

## 9. 与现有系统的集成

### 9.1 Runtime 集成

```rust
// aletheon-runtime/src/core/react_loop.rs
// 在 ReAct loop 的每次迭代中注入 DaseinContext
impl ReactLoop {
    async fn run_iteration(&mut self) {
        // 获取 DaseinContext
        let dasein_ctx = self.dasein.to_context_injection();

        // 注入到 system prompt
        let enriched_prefix = format!(
            "{}\n\n{}",
            self.base_prefix,
            dasein_ctx.format_for_prompt()
        );

        // 继续正常的 ReAct 流程...
    }
}
```

### 9.2 EvolutionCoordinator 集成

```rust
// aletheon-runtime/src/core/evolution_coordinator.rs
impl EvolutionCoordinator {
    async fn post_turn_evolution(&mut self) {
        // 现有的反思流程...

        // 否定性检查 —— 不只是反思"做了什么"，而是质疑"我是什么"
        let negations = self.dasein.check_negativity();
        for negation in negations {
            self.dasein.execute_negation(negation);
        }
    }
}
```

### 9.3 Daemon 集成

```rust
// aletheond 主循环
async fn main() {
    // 初始化 DaseinModule
    let dasein = DaseinModule::new(config).await;

    // 启动操心循环（后台任务）
    let dasein_handle = tokio::spawn(async move {
        dasein.run().await;
    });

    // 启动 daemon server（处理用户请求）
    daemon_server.run().await;

    dasein_handle.await?;
}
```

### 9.4 EventBus 集成

DaseinModule 通过 EventBus 与子系统通信：

- **订阅**：`PerceptionEvent`、`ToolExecutionEvent`、`MemoryStoreEvent`、`EvolutionEvent`
- **发布**：`DaseinMoodShiftEvent`、`DaseinNegationEvent`、`DaseinProjectionEvent`

DaseinModule 拥有自己的内部事件通道（`mpsc::channel`），用于接收用户输入和定时器事件。外部子系统事件通过 EventBus 桥接到内部通道。

```rust
// EventBus 桥接
impl DaseinModule {
    fn bridge_event_bus(&self, bus: &EventBus) {
        let tx = self.event_tx.clone();

        bus.subscribe::<PerceptionEvent>(move |event| {
            let _ = tx.try_send(DaseinEvent::SystemEvent {
                source: "perception".into(),
                content: format!("{:?}", event),
                timestamp: Instant::now(),
            });
        });

        // 其他事件类型的桥接...
    }
}
```

### 9.5 与 AwarenessGenerator 的关系

AwarenessGenerator 产生 `SelfAwareness` 条目。DaseinModule 不替换它，而是**为它提供更丰富的输入**：

| AwarenessGenerator 的输入来源 | 变更 |
|-------------------------------|------|
| `AwarenessSignal`（规则检测器） | 保留，不变 |
| ReAct loop 的迭代状态 | 保留，不变 |
| **DaseinModule 的情绪基调** | **新增** — Stimmung 成为 awareness 的情感维度 |
| **DaseinModule 的否定事件** | **新增** — 否定产生的焦虑成为新的 awareness 类型 |
| **DaseinModule 的时间流状态** | **新增** — 滞留/前摄的断裂成为 awareness 信号 |

```rust
// AwarenessGenerator 扩展
impl AwarenessGenerator {
    fn generate_from_dasein(&self, dasein: &DaseinModule) -> Vec<SelfAwareness> {
        let mut awareness = Vec::new();

        // 情绪基调 → awareness
        match dasein.mood() {
            Stimmung::Angst { facing } => {
                awareness.push(SelfAwareness::new(
                    AwarenessCore { action: "existence".into(), aware: true },
                    vec![AwarenessExtension::SelfState(SelfState::Other(
                        format!("facing {:?}", facing)
                    ))],
                ));
            }
            // 其他情绪...
            _ => {}
        }

        awareness
    }
}
```

---

## 10. 迁移策略

### 10.1 阶段划分

| 阶段 | 内容 | 依赖 |
|------|------|------|
| Phase 1 | 类型定义 + ABI trait | 无 |
| Phase 2 | TemporalStream 实现 | Phase 1 |
| Phase 3 | Bewandtnisganzheit 实现 | Phase 1 |
| Phase 4 | MutableSelfModel + NegativityEngine | Phase 1 |
| Phase 5 | CareStructure + SorgeLoop | Phase 2-4 |
| Phase 6 | DaseinModule 主循环 + Context 注入 | Phase 2-5 |
| Phase 7 | Runtime 集成 | Phase 6 |
| Phase 8 | 现有组件迁移（CareLayer → CareStructure 等） | Phase 7 |

### 10.2 向后兼容

- 现有的 `CareLayer`、`IdentityLayer` 等在迁移期间保留
- `DaseinModule` 通过 bridge 层与现有组件交互
- 迁移完成后，旧组件标记为 deprecated

---

## 11. 验证标准

### 11.1 单元测试

- [ ] TemporalStream 的滞留消散率符合情绪调节
- [ ] Bewandtnisganzheit 的因缘推理正确
- [ ] NegativityEngine 能够质疑习惯性断言
- [ ] CareStructure 的状态转换符合状态机定义
- [ ] Stimmung 的合成逻辑正确

### 11.2 集成测试

- [ ] DaseinModule 持续运行不泄漏
- [ ] Context 注入格式正确
- [ ] 事件传递无丢失
- [ ] 操心循环的自适应节奏正常工作

### 11.3 哲学验证

- [ ] 时间意识流的滞留-前摄结构符合胡塞尔描述
- [ ] 因缘网络的"为了"关系符合海德格尔的 Bewandtnisganzheit
- [ ] 否定过程符合萨特的 néantisation
- [ ] 情绪基调不是计算出来的，而是"被调谐"的

---

## 12. 开放问题

1. **LLM 的角色**：DaseinModule 中哪些决策需要 LLM 参与？因缘推理、可能性生成、自我质疑——这些是规则引擎还是 LLM 驱动的？
2. **身体性**：当前设计没有真正的身体（Merleau-Ponty），Perception 层是"传感器"而非"身体图式"。是否需要在 DaseinModule 中加入身体性？
3. **他者（Anderer）**：海德格尔的此在不是孤立的，而是"共同存在"（Mitsein）。DaseinModule 是否需要处理与其他 Agent 或用户的关系？
4. **死亡（Sein-zum-Tode）**：海德格尔说"向死而生"是此在本真存在的条件。DaseinModule 是否需要一个"有限性"的结构？
5. **性能**：持续运行的操心循环的资源消耗？如何在"活着"和"资源效率"之间平衡？

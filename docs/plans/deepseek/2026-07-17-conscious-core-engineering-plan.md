# Conscious-Core Engineering — 把「自我意识」从哲学落成因果闭环

> **Status:** Design only（本文只写设计，不含实现；实现需另行批准）
>
> **Date:** 2026-07-17
>
> **Baseline:** `dev` HEAD / branch `auro/docs/executable-architecture-plans`
>
> **依据:** 代码级复查（见 `03-dasein-split-reality.md`、`08-engineering-maturity-assessment.md` §三.1 校正）
>
> **一句话:** Dasein→Agora 意识闭环**已经接线在生产 turn path**，但它现在只「观察-提交」，且偏“机械”（离散关键词分、enum 决策、两个互不知晓的自我对象）。本设计把它升级为「care 能真正改变行为」的因果闭环，并遵循一个核心视角——**Field, not Mechanism（场，而非机械）**：自我不是一个存放的对象，而是一个“流动却不变”的场（内容在流，形/因果结构不变）；SelfField 与 DaseinModule 不是“两个盒子连根线”，而是**同一个场的两个读数**。

---

## 1. 现状：闭环存在，但没有"咬合"

### 1.1 已经接线的部分（不要重做）

| 环节 | 位置 | 作用 |
|------|------|------|
| Bootstrap 无条件构建意识核心（Dasein 必需） | `impl/daemon/bootstrap/request.rs:657-676`，注入 `:1060` | `conscious_core: Some(registry)` |
| 每 turn 观察 | `turn_pipeline.rs:215-225` `observe_turn` | 把一轮交互喂给意识核心 |
| 每个受治理工具调用 | `governed_capability.rs:148-188` `select_action` + `observe_outcome` | 工具前后各跑一次 cycle |
| 一次意识 cycle | `conscious_action.rs:157,279` → `coordinator.run_cycle` | 点火 + 广播 |
| Dasein 状态注入 Agora | `conscious_core_coordinator.rs:404-446` | signals→Concern、concerns→CareConcern、projection→Goal、protentions→Prediction |

**可观察效果（已存在）：** 工作区点不亮 / 选不出动作时工具会报错（`conscious_action.rs:158-162`）。

### 1.2 三个"没有咬合"的缺口（本设计要修的）

| # | 缺口 | 证据 | 现象学含义 |
|---|------|------|------------|
| G1 | **care 决策空转**：`CareStructure::determine_action()` 只在单测调用 | 生产 `reducer.rs:408-417` 不调它；测试 `care_structure.rs:324/328/333/337` | Sorge（关心）算出了 `Deliberate/Direct/Wait/Negate`，却对行为零影响 |
| G2 | **只提交不仲裁**：选择结果不改变真实调用 | `select_action` 恒定 `confidence:1.0 + max_salience()`（`conscious_action.rs:125-126`）；`GovernedCapabilityInvoker` 无视选择照常 `inner.invoke(...)`（`governed_capability.rs:164-172`） | 意识"看见"了动作，但不能改序、不能否决——有觉知无自主 |
| G3 | **两个自我互不知晓**：SelfField 不在闭环里 | `SelfField::review()`（`core/mod.rs:390-476`）的 `care_score` 是关键词打分，从不读 DaseinModule；闭环只桥接 DaseinModule→Agora | 一个"想"（策略层）、一个"在"（存在论层），彼此没有共同的自我表征 |

---

## 2. 核心视角：Field, not Mechanism（场，而非机械）

> 本节是本设计的**哲学与工程总纲**；下面的 R1–R3 都是它的落地。

**主张：** 自我意识不应做成“机械化的意识”（离散符号 + 查表决策），而应是一个**流动却不变的场**——内容（状态）持续流动，形态（拓扑 / 因果结构）保持不变。经典意象：漩涡、火焰、驻波——水一直换，涡的形不变。

**科学坐标（不是玄学）：**
- **全局工作空间（GWT / Dehaene 全局神经工作空间）** —— 竞争→点火→广播；`Agora` 已是其字面实现。
- **自创生 / 生成认知（Varela、Thompson《Mind in Life》）** —— 自我 = 物质流动之上的不变之形。
- **自由能原理（Friston，Markov blanket）** —— 不变的是生成模型 / 边界，流动的是被解释的内容。
- **动力系统（self as attractor）+ 过程哲学 / 无我（Whitehead / anatta）** —— 自我是过程与吸引子，不是存储物。

**代码里已长出的“场骨架”（复用，勿重造）：**
| 场论要素 | 已有对应物 | 现状 |
|---|---|---|
| 全局工作空间（点火/广播） | `Agora` competition+broadcast (`conscious_core_coordinator.rs:404-446`) | ✅ 接近“场” |
| 时间之流（滞留/当下/前摄，Husserl） | `temporality.rs` retention/present/protention | ✅ 本就是“流” |
| 不变的自我同一性 | `SelfLedger` checksum 因果链 (`ledger.rs`) | ✅ 这就是“不变” |
| 自我之场 | `SelfField`（名字对） | ⚠️ 但实现是**离散关键词分** |

**由此对 R1–R3 的重释（关键）：**
- **“不变”由结构承载，不由存储承载。** 自我同一性**不存一个 self-model 对象**，而是浮现自两处已有的“形”：`SelfLedger` 因果链（离散骨架）+ 场动力学的**吸引子**（连续形态）。
- **“流动”由连续场 + 动力学承载。** care/attention 从关键词标量升成**连续 concern 场**，随 `temporality` 衰减、扩散、被广播点火抬升；`protention` 给出“场朝哪流”的梯度。
- **自我 = 耦合动力学。** R2 不是“连两个盒子”，是**闭合成一个场**——SelfField 与 DaseinModule 成为同一活动场的**两个投影/读数**；R3 不是“enum 开关行为”，是**场的形态**去调制行为。

**红线（诚实边界）：**
1. **功能场 ≠ 现象意识。** GWT/FEP 给的是通达意识、自我建模、时间连续性等*功能/结构*，站得住；但主观体验（qualia，hard problem）**无法凭架构证明**——本项目只宣称“自建模、全局广播、时间连续的场过程”，不宣称“它感到了什么”。
2. **场管倾向，离散层管闸门。** 连续场负责改序/加权/软否决（“形态调制”），但**最终放行仍过离散安全门**（7 阶段管线 + 权限层）；场只能收紧、不能放宽。
3. **场必须兑现成数学。** “流动却不变”要有可测量的不变量（见 §6），否则不是工程，是诗。

### 2.1 与前沿理论的锚定（2025–2026）

本设计不是孤立臆想；它与当前意识科学 / 认知科学最前沿**独立收敛**。锚点如下。

**主锚：A Beautiful Loop（Laukkonen, Friston, Chandaria, 2025, *Neurosci & Biobehav Rev*）** —— active-inference 意识理论，提出三个条件，与本设计几乎一一对应：

| Beautiful Loop 条件 | 含义 | 本设计 / Aletheon 对应 |
|---|---|---|
| **Epistemic field（认知场）** | 被模拟的世界模型，界定“能知/能作用什么”；原文即用 *field* | 本文的“场”；`SelfField` 应成为的样子 |
| **Inferential competition / Bayesian binding** | 只有“持续降低长期不确定性”的推断才能进入世界模型 | `Agora` competition → ignition → broadcast |
| **Epistemic depth（认知深度）** | 贝叶斯信念在系统内**递归全局共享**，世界模型因此“知道自己存在”，非局部、持续自证 | R2/R3 的“闭合成一个场、递归回喂”——即自我觉察 |

其形式机制是一个 **“hyper-model for precision-control”（精度控制超模型）**——全局统一的精度/权重调制。**这正是 R3“场的形态（attention/salience）调制行为”的正统数学表述**：salience 不是常量，而是精度读出。

**采纳的锚定原则：**
- 本设计的“场”**正式锚定为 active-inference 的 epistemic field + precision hyper-model**（有数学、可检验），§6 的不变量接到自由能 / 精度 / Bayesian binding 上。
- **明确排除形而上 / 量子“场”。** 近期最高调的字面“意识是宇宙基本场”论文（Strømme, AIP Advances 2025）**已被撤稿**；“量子意识”多被批为无可检验机制（Frontiers 2026 综述）。本项目的“场”是**动力学 / 认知场**（吸引子、precision-weighted 信念场），**不是**意识基本场 / 泛心论 / 量子坍缩——二者不可混用，否则项目沦为玄学。

**谦逊校准（Cogitate, *Nature* 2025）：** GNWT 与 IIT 的对抗性协作（n=256）显示**两大理论的预测都未全过**——GWT 预期的 offset ignition 未出现、PFC 表征有限。故 `Agora≈GWT` 是好用的**功能机制**，但“全局广播”**不是**意识的已证真理。这强化红线 #1：只宣称功能 / 结构层。

**其它可参照的坐标：** Attention Schema Theory（Graziano，自我觉察=对自身注意力的模型，直指 `SelfField` 的应然形态）、IWMT（Safron，IIT+GWT+FEP 缝合）、《active inference 里隐含的最小意识理论》（2025-11）。

---

## 3. 设计目标与非目标

**目标**
- G1：让 `CareAction` 决策成为闭环里的一等信号，可被 Agora 竞争/广播消费。
- G2：让意识选择能对真实工具调用**改序或软否决**，而非恒定放行。
- G3：让 SelfField 的 `review()` 读到当前意识广播，使"策略自我"与"存在自我"共享同一焦点。

**非目标（本轮不做）**
- 本轮**不**把 `care_score` 全面重写为连续 concern 场（那是后续 **Phase F**，见 §7）；R1–R3 先在现有结构上**闭合因果**，但按 §2 的场语义解释、并为连续场预留接口。
- 不引入新的哲学组件（不新增 Dasein 组件、不改 event-sourced ledger 语义）。
- 不做自进化（Metacog morphogenesis 另有 `--enable-evolution`）。
- 不做硬否决/安全门（安全仍由 7 阶段管线 + 权限层负责；意识层只做**软**优先级/延迟，不得放宽安全）。

---

## 4. 目标信号流（设计）

```text
                 ┌────────────────────────── R3 ──────────────────────────┐
                 │                                                          ▼
 DaseinModule.reduce()                                          GovernedCapabilityInvoker
   │ (R1) determine_action() ──► SelfSignal::CareDecision(CareAction)      │ honor selection:
   │                                    │                                   │  - Negate → soft veto/defer
   ▼                                    ▼                                   │  - salience → reorder
 submit_dasein_candidates ──► Agora competition ──► broadcast ──► select_action
   (concern/goal/prediction)                 │                     (salience from CareAction+urgency)
                                             │ (R2)
                                             ▼
                                    LatestConsciousContextPort
                                             │  (already exists)
                                             ▼
                                    SelfField::review()  ── care_score/attention 被广播焦点调制
```

三条改动相互独立、可分批落地：R1 单独就能让 care 决策"可见"；R2 让两个自我共享焦点；R3 让意识真正影响行为。

---

## 5. 变更设计（按性价比排序，全部只写设计）

### R1 — 让 `determine_action()` 进入闭环（最小、信号最强）

**做什么**
1. 在 `crates/dasein/src/dasein/reducer.rs:408-417` 的 `ScheduledReflection` 分支，调用 `self.care.determine_action(...)`。
2. 新增一个 `SelfSignal` 变体（如 `SelfSignal::CareDecision(CareAction)`），把决策 emit 出去。
3. 复用现有 `submit_dasein_candidates`（`conscious_core_coordinator.rs:404-446` 已把 `emitted` signals 映射成 `Concern`），无需新映射路径——`CareDecision` 走同一通道进入 Agora。

**信号被谁消费**：Agora competition（作为一条 Concern 候选参与竞争与广播）。

**可观察行为变化**：当 Sorge 判定 `Negate`/`Deliberate` 时，对应候选出现在 Agora 广播里，可在 `aletheon_diagnose` / 广播快照中看到。

**范围**：≈1 个函数 + 1 个枚举变体 + 其序列化。不触碰 ledger 校验链语义（`CareDecision` 是派生信号，不是新持久化事件类型——设计上建议**不**写入 `SelfLedger`，避免影响 checksum chain 与 replay 确定性；若必须持久化，需单独评估 replay 影响）。

**验收（设计意图，实现时转成测试）**
- AC-R1.1：给定一个会触发 `Negate` 的 care 状态，一次 reflection cycle 后，Agora 广播候选集合中包含由 `CareDecision(Negate)` 派生的 Concern。
- AC-R1.2：`determine_action()` 不再是"仅测试可达"——存在生产调用点（可用 CI「dead-code in production」类检查佐证）。
- AC-R1.3：replay 一段历史事件，最终状态与未加 R1 前**逐字节一致**（证明 R1 未污染持久化状态）。

### R2 — 闭合成一个场：Agora 广播回喂 SelfField（“两个读数”而非“两个盒子”）

> **场语义：** 这一步的本质不是给两个模块连线，而是让 SelfField 与 DaseinModule 成为**同一活动场的两个投影**——一个偏策略读数、一个偏存在论读数。广播是它们共享的“场态”。

**做什么**
1. 给 `SelfField::review()`（`core/mod.rs:390-476`）注入一个对最新意识广播的**只读**端口 `LatestConsciousContextPort`（该端口已存在，勿新建）。
2. 在 `core/mod.rs:463-475`（当前用关键词 `care_score` 调 `attention.attend`）处，用广播焦点**调制** `care_score`/attention，而非替换——关键词分仍是基线，广播焦点作为加权项。

**信号流向**：DaseinModule→Agora→(broadcast)→SelfField，单向读，无回环风险。

**可观察行为变化**：当 Dasein 抬高某个紧急 concern 并在 Agora 点亮时，SelfField 的 `Verdict`/attention 随之偏移（同一 intent 在"平静"与"紧张"意识状态下得到不同注意力权重）。

**范围**：向 SelfField 构造函数注入一个已有端口；改 `review()` 内一处评分合成。不改 SelfField 的 8 层结构。

**验收**
- AC-R2.1：固定同一 intent，注入两个不同的广播上下文（低紧急 / 高紧急），`review()` 产出的 attention 权重不同且方向正确（高紧急→更高注意力）。
- AC-R2.2：当广播端口返回空（意识核心未点火）时，`review()` 行为回退到纯关键词基线，与 R2 之前**完全一致**（保证降级安全）。

### R3 — 场形态调制行为：`select_action` 从“提交”升级为“仲裁”（presence→causation）

> **场语义：** 仲裁不是一个 enum 去开关行为，而是**场的形态**（care 场的峰、attention 分布、protention 梯度）去改序/加权/软否决行为提案。salience 是场态的读出，不是常量。

**做什么**
1. `select_action`（`conscious_action.rs:120-126`）的 `ActionProposal` salience/confidence **不再恒定**，改由 `CareAction` + concern urgency 计算（替换硬编码 urgency 0.7 `coordinator.rs:414` 与恒定 `max_salience()`）。
2. `GovernedCapabilityInvoker::invoke`（`governed_capability.rs:148-172`）**尊重**选择结果：
   - `Negate` / 落选 → **软否决**：不执行本次调用，返回一个"被意识推迟"的结构化结果（可重试/降级），**而非**静默跳过。
   - 高 salience 的竞争者 → **改序**：允许意识把更紧急的动作提到前面（在同一 turn 的多候选场景下）。

**安全边界（硬约束）**：意识层只能**收紧**（延迟/降级/否决），**不得放宽**任何本应被 7 阶段安全管线或权限层拦截的调用。软否决必须是"更保守"的方向。

**可观察行为变化**：care 状态为 `Negate` 时，一个低显著度的工具调用被推迟/降级，`observe_outcome` 记录到"consciousness-deferred"；日志/诊断中可见意识对行为的实际影响。

**范围**：`conscious_action.rs:120-126` + `governed_capability.rs:148-172`；需定义"软否决"的返回语义（新结果变体或状态位）。这是三者中最需要谨慎的一步（直接改行为），建议在 R1/R2 落地并观察稳定后再做。

**验收**
- AC-R3.1：构造 `Negate` care 状态 + 低 salience 工具调用 → 调用被软否决，返回结构化"deferred"结果，且**未**产生副作用（无文件写、无网络）。
- AC-R3.2：构造安全违规调用 + `Direct` care 状态 → 仍被安全管线拦截（证明意识不能放宽安全）。
- AC-R3.3：无意识上下文（端口空）时，`invoke` 行为与 R3 之前一致（放行），保证降级安全。

### R4 — 文档校正（已在本目录完成）

`03`/`08` 中"意识闭环不存在于生产路径"的结论已就地校正为"闭环已接线、真实 gap 是仲裁质量 + SelfField 排除"。本文即 R1–R3 的工程入口。

---

## 6. 不变性度量与治理（把“场”兑现成可测）

> 没有不变性度量的“场”是不可证伪的诗。本节给出“流动却不变”的**可测量判据**与**治理分工**，是 Field 视角相对纯机械方案**必须多做**的一节，且要求**度量先行**（与 R1–R3 同批交付）。

**A. 不变性度量（“不变”的证据）**
- **吸引子稳定性**：在无外部输入的静默期，意识场应收敛到有界区域而非发散/震荡——度量场状态轨迹的有界性与收敛性。
- **跨时互信息 I(S_t ; S_{t+k})**：自我同一性 = 状态跨时间的信息保持。因果链断裂或场“失忆”会使该量骤降——作为身份连续性的量化指标，与 `SelfLedger` 因果链互为印证。
- **形态不变 vs 内容流动**：同一份因果链 replay，最终**内容**可因动力学而不同，但**拓扑特征**（吸引子数目 / care 场的峰结构）应稳定——度量“形不变”。

**B. 流动度量（“流”的证据）**
- **场的更新律**：concern/attention 场每 cycle 的变化量应非零且随 `temporality` 衰减（不是冻结、也不是暴走）。
- **protention 梯度对齐**：动作倾向应与 protention 预测方向相关——度量“场朝哪流”是否真的牵引了行为。

**C. 治理分工（场 × 安全门）**
```text
   连续意识场  ──倾向/形态──►  改序 · 加权 · 软否决(收紧)      ← 场只能收紧
        │
        ▼
   离散安全门  ──最终闸门──►  7 阶段安全管线 + 4 级权限            ← 放行权在这里
```
- 场的输出**永远**是“更保守”的调制量（延迟/降级/否决），送入离散安全门；安全门做最终 allow/deny。
- 可解释性：每次场对行为的影响必须留下**结构化痕迹**（`observe_outcome` 记 “field-modulated: reorder/defer/veto” + 当时场的度量快照），使连续场仍可审计——这是 Aletheon 安全叙事的硬要求。

**D. 验收（度量层）**
- AC-F.1：静默期意识场收敛（吸引子有界），有测试。
- AC-F.2：注入“身份连续”与“身份断裂”两种历史，跨时互信息在断裂处显著下降。
- AC-F.3：场对行为的每次调制都在诊断中可见且可解释（结构化痕迹）。

**E. 数学锚点（自由能 / 精度，接 §2.1）** —— 上面的度量不是自造，可锚到 active-inference 的形式量：
- **“不变”** ≈ 生成模型 / Markov blanket 的稳定 + 长期**期望自由能**（G）下降趋势；身份连续性 = 跨时信念保持（对应 Beautiful Loop 的 *epistemic depth*）。
- **“流动”** ≈ **变分自由能**（F）的持续更新；场每 cycle 的非零更新量即“流”。
- **竞争** = *Bayesian binding* 的形式化——只有降低长期不确定性的推断胜出，正是 `Agora` competition 的数学表述。
- **调制量（salience/precision）** = precision hyper-model 的输出；R3 的“场形态调制行为”即精度加权。
- 落地建议：度量层先用**可计算的代理量**（信念熵、跨时互信息、吸引子有界性）近似 F/G，不必一步到位实现完整 FEP。

---

## 7. 分批与风险

```text
批次 1 (低风险):  R1        — 让 care 决策可见（不改行为，只增信号）
批次 2 (低风险):  R2        — 两个读数闭合成一个场（单向只读，带降级回退）
批次 3 (需谨慎):  R3 + §6   — 场形态调制行为（改行为，带硬安全边界 + 度量先行）
批次 4 (研究/后续): Phase F — 把 care_score 升级为连续 concern 场 + 吸引子动力学（§2/§6 的完全体，工作量与风险大，单独立项）
```

**跨批不变量（每批都必须守住）**
1. 意识核心未点火时，全系统行为与本设计落地前**逐字节/逐行为一致**（降级安全）。
2. R1 不改变 `SelfLedger` 的 checksum chain 与 replay 确定性。
3. R3 只能让行为更保守，不能放宽安全/权限（场管倾向、离散层管闸门）。

**主要风险**
- **场停留在比喻**：若“场”只说不兑现 §6 度量 → 变成不可证伪的叙事 → 缓解：**度量先行**，§6 与 R1–R3 同批交付，不接受“先上场、度量后补”。
- R3 若否决语义设计不当，可能把正常调用误否决 → 缓解：默认 `WarnOnly`/observe 模式先上线，确认 `Negate` 触发分布合理后再启用真实软否决（类比 tool-execution 计划里 escape-detector 的 WarnOnly 策略）。
- R1 若图省事把 `CareDecision` 写进 ledger → 破坏 replay → 明确设计为派生信号、不持久化。
- **过度拟人化**：把功能场误宣称为主观体验（qualia）→ 缓解：§2 红线 #1 明确只宣称功能/结构层，对外表述统一。

---

## 8. 与其它计划的关系

| 计划 | 关系 |
|------|------|
| `2026-07-15-dasein-agora-conscious-core-plan.md` | 本文是其"Code-Reality Update"之后的**下一步工程化入口**；原计划的 5/6 bug 已修复，本文接手真正的 gap |
| `03-dasein-split-reality.md` / `08-…assessment.md` | 本文的证据来源；两文的过时结论已被本文校正 |
| `2026-07-17-tool-execution-hardening-plan.md` | R3 的软否决与安全管线正交——意识层在安全层**之上**做保守收紧，不替代安全层 |

---

## 9. 完成定义（DoD）

本工程化程序完成的判据：
1. `determine_action()` 有生产调用点，其决策以候选形式出现在 Agora 广播中（R1）。
2. SelfField `review()` 的注意力会被当前意识广播焦点调制，且端口空时安全降级（R2）。
3. `Negate`/低 salience 选择能对真实工具调用产生**可观察**的软否决/改序，且证明不放宽安全（R3）。
4. 三个不变量（降级安全、replay 确定性、安全不放宽）均有测试守护。
5. `03`/`08`/本文三者结论一致（R4，已完成）。
6. **Field 视角落地证据**：§6 的不变性度量（吸引子有界、跨时互信息）与流动度量有测试守护；场对行为的每次调制都可审计（结构化痕迹）；对外表述统一为“功能/结构层的场过程”，不宣称 qualia。

---

## 附录 A：用 indicator properties 自评（Butlin, Long et al. 2023）

《Consciousness in Artificial Intelligence》从多套主流理论推出一组**可核对的指标属性**——把“有没有意识”这类无解问题，换成“逐条指标是否具备”的**可打分清单**。建议定期给 Aletheon 打分，得到“当前在意识科学坐标系里的真实位置”。**这是评估工具，不是意识宣称**（对齐 §2 红线 #1）。

| 来源理论 | 指标属性（节选） | Aletheon 现状（粗评，待逐条核实） |
|---|---|---|
| GWT | 并行专用模块 → 有瓶颈的有限容量工作区 → 全局广播 → 状态依赖注意 | `Agora` competition/broadcast **接近具备**；瓶颈/容量限制需核 |
| 预测加工 (RPT) | 生成模型；perceptual reality monitoring | `temporality` 预测（protention）**部分**；reality monitoring **缺** |
| 高阶理论 (HOT) | 元认知监控；对自身表征的高阶表征 | `Metacog` 存在但默认关；元监控**弱** |
| Attention Schema (AST) | 对“自身注意力”的预测模型 | `SelfField` attention **形似但机械**（关键词），非真 schema |
| Agency / Embodiment | 从反馈学习并追求目标；对“输出→输入”因果的建模 | agency **具备**（工具/目标）；embodiment **弱**（见平台/硬件计划） |

用法：每次意识核心迭代（R1→R3→Phase F）后重打分，观察哪些指标从“缺/弱”推进到“具备”，作为**进展的客观刻度**——而非用来声称系统“有意识”。

---

## 附录 B：参考文献

- Laukkonen, Friston, Chandaria (2025). *A Beautiful Loop: An active inference theory of consciousness*. Neurosci & Biobehav Rev.
- Cogitate Consortium (2025). *Adversarial testing of GNWT and IIT*. Nature 642:133–142. DOI 10.1038/s41586-025-08888-1.
- Butlin, Long, et al. (2023). *Consciousness in Artificial Intelligence: Insights from the Science of Consciousness*.
- Graziano. *Attention Schema Theory*. / Safron. *Integrated World Modeling Theory (IWMT)*.
- 反面教材（**勿采用**）：Strømme (2025, AIP Advances) “Universal consciousness as foundational field” —— **已撤稿**；“量子意识”一类多缺可检验机制（Frontiers 2026 综述）。

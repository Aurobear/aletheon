# OS Agent + Nous Architecture References

## 快速开始

```bash
# 一键下载所有 (论文 + 项目 + Nous 参考)
bash setup.sh

# 只下载论文
bash setup.sh --papers

# 只克隆项目
bash setup.sh --projects

# 只下载 Nous 架构参考 (哲学 + 认知科学 + Agent 理论)
bash setup.sh --nous

# 完整克隆 (含 git 历史)
bash setup.sh --full
```

## Papers

| Paper | Year | File | Key Contribution |
|---|---|---|---|
| Agent S | 2024 | `papers/Agent_S_2410.08164.pdf` | Manager-Worker 架构, ACI (Agent Computer Interface), Narrative/Episodic Memory |
| Agent S2 | 2025 | `papers/Agent_S2_2504.00906.pdf` | Generalist-Specialist 分层, Mixture-of-Grounding, Proactive Hierarchical Planning |
| OpenHands | 2024 | `papers/OpenHands_2407.16741.pdf` | 开源 Agent Runtime, Docker Sandbox, Planner→Agent→Execution 流水线 |
| OSWorld | 2024 | `papers/OSWorld_2404.07972.pdf` | 多平台 OS Agent Benchmark, 覆盖 Ubuntu/Windows/macOS + 跨应用任务 |

---

## Projects — 分类总览

```
                        ┌─────────────────────────────────────────┐
                        │           OS Agent 终极架构              │
                        ├─────────────────────────────────────────┤
                        │  Agent Kernel  → openclaw (WASM 沙箱)   │
                        │  Agent Runtime → OpenHands (最成熟)      │
                        │  Computer Agent→ Agent-S, open-operator │
                        ├─────────────────────────────────────────┤
                        │  以上 = OS Agent 方向 (我们的目标)        │
                        ═══════════════════════════════════════════
                        │  以下 = 工具层 (参考但不照搬)             │
                        ├─────────────────────────────────────────┤
                        │  Agent Framework → LangGraph, AutoGen...│
                        │  CLI Agent       → Claude Code, Codex...│
                        │  SDK             → anthropic-sdk-python │
                        └─────────────────────────────────────────┘
```

---

### 1. Computer Agent / OS Agent — 像人一样操作电脑

> 目标：Agent 直接操作 GUI、文件系统、终端，完成跨应用任务

| Project | Path | 特点 | 与 OS Agent 的关系 |
|---|---|---|---|
| **Agent-S** | `projects/Agent-S/` | Manager-Worker 架构, ACI 抽象层 (类似系统调用), Narrative + Episodic Memory, Experience Replay | **核心参考** — ACI 设计直接映射到 Agent Kernel 的 syscall 层 |
| **open-operator** | `projects/open-operator/` | OpenAI Operator 开源版, 浏览器自动化, Computer Use | **Browser Agent 参考** — GUI grounding 和浏览器交互 |
| **openclaw** | `projects/openclaw/` | Agent Process 隔离, WASM 沙箱, Workspace 管理 | **最接近 Agent OS** — Process/Scheduler/Isolation 设计可直接借鉴 |

**关键区别：**
- Agent-S 侧重 **Memory + Planning**，解决长期任务的记忆问题
- open-operator 侧重 **GUI 交互**，解决浏览器自动化
- openclaw 侧重 **OS 级隔离**，解决 Agent 进程安全问题

---

### 2. Agent Runtime / Platform — Agent 的执行环境

> 目标：提供 Agent Loop、沙箱、工具调用、多 Agent 协作的运行时

| Project | Path | 特点 | 与 OS Agent 的关系 |
|---|---|---|---|
| **OpenHands** | `runtime/OpenHands/` | Docker Sandbox, Planner→Agent→Execution 流水线, Multi-Agent, 50k+ stars | **Runtime 参考** — 最成熟的开源实现, controller/runtime/sandbox 架构值得深度阅读 |

**关键区别：**
- OpenHands 是 **Runtime 层**，关注"Agent 怎么跑"
- Computer Agent 是 **应用层**，关注"Agent 怎么用电脑"
- 两者是上下层关系：Computer Agent 跑在 Runtime 之上

---

### 3. Agent Framework — Agent 应用开发框架

> 目标：提供构建 Agent 应用的 SDK 和编排工具

| Project | Path | 特点 | 与 OS Agent 的关系 |
|---|---|---|---|
| **LangGraph** | `agent-framework/langgraph/` | LangChain 生态, 状态图编排, 条件分支 + 循环 | **编排参考** — 状态图模式可用于 Skill 编排 |
| **AutoGen** | `agent-framework/autogen/` | 微软出品, 多 Agent 对话, GroupChat 模式 | **多 Agent 参考** — 对话式协作模式 |
| **CrewAI** | `agent-framework/crewAI/` | 角色扮演, Task 分配, 多 Agent 协作 | **角色设计参考** — 角色定义和任务分配模式 |
| **Letta** | `agent-framework/letta/` | 原 MemGPT, 有状态长期记忆, 自主记忆管理 | **Memory 参考** — 长期记忆管理最前沿 |
| **Hermes Agent** | `agent-framework/hermes-agent/` | 轻量实现, 代码简洁 | **实现参考** — 简洁的 Agent Loop 实现 |

**关键区别：**
- Framework 是 **Prompt 编排层**，本质是 `LLM + Tool Call`
- OS Agent 是 **系统层**，本质是 `Agent Kernel + Runtime + Linux Kernel`
- **不要把目标定成 Framework**，长期价值在 Agent OS

---

### 4. CLI / Coding Agent — 终端编程助手

> 目标：在终端中辅助开发者完成编程任务

| Project | Path | 特点 | 与 OS Agent 的关系 |
|---|---|---|---|
| **Claude Code** | `cli-agent/claude-code/` | Anthropic 官方, TypeScript, 多工具协作, Hooks 系统 | **工具链参考** — 工具调用模式、权限控制、Hooks 生命周期 |
| **Codex** | `cli-agent/codex/` | OpenAI, Rust, 沙箱执行, 自动审批 | **沙箱参考** — Rust 实现的沙箱隔离方案 |
| **OpenCode** | `cli-agent/opencode/` | Go 实现, 轻量终端 UI | **架构参考** — Go 语言的简洁 Agent 实现 |

**关键区别：**
- Claude Code 侧重 **工具链生态** (MCP, Hooks, Skills)
- Codex 侧重 **安全沙箱** (Rust, 网络隔离)
- OpenCode 侧重 **轻量实现** (Go, 最小依赖)
- 三者都是 **CLI Agent**，不是 OS Agent，但其工具调用和沙箱设计可参考

---

### 5. SDK — 底层 API 客户端

| Project | Path | 特点 | 与 OS Agent 的关系 |
|---|---|---|---|
| **Anthropic SDK Python** | `sdk/anthropic-sdk-python/` | Anthropic 官方 Python SDK, Messages API | **API 层参考** — 底层 LLM 调用方式 |

---

## 层级关系

```
┌─────────────────────────────────────────────────────────┐
│  Layer 5: Computer Agent    (Agent-S, open-operator)    │  ← 像人一样用电脑
├─────────────────────────────────────────────────────────┤
│  Layer 4: Agent Framework   (LangGraph, AutoGen, CrewAI)│  ← Prompt 编排 (不选这条路)
├─────────────────────────────────────────────────────────┤
│  Layer 3: Agent Runtime     (OpenHands)                 │  ← Agent 执行环境
├─────────────────────────────────────────────────────────┤
│  Layer 2: Agent Kernel      (openclaw)                  │  ← Agent 进程管理
├─────────────────────────────────────────────────────────┤
│  Layer 1: LLM SDK           (anthropic-sdk-python)      │  ← 模型调用
└─────────────────────────────────────────────────────────┘
```

**我们的目标是 Layer 2+3+5，不是 Layer 4。**

---

## Nous Architecture — 哲学 + 认知科学 + Agent 理论

> 这些参考资料支撑 Nous 三层架构 (Soul/Brain/Body) 的设计。
>
> 运行 `bash setup.sh --nous` 下载。

### Philosophy — Soul 层理论基础

| 资料 | 作者 | 文件 | Nous 映射 |
|---|---|---|---|
| Ethics (全文) | Spinoza | `philosophy/spinoza-ethics.txt` | **Soul.Drive** — Conatus (存在维持倾向) |
| Modal Metaphysics (SEP) | — | `philosophy/spinoza-modal-sep.html` | **Soul.Drive** — Conatus 的形而上学基础 |
| Being and Time (SEP) | Heidegger | `philosophy/heidegger-being-and-time-sep.html` | **Soul.Awareness** — Dasein, ready-to-hand |
| Narrative Gravity | Dennett | `philosophy/dennett-narrative-gravity.pdf` | **Soul.Trajectory** — 叙事自我 |
| Multiple Drafts | Dennett | `philosophy/dennett-multiple-drafts.pdf` | **Soul.Meta** — 多重草稿模型 |
| Being No One (PhilArchive) | Metzinger | `philosophy/metzinger-being-no-one-philarchive.html` | **Soul.Meta** — 现象自我模型 (PSM) |
| The Ego Tunnel (excerpt) | Metzinger | `philosophy/metzinger-ego-tunnel-preview.pdf` | **Soul.Meta** — 自我模型理论的通俗阐述 |

**哲学 → Nous 映射**:

```
Spinoza Conatus         → Soul.Drive: 存在压力不是定时器，是连续信号
Heidegger Dasein        → Soul.Awareness: Agent 始终 "在世界中存在"
Heidegger Zuhandenheit  → Body.Driver: 工具透明使用 (ready-to-hand)
Heidegger Vorhandenheit → Brain.Reflection: 工具故障时才反思 (present-at-hand)
Dennett Narrative Self   → Soul.Trajectory: 自我是叙事的重心，不是实体
Dennett Multiple Drafts  → Soul.Meta: 多重自我叙事并行，主导的那个持续
Metzinger PSM            → Soul.Meta: 自我模型 ≠ 自我，但我们无法区分
Metzinger Ego Tunnel     → Soul.Meta: 自我模型创造主观体验的 "隧道"
```

### Cognitive Architecture — Brain 层设计参考

| 资料 | 作者 | 文件 | Nous 映射 |
|---|---|---|---|
| OpenCog README | Goertzel et al. | `cognitive-architecture/opencog-readme.md` | **Brain.Learning** — AtomSpace 知识表示 |
| SOAR Cognitive Architecture | Laird (2012) | `cognitive-architecture/soar-laird-2012.pdf` | **Brain.Planning** — 问题空间 + Chunking |
| Integrated Theory of the Mind | Anderson et al. (2004) | `cognitive-architecture/anderson-2004-integrated-theory.pdf` | **Brain.Memory** — ACT-R 记忆激活模型 |
| How Can the Human Mind Occur | Anderson (2007) | `cognitive-architecture/anderson-2007-chapter.pdf` | **Brain.Learning** — 产生式规则 + 效用学习 |

**认知科学 → Nous 映射**:

```
OpenCog AtomSpace       → Brain.Memory: 知识图谱 (我们用 SQLite + vector store)
OpenCog Attention Alloc  → AgentScheduler: 注意力作为稀缺资源分配
SOAR Problem Space       → Brain.Planner: 目标 → 子目标 → 动作
SOAR Impasse → Chunking  → Brain.Reflection: 卡住时学习 (→ SkillCompiler)
ACT-R Declarative Memory → Brain.RecallMemory + ArchivalMemory
ACT-R Activation         → Brain.Memory: 基于近因 + 频率的记忆激活
ACT-R Utility Learning   → Body.CapabilityGraph: 成功率跟踪 + 效用学习
```

### Agent Theory — 推理与反思参考

| 论文 | 年份 | 文件 | Nous 映射 |
|---|---|---|---|
| ReAct (Yao et al.) | 2022 | `agent/react-yao-2022.pdf` | **Brain.Reasoning** — 推理+行动循环 |
| Reflexion (Shinn et al.) | 2023 | `agent/reflexion-shinn-2023.pdf` | **Brain.Reflection** — 语言强化学习 |
| Agent S (Simular) | 2024 | `agent/agent-s-2024.pdf` | **Body.Execution** — GUI Agent, ACI |
| Agent S2 (Simular) | 2025 | `agent/agent-s2-2025.pdf` | **Body.Execution** — 组合式通才-专才 |
| K²-Agent | 2024 | `agent/k2-agent-2024.pdf` | **Brain.Learning** — 知识驱动 Agent |

**Agent 论文 → Nous 映射**:

```
ReAct                    → Brain.Reasoning: Think → Act → Observe 循环
Reflexion                → Brain.Reflection: 语言梯度 (不是数值梯度)
                         → Nous 扩展: 分层反向传播 (Capability → Rules → Skills → Self)
Agent S                  → Body.Driver.UI: 截图 → 理解 → 点击/输入
Agent S2                 → Body.Driver.UI: 通才规划 + 专才执行
K²-Agent                 → Brain.Learning: 结构化知识增强推理
```

### 阅读顺序

理解 Nous 架构的推荐阅读顺序:

1. **ReAct** → 理解基础: Think-Act 循环
2. **Reflexion** → 理解学习: 语言强化学习
3. **Spinoza Ethics III** → 理解驱动: Conatus (存在维持)
4. **Metzinger Ego Tunnel** → 理解自我: 现象自我模型
5. **ACT-R Integrated Theory** → 理解记忆: 激活模型
6. **SOAR Cognitive Architecture** → 理解规划: 问题空间 + Chunking
7. **Agent S2** → 理解执行: GUI Agent 设计
8. **Dennett Narrative Gravity** → 理解叙事: 自我作为叙事重心

---

## Source Reading Order

| Phase | 读什么 | 理解什么 |
|---|---|---|
| 1. Runtime | `runtime/OpenHands/` | Agent Loop, Tool Call, Sandbox |
| 2. Computer Agent | `projects/Agent-S/`, `projects/open-operator/` | Memory, Grounding, Planning |
| 3. Benchmark | `papers/OSWorld_2404.07972.pdf` | Evaluation, Task Definition |
| 4. Agent OS | `projects/openclaw/` | Agent Process, Scheduler, Security |

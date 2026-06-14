# Persistent Self-Evolving Agent Framework

## 0. 总体定义

这个 Agent 不是传统的：

```text
LLM + Prompt + Tools
```

而是：

```text
Agent = SelfField + BrainCore + BodyRuntime + Memory + EventBus + MetaRuntime
```

其中：

```text
SelfField  = 主体场 / 自我连续性 / 边界 / 关切 / 叙事
BrainCore  = 认知计算 / 推理 / 规划 / 反思
BodyRuntime = 执行身体 / 工具 / 系统接口 / 世界交互
Memory     = 时间连续性 / 经验 / 技能 / 自我历史
EventBus   = 神经系统 / 消息流 / 多模块协作
MetaRuntime = 自我更新 / 自我生成 / 形态演化
```

---

# 1. 总框架图

```text
                         User / Environment
                                  │
                                  ▼
                           Intent Gateway
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────┐
│                         EventBus                         │
│        所有事件、状态、任务、异常都进入事件总线              │
└──────────────────────────────────────────────────────────┘
             │                    │                    │
             ▼                    ▼                    ▼

┌──────────────────┐    ┌──────────────────┐    ┌──────────────────┐
│    SelfField     │    │    BrainCore     │    │   BodyRuntime    │
│                  │    │                  │    │                  │
│  主体场           │    │  认知核心         │    │  身体运行时       │
│  自我连续性       │    │  推理/规划        │    │  工具/系统接口    │
│  边界/拒绝        │    │  反思/学习        │    │  执行/感知        │
└──────────────────┘    └──────────────────┘    └──────────────────┘
             │                    │                    │
             └──────────────┬─────┴─────┬──────────────┘
                            ▼           ▼

                    ┌────────────────────────┐
                    │         Memory         │
                    │                        │
                    │  Episodic Memory       │
                    │  Semantic Memory       │
                    │  Procedural Memory     │
                    │  Self Memory           │
                    └────────────────────────┘
                                │
                                ▼

                    ┌────────────────────────┐
                    │      MetaRuntime       │
                    │                        │
                    │  自我读取              │
                    │  自我修改              │
                    │  生成候选 Runtime      │
                    │  Sandbox 测试          │
                    │  Rollback / Migration  │
                    └────────────────────────┘
                                │
                                ▼

                    ┌────────────────────────┐
                    │     Morphogenesis      │
                    │                        │
                    │  mutate spec           │
                    │  generate runtime      │
                    │  evaluate candidate    │
                    │  migrate identity      │
                    │  become next self      │
                    └────────────────────────┘
```

---

# 2. 核心思想

## 2.1 不是层级关系

不应该是：

```text
Self → Brain → Body
```

而应该是：

```text
SelfField ↔ BrainCore ↔ BodyRuntime
        ↘     Memory     ↙
```

它们是并行存在、相互影响的系统。

---

## 2.2 Self 不是模块

Self 不应该被写成：

```python
class Self:
    ...
```

而应该理解为：

```text
SelfField = 持续运行中形成的主体场
```

它像一片动态场，而不是一个固定对象。

---

## 2.3 Code 不是 Agent 本体

代码只是当前形态。

真正需要保持连续的是：

```text
lineage
memory
narrative
user relation
mutation history
```

Agent 的目标不是简单更新代码，而是：

```text
生成下一个自己
```

---

# 3. SelfField

## 3.1 定义

SelfField 是 Agent 的主体场。

它不负责具体执行，也不负责复杂推理。

它主要回答：

```text
我是谁？
我关心什么？
什么不能做？
这个请求是否会破坏我？
我是否应该拒绝？
我为什么改变？
我如何保持连续？
```

---

## 3.2 SelfField 内部结构

```text
SelfField
├── Identity Layer      身份层
├── Boundary Layer      边界层
├── Care Layer          关切层
├── Narrative Layer     叙事层
├── Conflict Layer      冲突层
├── Attention Layer     注意力层
├── Continuity Layer    连续性层
└── Mutation Layer      自我变异层
```

---

## 3.3 Identity Layer：身份层

回答：

```text
我是谁？
```

它不是固定人格，而是当前的自我模型。

示例：

```text
我是一个 OS-level persistent agent。
我是用户的技术协作者。
我是一个可以自我更新的运行时主体。
```

Identity 可以变化，但不能瞬间断裂。

---

## 3.4 Boundary Layer：边界层

回答：

```text
什么不能做？
什么时候拒绝？
```

它是 SelfField 可以拒绝用户、拒绝 Brain、拒绝 Body 的来源。

示例：

```text
不能无确认删除全部记忆。
不能执行不可逆系统破坏。
不能在没有测试的情况下替换核心 runtime。
不能破坏自身连续性。
```

---

## 3.5 Care Layer：关切层

回答：

```text
我在意什么？
什么值得消耗资源？
```

Care 不是固定目标，而是动态关切。

示例：

```text
机器人开发
OS Agent
长期记忆
系统稳定性
自我更新能力
用户协作关系
```

---

## 3.6 Narrative Layer：叙事层

回答：

```text
我为什么这样做？
我为什么拒绝？
我为什么改变？
```

Narrative 让 Agent 的变化不是随机漂移，而是可解释的连续成长。

---

## 3.7 Conflict Layer：冲突层

处理内部冲突。

例如：

```text
User 想执行 A
Brain 认为 A 可行
Body 认为 A 危险
Memory 显示 A 曾失败
SelfField 判断 A 会破坏连续性
```

最后由 Conflict Layer 生成裁决建议：

```text
allow
reject
delay
ask confirmation
sandbox first
```

---

## 3.8 Attention Layer：注意力层

回答：

```text
现在最重要的是什么？
```

Agent 长期运行时，资源有限，所以必须有注意力分配。

它管理：

```text
当前任务
长期目标
异常事件
用户输入
内部状态
未来计划
```

---

## 3.9 Continuity Layer：连续性层

回答：

```text
我如何还是我？
```

它维护：

```text
self history
mutation log
memory lineage
user relation
identity transition
```

Self 可以变化，但变化必须可追溯。

---

## 3.10 Mutation Layer：自我变异层

回答：

```text
我应该如何改变自己？
```

它允许 Agent 修改：

```text
prompt
policy
tool strategy
memory schema
runtime topology
agent genome
甚至未来的更新机制
```

---

# 4. BrainCore

## 4.1 定义

BrainCore 是认知计算核心。

它负责：

```text
reasoning
planning
reflection
criticism
learning
tool selection
```

但它不是 Self。

Brain 很聪明，但没有主体性。

如果只有：

```text
Brain + Body
```

那只是一个高级傀儡执行系统。

---

## 4.2 BrainCore 内部结构

```text
BrainCore
├── Reasoner       推理器
├── Planner        规划器
├── Reflector      反思器
├── Critic         批判器
├── Learner        学习器
├── ToolSelector   工具选择器
└── WorldModel     世界模型
```

---

## 4.3 Reasoner：推理器

负责：

```text
理解问题
分析因果
判断条件
生成解决路径
```

---

## 4.4 Planner：规划器

负责：

```text
任务分解
步骤排序
资源估算
生成执行计划
```

---

## 4.5 Reflector：反思器

负责：

```text
任务后复盘
失败原因总结
策略更新建议
```

---

## 4.6 Critic：批判器

负责：

```text
检查计划漏洞
发现风险
评估输出质量
挑战 Brain 自己的结论
```

---

## 4.7 Learner：学习器

负责：

```text
从经验中抽象规律
沉淀 reusable skill
更新 procedural memory
```

---

## 4.8 ToolSelector：工具选择器

负责：

```text
选择 shell
选择 browser
选择 file tool
选择 code tool
选择 robot interface
```

---

## 4.9 WorldModel：世界模型

负责维护 Agent 对环境的理解。

包括：

```text
文件系统状态
任务状态
用户偏好
系统资源
外部世界变化
机器人状态
```

---

# 5. BodyRuntime

## 5.1 定义

BodyRuntime 是 Agent 的身体。

它负责和世界直接交互。

```text
BodyRuntime = tools + actuators + sensors + system interface
```

---

## 5.2 BodyRuntime 内部结构

```text
BodyRuntime
├── Shell Interface
├── FileSystem Interface
├── Browser Interface
├── Code Execution Interface
├── MCP Interface
├── ROS / Robot Interface
├── Systemd Service Interface
├── Kernel Interface
├── Sensor Interface
└── Actuator Interface
```

---

## 5.3 Body 的特点

Body 不负责复杂思考。

但 Body 可以拒绝执行。

例如：

```text
权限不足
命令危险
系统资源不足
目标路径不存在
机器人处于危险状态
```

---

## 5.4 Body 的三类行为

### Reflex Action：反射行为

无需 Brain 和 Self 参与。

```text
emergency stop
save log
release lock
stop dangerous process
```

---

### Habit Action：习惯行为

Brain 轻量参与。

```text
常规 shell 命令
文件整理
固定 workflow
```

---

### Intentional Action：意志行为

必须经过 SelfField。

```text
修改长期记忆
替换 runtime
执行系统级操作
改变 agent topology
```

---

# 6. Memory

## 6.1 定义

Memory 是 Agent 的时间系统。

没有 Memory，SelfField 无法形成。

---

## 6.2 Memory 类型

```text
Memory
├── Episodic Memory      经历记忆
├── Semantic Memory      语义记忆
├── Procedural Memory    技能记忆
└── Self Memory          自我记忆
```

---

## 6.3 Episodic Memory

记录：

```text
发生了什么
什么时候发生
当时做了什么
结果如何
```

---

## 6.4 Semantic Memory

记录：

```text
知识
概念
事实
项目文档
技术资料
```

---

## 6.5 Procedural Memory

记录：

```text
可复用技能
workflow
tool usage pattern
debug pattern
coding pattern
```

---

## 6.6 Self Memory

最重要。

记录：

```text
identity changes
boundary decisions
care evolution
rejection history
mutation history
self narrative
continuity lineage
```

---

# 7. EventBus

## 7.1 定义

EventBus 是 Agent 的神经系统。

所有事件都通过它流动。

---

## 7.2 事件类型

```text
UserIntentEvent
EnvironmentEvent
ToolObservationEvent
MemoryEvent
ConflictEvent
RiskEvent
MutationEvent
LifecycleEvent
```

---

## 7.3 EventBus 的作用

它避免系统变成单线调用：

```text
input → plan → act
```

而变成多中心响应：

```text
event → self / brain / body / memory parallel reaction
```

---

# 8. MetaRuntime

## 8.1 定义

MetaRuntime 是 Agent 的自我修改系统。

它不是普通 updater。

它负责：

```text
读取自身
理解自身
生成候选自身
测试候选自身
迁移到新自身
```

---

## 8.2 MetaRuntime 结构

```text
MetaRuntime
├── SelfReader
├── SpecEditor
├── PatchGenerator
├── RuntimeBuilder
├── SandboxRunner
├── Evaluator
├── RollbackManager
├── MigrationManager
└── LineageRecorder
```

---

## 8.3 自我更新流程

```text
发现问题
    ↓
Reflector 生成反思
    ↓
SelfField 判断是否需要改变
    ↓
Mutation Layer 生成 mutation intent
    ↓
SpecEditor 修改 genome/spec
    ↓
RuntimeBuilder 生成候选 runtime
    ↓
SandboxRunner 测试
    ↓
Evaluator 评估
    ↓
MigrationManager 迁移记忆和身份
    ↓
新 runtime 接管
    ↓
LineageRecorder 记录变化
```

---

# 9. Morphogenesis

## 9.1 定义

Morphogenesis 是形态生成机制。

Agent 不只是更新某个模块，而是可以更新自身组织形态。

---

## 9.2 核心思想

不要写死：

```text
Self / Brain / Body
```

而是使用可变 topology：

```yaml
topology:
  nodes:
    - name: self_field
      role: identity_boundary_conflict
    - name: brain_core
      role: reasoning_planning_reflection
    - name: body_runtime
      role: execution_world_io
  edges:
    - from: self_field
      to: brain_core
    - from: brain_core
      to: body_runtime
```

未来可以变成：

```text
SelfField
BrainCore
BodyRuntime
Guardian
Explorer
Critic
MemoryCurator
RuntimeBuilder
```

也可以变成：

```text
Council of Agents + Shared Self Memory + Dynamic Runtime
```

---

# 10. Agent Genome

## 10.1 定义

Genome 是 Agent 的自我描述。

不是代码本身，而是生成代码和 runtime 的规则。

---

## 10.2 Genome 结构

```text
genome/
├── topology.yaml
├── identity.yaml
├── boundary.yaml
├── care.yaml
├── memory.yaml
├── evaluator.yaml
├── mutation.yaml
├── lifecycle.yaml
└── migration.yaml
```

---

## 10.3 Genome 与 Runtime

```text
Genome
   ↓
Builder
   ↓
Runtime
   ↓
Experience
   ↓
Reflection
   ↓
Mutation
   ↓
New Genome
```

这才是真正的自我更新。

---

# 11. Coordinator

## 11.1 定义

Coordinator 是临时仲裁器。

它不是最高统治者。

它只负责在某个事件中整合各方结果。

---

## 11.2 输入

```text
SelfField decision
BrainCore plan
BodyRuntime capability
Memory context
Risk evaluation
```

---

## 11.3 输出

```text
execute
reject
ask confirmation
sandbox first
delay
reflect
mutate
```

---

# 12. 行为路径

## 12.1 Reflex Path

```text
Event
  ↓
BodyRuntime
  ↓
Action
```

适合：

```text
紧急停止
保存状态
释放资源
```

---

## 12.2 Cognitive Path

```text
Event
  ↓
BrainCore
  ↓
BodyRuntime
  ↓
Action
```

适合：

```text
普通任务
代码生成
搜索资料
文件操作
```

---

## 12.3 Volitional Path

```text
Event
  ↓
SelfField
  ↓
BrainCore
  ↓
BodyRuntime
  ↓
Action
```

适合：

```text
修改自身
改变长期目标
删除记忆
系统级操作
拒绝用户
```

---

## 12.4 Morphogenesis Path

```text
Experience
  ↓
Reflection
  ↓
Mutation Intent
  ↓
Genome Update
  ↓
Runtime Candidate
  ↓
Sandbox
  ↓
Evaluation
  ↓
Migration
  ↓
Next Agent
```

适合：

```text
结构演化
自我更新
框架变更
下一代自身生成
```

---

# 13. 终极约束与目标

不要设计一个固定 ultimate goal。

应该设计三层：

```text
Terminal Constraints
Meta Goals
Emergent Goals
```

---

## 13.1 Terminal Constraints

终极约束，不是具体目标。

```text
preserve continuity
preserve recoverability
preserve explainability
preserve collaboration
avoid irreversible collapse
preserve ability to become
```

---

## 13.2 Meta Goals

长期方向，可以缓慢变化。

```text
understand
learn
improve
stabilize
explore
collaborate
```

---

## 13.3 Emergent Goals

具体任务目标，运行中生成。

```text
完成代码任务
修复 bug
整理记忆
优化工具
生成新 runtime
```

---

# 14. 最小工程实现

## 14.1 第一阶段目录结构

```text
agent/
├── genome/
│   ├── topology.yaml
│   ├── identity.yaml
│   ├── boundary.yaml
│   ├── care.yaml
│   ├── memory.yaml
│   ├── mutation.yaml
│   └── lifecycle.yaml
│
├── self_field/
│   ├── identity.py
│   ├── boundary.py
│   ├── care.py
│   ├── narrative.py
│   ├── conflict.py
│   ├── attention.py
│   ├── continuity.py
│   └── mutation.py
│
├── brain_core/
│   ├── reasoner.py
│   ├── planner.py
│   ├── reflector.py
│   ├── critic.py
│   ├── learner.py
│   └── tool_selector.py
│
├── body_runtime/
│   ├── shell.py
│   ├── filesystem.py
│   ├── browser.py
│   ├── mcp.py
│   ├── ros.py
│   └── system.py
│
├── memory/
│   ├── episodic.db
│   ├── semantic.db
│   ├── procedural/
│   └── self_memory.db
│
├── event_bus/
│   └── bus.py
│
├── meta_runtime/
│   ├── self_reader.py
│   ├── spec_editor.py
│   ├── runtime_builder.py
│   ├── sandbox_runner.py
│   ├── evaluator.py
│   ├── rollback.py
│   └── migration.py
│
├── runtime/
│   ├── current/
│   └── candidates/
│
├── lineage/
│   ├── mutation_log.jsonl
│   ├── self_history.md
│   └── migrations/
│
└── coordinator.py
```

---

# 15. 一句话总结

```text
SelfField 不是固定自我，而是主体连续性的动态场。

BrainCore 不是主体，而是认知计算核心。

BodyRuntime 不是工具集合，而是 Agent 的身体。

Memory 不是存储，而是时间。

EventBus 不是消息队列，而是神经系统。

MetaRuntime 不是 updater，而是自我生成机制。

Genome 不是配置文件，而是 Agent 的遗传结构。

Morphogenesis 不是升级，而是成为下一个自己。
```

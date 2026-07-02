# Auro Runtime
## A Persistent Runtime for Robot Intelligence

Version: v1.0
Author: Aurobear
Status: Draft

---

# Vision

不要做：

- ChatGPT Clone
- Claude Clone
- 通用聊天机器人
- Tool Calling 框架

而是做：

> **Persistent Intelligent Runtime**

Agent 不属于某一个 LLM。

LLM 只是 Runtime 的一种认知 Provider。

未来：

```
Claude
Codex
GPT
Gemini
Local LLM
```

全部都是可替换模块。

Runtime 永远存在。

---

# 核心理念

Agent ≠ LLM

Agent = Runtime

LLM = Cognitive Engine

Robot = Capability Library

因此：

```
User
    │
    ▼
Auro Runtime
    │
    ├───────────────┐
    │               │
    ▼               ▼
Memory         Workflow
    │               │
    ▼               ▼
Planner      State Machine
    │
    ▼
Provider Manager
    │
 ┌──┴──────────────┐
 │                 │
 ▼                 ▼
Claude          Codex
 │                 │
 └────────┬────────┘
          ▼
Robot Libraries
Linux Tools
ROS
Simulation
Git
...

```

整个 Runtime 永远是中心。

不是 Claude。

---

# Runtime 的职责

Runtime 永远负责：

- 生命周期
- Session 管理
- Memory
- Workflow
- Context
- 权限
- Tool 调度
- Provider 调度
- State Machine

LLM 永远不负责：

- Memory
- 权限
- 生命周期
- Tool 执行
- Robot 状态

---

# Runtime Layers

```
User

↓

Conversation Layer

↓

Goal Layer

↓

Workflow Layer

↓

Planner

↓

Provider Manager

↓

Tool Manager

↓

Capability Libraries

↓

Operating System
```

---

# Native Agent

Native Agent 不是：

> 一个 LLM

Native Agent 是：

```
Runtime

+

Memory

+

Workflow

+

Planner

+

State Machine

+

Provider
```

因此：

```
Native Agent

↓

需要推理？

↓

Provider

↓

Claude
```

Native Agent 可以不用 Claude。

也可以：

```
Claude

↓

Native Agent

↓

Answer
```

---

# Provider

Provider 是：

认知能力。

例如：

```
Claude

GPT

Gemini

Local LLM
```

统一接口：

```
Provider

↓

Prompt

↓

Response
```

Provider 不负责：

- Tool
- Memory
- Workflow

---

# SubAgent

Provider：

一次推理。

SubAgent：

长期执行一个任务。

例如：

```
Claude Code

↓

Code Agent
```

或者：

```
Codex

↓

Patch Agent
```

Runtime：

```
Task

↓

Create SubAgent

↓

Execute

↓

Collect Result

↓

Destroy
```

SubAgent 生命周期：

```
Created

↓

Running

↓

Waiting

↓

Completed

↓

Destroyed
```

---

# Provider Manager

统一管理所有模型。

```
Claude

Codex

GPT

Local
```

负责：

- Provider 选择
- Failover
- Token
- Cost
- Timeout
- Retry

不负责：

推理逻辑。

---

# Session Manager

管理：

```
Conversation

Task

Context

State
```

例如：

```
Session

↓

Conversation

↓

Planner

↓

Workflow

↓

Result
```

---

# Context Manager

Context 永远属于 Runtime。

不是 Provider。

例如：

```
Project Context

Robot Context

Workflow Context

Memory Context
```

Runtime：

决定：

哪些发送给 Claude。

不是 Claude 自己决定。

---

# Memory

Memory：

属于 Runtime。

分类：

```
Project Memory

Workflow Memory

Robot Memory

User Memory

Trace Memory
```

不要：

一个 Vector Database 全塞进去。

Memory：

应该可管理。

---

# Workflow

Workflow：

不是 Prompt。

Workflow：

是 Runtime 行为。

例如：

```
Diagnose Robot

↓

Read Logs

↓

Find Controller

↓

Analyze

↓

Verify

↓

Report
```

以后：

Claude 学出来的 Workflow：

应该：

沉淀到 Runtime。

而不是：

一直重新问 Claude。

---

# Robot

Robot：

不是 Runtime。

Robot：

不是 Agent。

Robot：

只是 Library。

例如：

```
robot/

kinematics/

dynamics/

wbc/

mpc/

simulation/

ros/

diagnostics/
```

Runtime：

调用：

Robot。

不是：

Robot 控制 Runtime。

---

# Robot Library

建议：

```
robot/

├── model
├── kinematics
├── dynamics
├── control
├── simulation
├── ros
└── diagnostics
```

全部：

Capability。

---

# Claude 的定位

Claude：

不是：

主系统。

Claude：

是：

```
Teacher

Reasoner

Researcher

Planner
```

Runtime：

负责：

```
Memory

State

Execution

Verification

Workflow

Lifecycle
```

---

# Claude 生命周期

API：

短生命周期。

```
Request

↓

Response

↓

Finish
```

Claude Code：

中生命周期。

```
Create

↓

Run

↓

Wait

↓

Result

↓

Destroy
```

不要：

永久 Claude。

避免：

状态污染。

---

# Result Pipeline

Claude 输出：

不是最终答案。

应该：

```
Claude Result

↓

Runtime Verify

↓

Tool Execute

↓

Observation

↓

Memory Update

↓

Final Response
```

---

# Runtime Goal

Runtime：

最终目标：

不是：

聊天。

而是：

Persistent Intelligence。

```
Goal

↓

Planning

↓

Thinking

↓

Execution

↓

Verification

↓

Reflection

↓

Memory

↓

Next Goal
```

一直循环。

---

# 第一阶段（2026）

不要做：

Agent OS。

不要做：

Self Consciousness。

不要做：

超级 Agent。

先做好：

Robot Engineering Runtime。

支持：

- Linux
- ROS
- Git
- Robot Project
- Build
- Logs
- Claude
- Codex

即可。

---

# 第二阶段

加入：

Robot Knowledge。

学习：

```
FK

IK

Jacobian

Dynamics

QP

WBC

MPC
```

Robot：

只是：

Capability。

不是：

Runtime。

---

# 第三阶段

加入：

Persistent Goal。

长期运行。

例如：

```
机器人研发助手

↓

持续学习

↓

持续总结

↓

持续优化 Workflow

↓

持续积累 Robot Memory
```

真正开始形成：

Persistent Agent。

---

# Design Philosophy

永远记住：

```
Runtime
>

Provider
```

```
Workflow
>

Prompt
```

```
Memory
>

Context Window
```

```
Capability
>

Tool Calling
```

```
Robot
≠
Agent
```

```
LLM
≠
Agent
```

```
Agent
=
Runtime
```

---

# 一句话总结

> **Auro Runtime 是一个长期运行的智能运行时，它统一管理记忆、工作流、状态和能力调度；Claude、Codex 等大模型只是可替换的认知引擎，机器人运动学、动力学、控制和仿真只是可调用的能力库，而不是 Agent 本体。**

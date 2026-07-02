# Auro Runtime Open Source Proposal

## Towards a Persistent Intelligent Runtime

**Version:** Draft v0.1

---

# Why this document

经过一段时间开发，目前 Auro Runtime 已经拥有约 20 万行 Rust 代码。

项目已经能够运行，并具有 Agent 的基础能力。

但是随着规模增长，一个问题越来越明显：

> **代码越来越多，但是职责越来越模糊。**

继续增加功能，只会让维护越来越困难。

因此，在继续开发之前，需要重新定义整个项目。

这份文档并不是描述代码。

而是定义：

* 项目定位
* 开源策略
* 模块职责
* 社区生态
* 长期演进方向

---

# Problem

目前 AI Agent 项目普遍存在几个问题：

## Chat First

很多 Agent：

```
User

↓

LLM

↓

Tool

↓

Answer
```

整个系统围绕：

> Chat

设计。

但是：

真正长期运行的智能系统，并不是聊天。

---

## Everything is Agent

很多 Framework：

Memory

Workflow

Planner

Tool

Robot

Plugin

全部放进：

Agent。

最后：

Agent：

越来越大。

越来越难维护。

---

## No Clear Boundary

很多项目：

很难回答：

```
什么属于 Runtime？

什么属于 Agent？

什么属于 Tool？

什么属于 Robot？
```

最终：

所有东西：

耦合在一起。

---

# Vision

Auro Runtime：

不是：

聊天机器人。

不是：

Claude Wrapper。

不是：

Prompt Framework。

而是：

> **Persistent Intelligent Runtime**

Runtime：

永远存在。

Provider：

可以替换。

Capability：

不断扩展。

Memory：

持续积累。

Workflow：

持续进化。

---

# Core Philosophy

Agent

≠

LLM

LLM

≠

Runtime

Robot

≠

Agent

Capability

≠

Runtime

因此：

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
Capability

>

Tool Calling
```

```
Memory

>

Context Window
```

---

# Runtime Responsibilities

Runtime：

只负责：

```
Session

Lifecycle

State

Workflow

Memory

Context

Scheduler

Plugin

Provider

Security

Permission
```

Runtime：

绝不负责：

```
Robot

Vision

IK

WBC

MPC

ROS

Simulation
```

这些：

全部属于：

Capability。

---

# Architecture

```
User

↓

Conversation Layer

↓

Goal Layer

↓

Workflow Layer

↓

Runtime Core

↓

Provider Manager

↓

Plugin Manager

↓

Capability Layer

↓

Operating System
```

其中：

Runtime：

永远位于中心。

---

# Provider

Provider：

代表：

推理能力。

例如：

```
Claude

GPT

Gemini

Local LLM
```

Provider：

统一接口。

例如：

```
complete()

stream()

embedding()

reason()
```

Provider：

没有：

Memory。

没有：

Workflow。

没有：

Tool。

---

# Native Runtime

Runtime：

才是真正的：

Agent。

Agent：

由：

```
Memory

Workflow

State

Planner

Scheduler

Provider
```

共同组成。

不是：

一个：

LLM。

---

# Capability

Capability：

全部放在：

Plugin。

例如：

```
Robot

Linux

Web

Git

Compiler

Simulation

Database
```

Runtime：

不知道：

Robot。

Runtime：

只知道：

Capability。

---

# Robot

Robot：

只是：

一个：

Capability Package。

例如：

```
auro-robot

├── model

├── ros

├── mujoco

├── pinocchio

├── ik

├── fk

├── dynamics

├── wbc

├── mpc

└── diagnostics
```

以后：

机器人开发者：

维护这里。

不是：

Runtime。

---

# Cognition

未来：

认知：

也应该：

Plugin。

例如：

```
belief

goal

reflection

attention

motivation

emotion

curiosity

planning
```

全部：

独立。

例如：

```
auro-cognition
```

里面：

可以存在：

不同理论。

例如：

```
Global Workspace

Predictive Processing

Active Inference

SOAR

ACT-R

BDI
```

Runtime：

不依赖：

任何一种。

---

# Plugin SDK

真正开放的是：

SDK。

不是：

代码。

例如：

```
trait Plugin {

init()

run()

shutdown()
}
```

Runtime：

只负责：

生命周期。

---

# Provider SDK

任何人：

都可以：

开发：

```
Claude Provider

Codex Provider

Gemini Provider

DeepSeek Provider

Local Provider
```

无需：

修改 Runtime。

---

# Robot SDK

机器人公司：

可以：

维护：

```
Robot SDK
```

例如：

```
Leju

Unitree

AgiBot

Boston Dynamics

Figure
```

甚至：

各自：

维护：

自己的：

Capability。

---

# Memory SDK

不同 Memory：

例如：

```
SQLite

Postgres

Qdrant

Milvus

Neo4j

Filesystem
```

全部：

Plugin。

---

# Workflow SDK

Workflow：

不是：

Prompt。

Workflow：

属于：

Runtime。

例如：

```
Diagnose Robot

↓

Read Logs

↓

Analyze

↓

Verify

↓

Report
```

未来：

Claude：

可以：

帮助：

生成 Workflow。

Runtime：

负责：

沉淀 Workflow。

---

# Self Evolution

Self Evolution：

不是：

模型：

自己学习。

而是：

Runtime：

持续：

积累：

```
Workflow

Memory

Trace

Knowledge

Capability
```

形成：

越来越好的：

Runtime。

因此：

Self Evolution：

属于：

Runtime。

不是：

LLM。

---

# Open Source Strategy

不要：

一个仓库。

建议：

Organization：

```
auro-runtime

auro-provider

auro-plugin-sdk

auro-memory

auro-workflow

auro-cognition

auro-robot

auro-system

auro-cli

auro-ui

auro-examples
```

每个：

独立维护。

---

# Community

Runtime：

吸引：

系统工程师。

---

Provider：

吸引：

AI 工程师。

---

Robot：

吸引：

机器人开发者。

---

Cognition：

吸引：

哲学。

认知科学。

神经科学。

心理学。

---

Workflow：

吸引：

AI Agent。

Prompt。

Automation。

开发者。

---

Memory：

吸引：

数据库。

知识图谱。

RAG。

开发者。

---

# Governance

未来：

维护者：

不应该：

全部：

修改 Runtime。

Runtime：

保持：

最小。

稳定。

Plugin：

快速演进。

因此：

Runtime：

更像：

Linux Kernel。

Plugin：

更像：

Linux Driver。

---

# Long-term Goal

第一阶段：

Robot Engineering Runtime。

第二阶段：

Persistent Runtime。

第三阶段：

Evolution Platform。

最终：

Auro Runtime：

不是：

最大的 Agent。

而是：

智能系统共同运行的平台。

---

# One Sentence

> **Auro Runtime is not another AI Agent. It is a persistent intelligent runtime where cognition, memory, workflows, providers, and capabilities can evolve independently while sharing a stable execution core.**

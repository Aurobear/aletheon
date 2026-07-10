目前这四份更像是第一阶段的重构计划，它们没有覆盖整个 Aletheon 的理论体系，而是专门回答一个问题：

如何把现在的 Runtime 收缩成真正的 Executive。

它们之间的关系是这样的：

RFC-010
    │
    ▼
Executive 应该是什么

RFC-011
    │
    ▼
各个 Core Subsystem 如何划分

RFC-012
    │
    ▼
它们之间如何通信、如何组织推理(Harness)

RFC-013
    │
    ▼
如何一步一步把现在代码迁过去
RFC-010 Executive Refactor

这是总体设计原则。

主要包含：

为什么要收缩 Runtime

例如：

Runtime 现在：

Session
Memory
ReAct
Hook
Skill
Agent
Evolution
Provider
Daemon

↓

God Object

然后定义：

Executive 永远只负责：

Lifecycle

Scheduler

Supervisor

Communication

Resource

Authority

以及：

哪些东西一定不能进入 Executive。

这一篇主要属于：

设计哲学

RFC-011 Core Subsystems

这是：

整个 Agent 应该有哪些核心模块。

目前里面主要包括：

Executive

Cognit

Dasein

Mnemosyne

Corpus

Metacog

以及：

每个模块：

负责什么。

拥有什么状态。

以后不能互相修改什么。

例如：

Executive

拥有：

Lifecycle

Scheduling

Resource

而：

Mnemosyne

拥有：

Memory

Index

Association

Replay

别人不能直接改。

这一篇属于：

模块边界

RFC-012 Communication + Harness

这是昨天讨论最多的。

包括：

Communication Fabric：

Command

Query

Event

Stream

为什么：

不能全部叫 Event。

以及：

Harness：

到底是什么。

为什么以后：

不是：

ReAct Loop

而应该：

Harness

例如：

Coding Harness

Research Harness

Robot Harness

OS Harness

这一篇属于：

模块如何合作。

RFC-013 Refactor Roadmap

这个就是：

真正开始改代码。

例如：

第一步：

拆：

RequestHandler

第二步：

Memory

迁出去。

第三步：

ReAct

迁出去。

第四步：

Skill

迁出去。

第五步：

Gateway

迁出去。

最后：

才：

runtime

↓

executive

这是：

施工路线图。

但是我认为还缺很多

其实这四篇。

只覆盖了：

Runtime 收缩。

如果按照昨天以及今天我们讨论。

整个 Aletheon。

至少应该有十五篇以上。

例如：

第一部分 Philosophy

这是整个项目最重要的。

例如：

RFC-001

Why Aletheon

为什么：

不是：

Agent。

不是：

Workflow。

不是：

Copilot。

而是：

Persistent Cognitive System。

第二部分

Executive

这一部分。

刚刚已经写了。

第三部分

Cognit。

这一部分。

其实我们昨天聊得最多。

应该详细定义：

例如：

Planner

Reasoner

Executor

Verifier

Reflector

Learner


以及：

为什么：

不是：

ReAct。

以后：

为什么：

Harness。

第四部分

Mnemosyne。

目前。

Memory。

其实讨论还不够。

例如：

以后：

Replay。

Dream。

Consolidation。

Decay。

Association。

Importance。

Background Task。

这些。

全部没写。

第五部分

Dasein。

我觉得。

这里还能继续扩展很多。

例如：

Identity

↓

Value

↓

Goal

↓

Boundary

↓

Narrative

↓

Continuity


甚至：

以后：

Long-term Goal。

Self Desire。

都应该在这里。

第六部分

Communication Fabric。

目前：

也只是：

Message。

实际上：

还应该定义：

例如：

Envelope

Mailbox

Routing

Topic

Subscription

Request

Response

Signal


全部协议。

第七部分

Harness。

我觉得。

这个以后会变成：

Aletheon 最大创新。

例如：

如何：

动态：

组合：

Planner。

Reasoner。

Verifier。

Memory。

第八部分

Capability。

目前：

Corpus。

其实。

还可以继续拆：

例如：

Driver

↓

Capability

↓

Tool

↓

Workflow

↓

Skill


以后：

所有：

Tool。

都是：

Capability。

组合出来。

第九部分

Agent。

Agent。

到底：

是不是：

一个：

Subsystem。

还是：

一个：

Process。

还是：

一个：

Actor。

这里：

其实昨天还没有讨论。

我真正建议的是：

不要继续写：

普通架构文档。

而是：

建立：

Aletheon RFC。

例如：

docs/

architecture/

RFC-001 Philosophy

RFC-002 Executive

RFC-003 Cognit

RFC-004 Dasein

RFC-005 Mnemosyne

RFC-006 Corpus

RFC-007 Communication Fabric

RFC-008 Harness

RFC-009 Capability

RFC-010 Agent

RFC-011 Session

RFC-012 Resource

RFC-013 Evolution

RFC-014 Plugin

RFC-015 Scheduler

RFC-016 Security

RFC-017 Memory Pipeline

RFC-018 Long-term Goals

RFC-019 Distributed Cognition

RFC-020 Architecture Language
我认为还有一个比这些更重要的东西

其实昨天晚上我一直在想一件事。

我们现在讨论的已经不是：

怎么写一个 Agent。

而是在定义：

Agent 世界里的基本概念（Primitive）。

Linux 有：

Process

Thread

File

Socket

Signal

Memory

ROS 有：

Node

Topic

Service

Action

Kubernetes 有：

Pod

Deployment

Service

Controller

而 Aletheon 还缺自己的 Primitive 集合。

我认为下一份 RFC 不应该继续写代码，而应该专门定义：

Aletheon Primitive

例如：

Executive
Cognit
Dasein
Mnemosyne
Corpus
Harness
Envelope
Capability
Intent
Experience
Narrative
Commitment

一旦这些 Primitive 定义稳定，后面的代码、通信协议和插件体系都会自然围绕它们演化。这可能会成为整个项目最重要的一份架构文档。
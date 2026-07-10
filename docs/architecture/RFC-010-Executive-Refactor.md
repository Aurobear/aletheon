# RFC-010 Executive 重构路线

## 目标

将当前 Runtime 收缩为 Executive，使其只负责系统运行，不负责认知实现。

> Executive 负责 **How the system runs**，而不是 **How the agent
> thinks**。

## Executive 唯一职责

-   Lifecycle
-   Scheduler
-   Supervisor
-   Resource Manager
-   Communication Fabric
-   Authority / Permission

Executive 禁止直接负责：

-   Planner
-   Reasoner
-   Prompt
-   Memory Recall
-   Reflection
-   Skill Matching
-   Tool Implementation
-   Evolution Strategy

## 重构原则

1.  状态归属唯一。
2.  Executive 不直接持有具体 Memory、Tool、LLM。
3.  所有核心模块通过 trait 与消息通信。
4.  Composition Root 放在 aletheon binary。

## 第一阶段

1.  拆分 RequestHandler。
2.  建立 CognitOps / DaseinOps / MnemosyneOps / CorpusOps。
3.  Runtime 仅持有这些接口。

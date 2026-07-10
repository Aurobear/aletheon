# RFC-012 Communication Fabric 与 Harness

## Communication Fabric

统一四种语义：

-   Command
-   Query
-   Event
-   Stream

不要所有通信都称为 Event。

Envelope 建议：

``` rust
enum MessageKind {
    Command,
    Query,
    Event,
    Stream,
}
```

------------------------------------------------------------------------

## Harness

Harness 负责组织认知，而不是 Executive。

示例：

Goal ↓ Context ↓ Planner ↓ Reasoner ↓ Executor ↓ Verifier ↓ Reflector ↓
Memory Update

未来不同 Harness：

-   Coding Harness
-   Research Harness
-   Robot Harness
-   OS Harness

Harness 可以是 Linear，也可以是 Graph。

------------------------------------------------------------------------

## Executive 与 Harness

Executive： - 调度 - 超时 - 生命周期

Harness： - 节点 - 顺序 - 重试 - 分支 - 并发

# RFC-013 重构路线图

## Phase 0

-   写架构约束
-   建立 trait

## Phase 1

拆 RequestHandler：

目标：

-   字段 \<=10
-   引入 CoreSystems

``` rust
struct CoreSystems {
    cognit,
    dasein,
    mnemosyne,
    corpus,
    metacog,
}
```

## Phase 2

迁移 Runtime Memory -\> Mnemosyne

## Phase 3

迁移 ReAct -\> Cognit

重命名：

ReActLoop ↓

LinearCognitiveHarness

## Phase 4

迁移 Skill、Hook、Tool

## Phase 5

拆 Gateway

Gateway： - RPC - Unix Socket - Streaming

Executive： - Lifecycle - Scheduler - Supervisor

## Phase 6

最后再改名 Runtime -\> Executive

不要提前改名。

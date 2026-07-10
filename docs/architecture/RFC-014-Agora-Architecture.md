# RFC-014 Agora Architecture

## Purpose

Agora is the shared cognitive workspace of Aletheon.

It is **not** long-term memory and **not** a planner. It is the active
cognitive environment in which all reasoning occurs.

## Position

Executive ↓ Dasein ↓ Cognit ↓ Agora ↓ Corpus ↓ Mnemosyne

## Responsibilities

-   Working Memory
-   Blackboard
-   Shared Context
-   Attention State
-   Reasoning Trace
-   Hypothesis
-   Evidence
-   Task Graph
-   Scratchpad
-   Temporary Variables
-   Tool Outputs
-   Sub-Agent Results

## Lifecycle

Input → Context Build → Recall Injection → Reasoning → Planning →
Execution → Reflection → Commit to Mnemosyne

## Suggested Modules

src/ ├── workspace/ ├── attention/ ├── scratchpad/ ├── blackboard/ ├──
context/ ├── task_graph/ ├── observation/ ├── artifact/ └── api/

## Trait

``` rust
trait AgoraOps {
    fn publish();
    fn recall();
    fn update();
    fn snapshot();
    fn clear();
}
```

## Principles

-   Shared but scoped
-   Session isolated
-   Fast access
-   Never persistent by itself

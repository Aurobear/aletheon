# Multi-Agent Orchestration Engine

> Migrated from `docs/design/orchestration/orchestration-engine.md` — code paths updated to match actual crate names (fabric, cognit, corpus, dasein, mnemosyne, metacog, interact, executive)

> Pluggable multi-agent collaboration orchestration system, supporting Selector/Handoff/DiGraph strategies, with delegation unified as Tool calls.

**Module:** 06
**Crate:** `executive`
**Code location:** `executive/src/impl/orchestration/`
**Related modules:** [react-loop.md](react-loop.md), [session.md](session.md)
**Last Updated:** 2026-06-14

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| Agent trait | Implemented | `executive/src/impl/orchestration/agent.rs` | Core agent abstraction |
| AgentRegistry | Implemented | `executive/src/impl/orchestration/registry.rs` | Agent registration and lookup |
| DelegateTool | Implemented | `executive/src/impl/orchestration/delegate.rs` | Delegation as tool call |
| SelectorStrategy | Implemented | `executive/src/impl/orchestration/selector.rs` | Agent selection strategy |
| HandoffStrategy | Implemented | `executive/src/impl/orchestration/handoff.rs` | Agent handoff pattern |
| TerminationConditions | Implemented | `executive/src/impl/orchestration/termination.rs` | Stop conditions for orchestration |
| IterationBudget | Implemented | `executive/src/impl/orchestration/budget.rs` | Token/iteration budget control |
| DiGraph | Implemented | `executive/src/impl/orchestration/digraph/` | DAG-based orchestration graph (edge, node, state) |
| Built-in agents | Implemented | `executive/src/impl/orchestration/builtin/` | fs_agent, net_agent, code_agent |
| ConfigAgent | Implemented | `executive/src/impl/orchestration/config_agent.rs` | Configuration-driven agent |

---

## 1. Overview

The orchestration engine coordinates multiple specialized agents to collaboratively complete complex tasks. Core innovation comes from AutoGen's pluggable orchestration strategies and CrewAI's "delegation as tool" pattern, unifying agent delegation as ordinary Tool calls, simplifying multi-agent interaction interfaces.

The orchestration engine delivers progressively by phase:

| Phase | Strategy | Use Case |
|-------|----------|----------|
| Phase 1 | SingleAgent | Single agent + tool calls, simplest start |
| Phase 4a | Selector | LLM routing to select agent, suitable for file/network/process dispatch |
| Phase 4b | Handoff (Swarm) | Explicit delegation, suitable for complex task decomposition |
| Phase 6 | DiGraph | DAG workflow + conditional edges + parallel fan-out |

---

## 2. Current Design

### 2.1 Orchestration Strategies (Process Strategy)

**ProcessStrategy** — Four orchestration strategies:
- **SingleAgent** — One main agent runs ReAct loop (Phase 1)
- **Selector** — LLM routing to select agent (Phase 4a)
- **Handoff (Swarm)** — Explicit delegation, delegation = one Tool call (Phase 4b)
- **DiGraph** — DAG workflow + conditional edges + parallel fan-out (Phase 6)

### 2.2 Agent Registry

**Agent trait** — Core agent abstraction, containing id, capabilities, tools, on_messages.
- Code location: `executive/src/impl/orchestration/agent.rs`

**AgentRegistry** — Agent registry, supports registration, lookup, lifecycle management.
- Code location: `executive/src/impl/orchestration/registry.rs`

```
+--------------+----------------+------------------+
| Agent ID     | Capabilities   | Available Tools   |
+--------------+----------------+------------------+
| coordinator  | Task split/rout| delegate, plan   |
| fs_agent     | Filesystem ops | read,write,grep  |
| net_agent    | Network ops    | curl,ssh,dns     |
| proc_agent   | Process mgmt   | ps,kill,systemd  |
| code_agent   | Code execution | bash,python      |
| ui_agent     | UI automation  | click,type,snap  |
+--------------+----------------+------------------+
```

### 2.3 Delegation as Tool (DelegateTool)

**DelegateTool** — Inspired by CrewAI's core innovation, agent delegation unified as Tool call.
- Code location: `executive/src/impl/orchestration/delegate.rs`

```
coordinator: "Help me check nginx config"
  -> delegate(fs_agent, "Read /etc/nginx/nginx.conf")
  -> fs_agent reads file, returns content
  -> coordinator gets result, continues reasoning
```

### 2.4 Termination Conditions

**TerminationCondition** — Inspired by AutoGen's composable termination conditions, supports And/Or composition.
- Code location: `executive/src/impl/orchestration/termination.rs`
- Types: MaxIterations, MaxTokens, Timeout, AndCondition, OrCondition

### 2.5 Safety Guardrails

Inspired by CrewAI's Guardrail pattern, each agent's output is validated:
1. Command whitelist/blacklist check
2. Permission level verification (L0-L3)
3. Side effect estimation (file modification/network request/process operation)
4. Failure -> retry or escalate to human confirmation

---

## 3. Identified Defects

### P2: Sub-Agent Independent Budget

**Problem:** Current design has delegated agents sharing parent agent's full context window and token budget. A deep delegation chain or parallel fan-out may exhaust budget with no graceful degradation.

### 3.1 P0: Sub-Agent Permission Inheritance and Security Model Disconnect

**Problem:** Security model and orchestration engine each define permission control mechanisms, but lack integration between them.

### 3.2 P1: Sub-Agent Shared State Has No Isolation

**Problem:** Sub-agents directly inherit parent agent's full memory context with no scope isolation.

### 3.3 P2: Sub-Agent Observability Missing

**Problem:** Active agent registry is only a concept, interrupt propagation has no protocol, pause/resume under resource pressure not designed.

### 3.4 P2: DiGraph Design Insufficient

**Problem:** DiGraph as Phase 6 core feature, design stays at conceptual level — no node definition format, no edge condition syntax, no inter-node state transfer mechanism.

---

## 4. Improved Design

### 4.1 IterationBudget — Independent Iteration Budget

Core structure: `IterationBudget` — lightweight consume/refund counter, thread-safe (AtomicUsize). Each sub-agent instance holds independent budget.

Budget operations: `consume()` attempts to consume one iteration, `refund()` returns iterations that shouldn't be charged (failed retries, execute_code sandbox rounds, 0-API-call timeouts), `remaining()` / `used()` read-only queries.

```
New design (independent mode):
  Parent Agent:  IterationBudget(90)     <- config parent.max_iterations
  +-- fs_agent:   IterationBudget(50)    <- config delegation.max_iterations
  +-- net_agent:  IterationBudget(50)
  +-- code_agent: IterationBudget(50)
  -> total iterations can reach 90+50+50+50=240, but each sub-agent max 50
  -> one sub-agent exhaustion doesn't affect others or parent
```

Code location: `executive/src/impl/orchestration/budget.rs`

### 4.2 Integration into DelegateTool

Key design parameters:
- `DELEGATE_BLOCKED_TOOLS` — tools forbidden for sub-agents
- `MAX_DELEGATE_DEPTH` — maximum delegation depth, default 1 (no grandchild agents)
- `DelegationConfig` — max_iterations (50), max_concurrent_children (3), max_depth (1), provider_override

### 4.3 Parallel Fan-Out Budget Allocation

Each sub-agent gets independent budget, no "equal split" logic. Parallel execution uses `JoinSet` + `Semaphore(max_concurrent_children)` to control concurrency limit. Results sorted by `task_index` to preserve order. Interrupt propagation: parent agent interrupt -> `abort_all` -> sub-agents stop.

### 4.4 Sub-Agent Permission Inheritance and Security Integration

Remove hardcoded `DELEGATE_BLOCKED_TOOLS`, replaced by `PolicyEngine` dynamically deriving sub-agent permissions.

Default degradation rules:
- Parent L3 -> Child L2 (forbid dangerous operations)
- Parent L2 -> Child L1 (forbid system directory writes)
- Parent L1 -> Child L0 (read-only)

### 4.5 DiGraph Complete Execution Specification

**Node types:** Agent, Branch (conditional branch), HumanApproval, SubGraph.

**Edges and conditions:** JSONPath + comparison operators, supports Always/When(expr)/Default three edge types.

**State transfer:** `GraphState` as shared typed dict for all nodes, upstream outputs stored with `NodeId` as key.

**Error handling:** Each node configures `RetryPolicy` (max_retries + BackoffStrategy), after exhaustion handle by OnExhausted strategy (FailGraph/SkipNode/Escalate).

**Checkpoint and recovery:** Auto-checkpoint to `~/.aletheon/checkpoints/` after each node completes. On recovery, completed nodes are not re-executed.

**Parallel fan-out-join:** `FanOutNode` generates N parallel nodes, `JoinStrategy` supports All/Any/FirstN(n)/TimeoutAll.

---

## 5. Implementation Notes

- **Phase 4a (Selector)**: No IterationBudget needed, single agent routing suffices.
- **Phase 4b (Handoff)**: Introduces IterationBudget, each delegation creates independent budget (default 50 iterations). Sub-agents disable `DELEGATE_BLOCKED_TOOLS`, tool set stripped at construction. Sub-agents have independent context, parent only sees delegation call + final summary.
- **Phase 6 (DiGraph)**: Parallel fan-out uses `JoinSet` + `Semaphore` for concurrency control. Each sub-agent has independent budget. Results sorted by `task_index`. Interrupt propagation: parent interrupt -> `abort_all` -> sub-agents stop.
- **Budget exhaustion**: Sub-agent iterations exhausted -> stop execution -> return produced summary via `ToolResult::partial()`.
- **Refund mechanism**: Return unconsumed iterations for "free" operations: failed retries, execute_code sandbox rounds, 0-API-call timeouts.

---

## 6. References

| Source | Borrowed Content |
|--------|-----------------|
| **AutoGen** (`autogen_agentchat/teams/`) | Selector/Swarm/DiGraph orchestration strategies, composable termination conditions |
| **CrewAI** (`crewai/crew.py:159`) | Delegation as tool (DelegateTool), Process strategy, Guardrail |
| **Hermes IterationBudget** (`hermes-agent/agent/iteration_budget.py`) | Independent budget (default 50 iterations), consume/refund thread-safe counter |
| **Hermes DelegateTool** (`hermes-agent/tools/delegate_tool.py`) | Batch parallel delegation, DELEGATE_BLOCKED_TOOLS, MAX_DEPTH=1 |
| **LangGraph** (`langgraph/pregel/`) | Checkpoint recovery, per-node strategy (RetryPolicy/CachePolicy/TimeoutPolicy) |

---

## Implementation Summary

**Code location:** `executive/src/impl/orchestration/`

**Key types/traits implemented:**
- `Agent` trait (`agent.rs`) — core agent abstraction with id, capabilities, tools, on_messages
- `AgentRegistry` (`registry.rs`) — thread-safe agent registration and lookup
- `DelegateTool` (`delegate.rs`) — delegation as tool call with depth check, budget creation, tool filtering
- `SelectorStrategy` (`selector.rs`) — LLM-based agent selection
- `HandoffStrategy` (`handoff.rs`) — explicit agent handoff pattern
- `TerminationCondition` trait (`termination.rs`) — composable conditions
- `IterationBudget` (`budget.rs`) — thread-safe consume/refund counter
- `DiGraph` (`digraph/`) — DAG orchestration with edge, node, state submodules
- `ConfigAgent` (`config_agent.rs`) — configuration-driven agent definition
- Built-in agents (`builtin/`) — fs_agent, net_agent, code_agent

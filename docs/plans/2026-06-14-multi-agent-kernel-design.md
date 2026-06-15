# Multi-Agent Kernel Design

**Date:** 2026-06-14
**Status:** Approved
**Branch:** auro/feat/engine-emit-tool-observation-event

## 1. Problem Statement

Aletheon has a substantial Layer 4 orchestration system (DelegateTool, HandoffStrategy, SelectorStrategy, DiGraph) but lacks the Layer 2 Agent Kernel primitives that an OS-level autonomous agent needs. The existing code has critical gaps:

- **No inter-agent communication** — agents are isolated, coordination is text-based pattern matching
- **No parallel execution** — DiGraph executes sequentially despite defining JoinStrategy
- **No thread-like abstraction** — no lightweight context-sharing for sub-agents
- **No global resource management** — per-agent token budgets with no system-wide pool
- **No lifecycle supervision** — no crash detection, restart, or health monitoring
- **Serial pulse dispatch** — all agents process pulses one at a time

## 2. Design Goals

1. **Linux-inspired process model** — AgentProcess (进程) + AgentFork (线程)
2. **Kernel primitives** — spawn, fork, wait, kill, suspend, resume, send, recv, broadcast
3. **Harness abstraction** — turn-level executor, separated from context preparation
4. **Global resource management** — system-wide token pool with priority scheduling
5. **Lifecycle supervision** — crash detection, exponential backoff restart, health monitoring
6. **Layered architecture** — Kernel (Layer 2) provides primitives, Orchestration (Layer 4) provides strategy

## 3. Architecture

```
Layer 5: Computer Agent (OS 级自治代理)
Layer 4: Orchestration (策略层 — 调用 Kernel API)
Layer 3: Agent Runtime (Harness + Context + Tools)
Layer 2: Agent Kernel (进程原语 + IPC + 调度 + 资源)
Layer 1: LLM SDK (LlmScheduler + LlmPulse)
```

### 3.1 Layer 2: Agent Kernel

The Kernel provides **primitives**, not **policy**. It manages agent entities themselves, not the logic between agents.

#### 3.1.1 System Calls

| Primitive | Behavior | Linux Analogy |
|-----------|----------|---------------|
| `spawn(config)` | Create independent AgentProcess, return Pid | `fork() + exec()` |
| `fork(parent_pid, directive)` | Copy parent context, create lightweight AgentFork | `fork()` (COW) |
| `wait(pid)` | Block until child completes, return result | `waitpid()` |
| `kill(pid)` | Terminate process, emit SIGTERM event | `kill(pid, SIGTERM)` |
| `suspend(pid)` | Pause process (no more pulses) | `SIGSTOP` |
| `resume(pid)` | Resume process | `SIGCONT` |
| `send(pid, msg)` | Send message to specific process | `write(pipe)` |
| `recv()` → msg | Receive message addressed to self (blocking) | `read(pipe)` |
| `broadcast(event)` | Broadcast event to EventBus | `signal()` |

#### 3.1.2 AgentProcess (进程)

Independent agent entity with isolated context, communicating via EventBus IPC.

```rust
pub struct AgentProcess {
    pid: Pid,
    parent: Option<Pid>,
    children: RwLock<Vec<Pid>>,
    state: AgentProcessState,
    budget: TokenBudget,
    config: AgentProcessConfig,
    harness: Arc<dyn AgentHarness>,
    engine: Option<Engine>,
    message_queue: AsyncQueue<Message>,
    last_heartbeat: AtomicInstant,
    event_bus: Arc<dyn EventBus>,
}

pub enum AgentProcessState {
    Idle,
    Thinking,
    Acting,
    Reflecting,
    Sleeping,
    Suspended,
    Terminated,
}
```

#### 3.1.3 AgentFork (线程)

Lightweight context-sharing sub-agent. Inherits parent's conversation history, system prompt, and tools (read-only). One-shot: receives a directive, returns a notification.

```rust
pub struct AgentFork {
    pid: Pid,
    parent_pid: Pid,
    inherited_messages: Arc<Vec<Message>>,
    inherited_system_prompt: Arc<String>,
    inherited_tools: Arc<Vec<Tool>>,
    harness: Arc<dyn AgentHarness>,  // inherited from parent
    directive: String,
    budget: TokenBudget,
    state: AgentForkState,
    result: Option<ForkResult>,
}

pub struct ForkDirective {
    pub prompt: String,
    pub inherit_history: bool,
    pub inherit_tools: bool,
    pub budget_ratio: f64,  // default 0.3
}

pub enum AgentForkState {
    Running,
    Completed,
    Failed(String),
}
```

Key properties:
- **Context inheritance**: Fork shares parent conversation history (read-only)
- **Prompt cache optimization**: All parallel forks share identical API prefix
- **One-shot communication**: Fork receives one directive, returns one notification
- **Cannot fork itself**: `can_spawn = false`, prevents recursion
- **Budget inheritance**: Takes a portion of parent's TokenBudget

#### 3.1.4 AgentSupervisor (生命周期管理)

Kernel component for health monitoring and lifecycle management. Analogous to Linux init/systemd.

```rust
pub struct AgentSupervisor {
    supervised: RwLock<HashMap<Pid, SupervisedProcess>>,
    restart_policy: RestartPolicy,
    event_bus: Arc<dyn EventBus>,
    health_check_interval: Duration,
}

pub struct RestartPolicy {
    pub initial_delay: Duration,       // 2s
    pub max_delay: Duration,           // 120s
    pub backoff_multiplier: f64,       // 2.0
    pub fast_fail_window: Duration,    // 10s
    pub fast_fail_threshold: u32,      // 5
    pub permanent_exit_codes: Vec<i32>, // [78]
}
```

Restart state machine:
```
Process crash
    → Check exit code → Permanent error (exit 78) → Parked
    → Check window → Fast fail (5 crashes/10s) → Parked
    → Calculate delay = min(initial * backoff^count, max)
    → sleep(delay) → respawn → Running
    → Reset restart_count (after running > 30s)
```

#### 3.1.5 GlobalTokenPool (全局资源管理)

System-wide token pool shared by all agents, with priority-based allocation.

```rust
pub struct GlobalTokenPool {
    total_budget: AtomicU32,
    allocated: AtomicU32,
    priority_queue: Mutex<BinaryHeap<PriorityClaim>>,
}

impl GlobalTokenPool {
    pub fn claim(&self, pid: Pid, requested: u32, priority: u8) -> u32;
    pub fn release(&self, pid: Pid, unused: u32);
}
```

### 3.2 Layer 3: Agent Runtime

#### 3.2.1 AgentHarness (Turn 执行器)

Single agent turn executor. Core layer prepares everything (prompt, tools, context), harness only runs.

```rust
#[async_trait]
pub trait AgentHarness: Send + Sync {
    fn supports(&self, ctx: &HarnessContext) -> HarnessBid;
    async fn run_attempt(&self, params: AttemptParams) -> AttemptResult;
}

pub struct AttemptParams {
    pub prompt: String,
    pub tools: Vec<Tool>,
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub budget: TokenBudget,
    pub runtime_plan: RuntimePlan,
    pub on_partial_reply: Option<Callback>,
    pub on_tool_event: Option<Callback>,
}

pub struct RuntimePlan {
    pub tool_policy: ToolPolicy,
    pub max_turns: u32,
    pub timeout: Duration,
    pub compaction_threshold: usize,
}
```

Design principles (from OpenClaw):
1. Harness does not own context — core layer assembles prompt, filters tools, manages history
2. Harness does not route — provider/model selection done at core layer
3. Harness only executes — receives prepared params, calls LLM, runs tools, returns result
4. Harness is swappable — different provider/model can use different harness
5. Harness cannot span turns — each turn independent, state managed by core layer

Built-in implementations:
- `ReActHarness` — default ReAct loop executor
- `CliHarness` — proxies to external CLI tools (claude-cli, etc.)

Harness selection policy (from OpenClaw):
1. Agent config specifies `harness_id` → use directly
2. Provider-level runtime policy → select by provider
3. Auto mode: all registered harnesses call `supports()` → highest priority wins
4. Fallback to built-in `ReActHarness`

### 3.3 IPC Mechanisms

Three IPC mechanisms, analogous to Linux pipes, signals, and shared memory.

#### 3.3.1 MessageChannel (管道)

Point-to-point message channel for task assignment and result collection.

```rust
pub struct Message {
    pub from: Pid,
    pub to: Pid,
    pub kind: MessageKind,
    pub payload: String,
    pub timestamp: Instant,
}

pub enum MessageKind {
    Task,
    Result,
    Query,
    Response,
    Signal(Signal),
}

pub enum Signal {
    Abort,
    Pause,
    Resume,
    HealthCheck,
    BudgetWarning,
}
```

#### 3.3.2 EventBus Topic (信号广播)

Topic-based broadcast using existing EventBus. Agent lifecycle and inter-agent communication topics.

```
AgentSpawned { parent_pid, child_pid, is_fork }
AgentCompleted { pid, result }
AgentFailed { pid, error }
AgentStateChanged { pid, old_state, new_state }
AgentMessage { from, to, kind, payload }
AgentGroupMessage { group_id, from, payload }
```

#### 3.3.3 SharedScratchpad (共享内存)

Shared key-value store for agents working on the same task.

```rust
pub struct SharedScratchpad {
    task_id: String,
    entries: RwLock<HashMap<String, ScratchpadEntry>>,
}

impl SharedScratchpad {
    pub async fn read(&self, key: &str) -> Option<String>;
    pub async fn write(&self, key: &str, value: String, writer: Pid);
    pub async fn delete(&self, key: &str);
    pub async fn list_keys(&self) -> Vec<String>;
}
```

IPC selection guide:

| Scenario | Mechanism | Reason |
|----------|-----------|--------|
| Parent-child task/result | MessageChannel | Point-to-point, ordered |
| Process state change | EventBus topic | Broadcast, decoupled |
| Same-task agent coordination | SharedScratchpad | Shared read/write, low overhead |
| Fork result return | EventBus (AgentCompleted) | One-shot, async |

### 3.4 Layer 4: Orchestration Adaptation

Existing orchestration layer adapts to call Kernel API instead of managing processes directly.

```rust
// HandoffStrategy — uses Kernel primitives
impl HandoffStrategy {
    async fn execute(&self, task: &str) -> Result<String> {
        let mut current_pid = self.initial_pid;
        loop {
            self.kernel.send(current_pid, Message::Task(task.into()));
            let result = self.kernel.wait(current_pid).await?;
            if let Some(next) = self.parse_handoff(&result.response) {
                current_pid = self.kernel.find_by_name(&next);
            } else {
                return Ok(result.response);
            }
        }
    }
}

// SelectorStrategy — uses Kernel find_by_capability
impl SelectorStrategy {
    async fn select_and_run(&self, task: &str) -> Result<String> {
        let candidates = self.kernel.find_by_capability(task);
        let selected = self.llm_select(candidates, task).await?;
        self.kernel.send(selected, Message::Task(task.into()));
        self.kernel.wait(selected).await
    }
}

// DiGraph — uses Kernel spawn + wait + SharedScratchpad
impl DiGraph {
    async fn execute(&self) -> Result<GraphState> {
        let scratchpad = self.kernel.create_scratchpad(&self.task_id);
        for node in self.topological_order() {
            let pid = self.kernel.spawn(node.agent_config());
            self.kernel.send(pid, Message::Task(node.prompt()));
            let result = self.kernel.wait(pid).await?;
            scratchpad.write(&node.id, result.output, pid).await;
        }
        Ok(scratchpad.snapshot().await)
    }
}
```

## 4. Existing Code Changes

| Existing Code | Change |
|--------------|--------|
| `AgentProcess` (process.rs) | **Upgrade**: Add Harness call, IPC send/recv, heartbeat |
| `AgentRegistry` (registry.rs) | **Upgrade**: Unified process table + fork table, add Supervisor |
| `LlmPulse` (pulse.rs) | **Upgrade**: Connect to GlobalTokenPool, priority-based allocation |
| `DelegateTool` (delegate.rs) | **Adapt**: Call `kernel.spawn()` + `kernel.wait()` |
| `HandoffStrategy` (handoff.rs) | **Adapt**: Call `kernel.send()` + `kernel.wait()` |
| `SelectorStrategy` (selector.rs) | **Adapt**: Call `kernel.find_by_capability()` + `send()` |
| `DiGraph` (digraph/) | **Adapt**: Call `kernel.spawn()` + `SharedScratchpad` |

## 5. New Files

| File | Crate | Purpose |
|------|-------|---------|
| `kernel/mod.rs` | aletheon-runtime | AgentKernel struct, system call implementations |
| `kernel/supervisor.rs` | aletheon-runtime | AgentSupervisor, RestartPolicy |
| `kernel/global_pool.rs` | aletheon-runtime | GlobalTokenPool |
| `kernel/ipc.rs` | aletheon-runtime | MessageChannel, SharedScratchpad |
| `agent/fork.rs` | aletheon-runtime | AgentFork, ForkDirective |
| `agent/harness.rs` | aletheon-runtime | AgentHarness trait, ReActHarness |
| `abi/ipc.rs` | aletheon-abi | Message, MessageKind, Signal types |

## 6. Implementation Phases

This design should be decomposed into 3 phases:

### Phase 1: Kernel Core + IPC
- `abi/ipc.rs` — Message, MessageKind, Signal types
- `kernel/mod.rs` — AgentKernel struct, spawn/fork/wait/kill/send/recv
- `kernel/ipc.rs` — MessageChannel, SharedScratchpad
- `agent/fork.rs` — AgentFork, ForkDirective
- Upgrade `AgentProcess` — add message_queue, heartbeat, IPC send/recv

### Phase 2: Harness + Resource Management
- `agent/harness.rs` — AgentHarness trait, ReActHarness
- `kernel/global_pool.rs` — GlobalTokenPool
- Upgrade `LlmPulse` — connect to GlobalTokenPool
- Upgrade `AgentRegistry` — unified process table + fork table

### Phase 3: Supervisor + Orchestration Adaptation
- `kernel/supervisor.rs` — AgentSupervisor, RestartPolicy
- Adapt `DelegateTool` → `kernel.spawn()` + `kernel.wait()`
- Adapt `HandoffStrategy` → `kernel.send()` + `kernel.wait()`
- Adapt `SelectorStrategy` → `kernel.find_by_capability()` + `send()`
- Adapt `DiGraph` → `kernel.spawn()` + `SharedScratchpad`

## 7. Notes

- `AsyncQueue` — async MPSC queue, can use `tokio::sync::mpsc` or implement custom
- `AtomicInstant` — atomic timestamp, can use `AtomicU64` storing millis since epoch
- `Callback` type in `AttemptParams` — alias for `Arc<dyn Fn(String) + Send + Sync>`

## 8. References

- **Claude Code fork-subagent**: Context-inheriting child agents, prompt cache optimization
- **Claude Code coordinator-mode**: Orchestrator-to-worker pattern, self-contained prompts
- **Claude Code daemon**: Supervisor/worker process model, exponential backoff restart
- **Claude Code token-budget**: Sustained execution via external control loop
- **Claude Code proactive**: Tick-driven autonomy, model self-scheduling
- **OpenClaw harness**: Turn-level executor abstraction, agent-core separation
- **Agent-S Manager-Worker**: DAG planning + subtask execution hierarchy
- **AutoGen pub/sub**: Event-driven agent activation, on-demand instantiation
- **AutoGen worker protocol**: Demand-driven placement, three-phase lifecycle

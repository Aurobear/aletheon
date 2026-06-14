# LLM Pulse + AgentProcess Design

> LLM as energy, Agent as process, EventBus as nervous system.

## Problem

The current architecture has three structural limitations:

1. **LLM is passive**: Components request LLM via LlmScheduler. LLM doesn't "flow" — it's pulled, not pushed.
2. **No agent entities**: There are Agent *registrations* (HashMap<String, AgentInfo>) but no process-like entities with lifecycle, state, or resources.
3. **No sub-agent spawning**: orchestration has Selector/Handoff/DiGraph but no mechanism to spawn an independent agent that runs in its own context.

## Solution: LlmPulse + AgentProcess

### Core Metaphor

```text
LlmPulse      = 能量源（心脏，周期性泵送认知能量到神经系统）
EventBus      = 神经系统（传递信号和能量）
AgentProcess  = 进程（有 PID、状态、生命周期，消耗能量来思考和行动）
Engine        = 大脑（嵌入 AgentProcess，执行 ReAct 循环）
```

### Architecture

```text
                    ┌─────────────┐
                    │   LlmPulse  │  ← 能量源（周期性脉冲）
                    │   (heart)   │
                    └──────┬──────┘
                           │ CognitivePulseEvent
                           ▼
                    ┌──────────────────────────────────────┐
                    │              EventBus                 │
                    └──┬────────┬────────┬────────┬────────┘
                       │        │        │        │
                       ▼        ▼        ▼        ▼
                   ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐
                   │Agent │ │Agent │ │Agent │ │Agent │  ← 进程
                   │  #1  │ │  #2  │ │  #3  │ │  #4  │
                   │(root)│ │(child)│ │(child)│ │(child)│
                   └──┬───┘ └──────┘ └──────┘ └──────┘
                      │ spawn
                      ▼
                   ┌──────┐
                   │Agent │
                   │  #5  │  ← 子进程
                   │(leaf)│
                   └──────┘
```

---

## Part 1: CognitivePulseEvent

New event type for LLM energy distribution.

```rust
// crates/aletheon-abi/src/evolution.rs (add to existing)

/// LLM energy pulse — broadcast periodically by LlmPulse.
/// Agents consume this energy to think and act.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitivePulseEvent {
    pub pulse_id: Uuid,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub available_tokens: u32,           // 总可用 token 预算
    pub provider_health: ProviderHealth,  // 各 provider 状态
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub name: String,
    pub available: bool,
    pub latency_ms: u64,
    pub tokens_remaining: Option<u32>,
}
```

Add `EventType::CognitivePulse` to the enum.

---

## Part 2: LlmPulse

Location: `crates/aletheon-brain/src/impl/llm/pulse.rs`

The heart of the system. Periodically broadcasts cognitive energy to EventBus.

```rust
pub struct LlmPulse {
    scheduler: Arc<LlmScheduler>,
    bus: Arc<dyn EventBus>,
    config: PulseConfig,
}

pub struct PulseConfig {
    pub interval: Duration,           // default: 30s
    pub token_budget_per_pulse: u32,  // default: 100_000
    pub health_check_timeout: Duration, // default: 5s
}

impl LlmPulse {
    /// Start the pulse loop. Runs until shutdown signal.
    pub async fn run(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(self.config.interval);
        
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    self.pulse().await;
                }
                _ = shutdown.changed() => {
                    tracing::info!("LlmPulse shutting down");
                    break;
                }
            }
        }
    }

    /// Emit one cognitive pulse to EventBus.
    async fn pulse(&self) {
        let health = self.scheduler.health_check().await;
        
        let event = CognitivePulseEvent {
            pulse_id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            available_tokens: self.config.token_budget_per_pulse,
            provider_health: health,
        };

        let concrete = ConcreteEvent::new(
            EventType::CognitivePulse,
            Priority::High,  // 能量是高优先级
            "llm_pulse".to_string(),
            Box::new(event),
        );

        if let Err(e) = self.bus.publish(Box::new(concrete)).await {
            tracing::error!("LlmPulse failed to publish: {}", e);
        }
    }
}
```

### LlmScheduler 新增 health_check

```rust
impl LlmScheduler {
    pub async fn health_check(&self) -> ProviderHealth {
        // Ping each provider, return health status
        // For now: check if provider is reachable
        ProviderHealth {
            name: self.default_provider.clone(),
            available: true,
            latency_ms: 0,
            tokens_remaining: None,
        }
    }
}
```

---

## Part 3: AgentProcess

Location: `crates/aletheon-runtime/src/impl/agent/process.rs`

A process-like entity that consumes LLM energy.

```rust
/// Agent 生命周期状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    /// 空闲，等待任务
    Idle,
    /// 正在思考（消耗 LLM 能量）
    Thinking,
    /// 正在执行工具
    Acting,
    /// 正在反思
    Reflecting,
    /// 休眠（不消耗能量）
    Sleeping,
    /// 已终止
    Terminated,
}

/// 进程式 Agent 实体
pub struct AgentProcess {
    /// 进程 ID
    pid: Pid,
    /// 当前状态
    state: AgentState,
    /// 父进程 PID（如果是子 agent）
    parent: Option<Pid>,
    /// 子进程列表
    children: RwLock<Vec<Pid>>,
    /// 能量预算管理
    energy: TokenBudget,
    /// 内嵌的 Engine（ReAct 循环）
    engine: Option<Engine>,
    /// 任务描述
    task: String,
    /// EventBus 引用
    bus: Arc<dyn EventBus>,
    /// 配置
    config: AgentProcessConfig,
}

#[derive(Debug, Clone)]
pub struct AgentProcessConfig {
    /// 每次脉冲可用的最大 token 数
    pub max_tokens_per_pulse: u32,
    /// 最大子 agent 数量
    pub max_children: usize,
    /// 空闲超时后自动休眠
    pub idle_timeout: Duration,
    /// 是否允许 spawn 子 agent
    pub can_spawn: bool,
}

/// Token 预算管理
pub struct TokenBudget {
    /// 本脉冲剩余可用 token
    remaining: AtomicU32,
    /// 总消耗
    total_consumed: AtomicU64,
}

impl AgentProcess {
    pub fn new(
        parent: Option<Pid>,
        task: String,
        bus: Arc<dyn EventBus>,
        config: AgentProcessConfig,
    ) -> Self {
        Self {
            pid: Pid::new(),
            state: AgentState::Idle,
            parent,
            children: RwLock::new(Vec::new()),
            energy: TokenBudget::new(config.max_tokens_per_pulse),
            engine: None,
            task,
            bus,
            config,
        }
    }

    /// 启动 agent：注册到 EventBus，开始接收脉冲
    pub async fn start(&mut self) -> Result<()> {
        self.state = AgentState::Idle;
        
        // 订阅 CognitivePulse
        let pid = self.pid;
        let bus = self.bus.clone();
        // ... register pulse handler
        
        // 发布启动事件
        self.bus.publish(Box::new(ConcreteEvent::new(
            EventType::AgentStarted,
            Priority::Normal,
            format!("agent:{}", pid),
            Box::new(AgentStartedPayload { pid: pid.as_u64(), task: self.task.clone() }),
        ))).await?;

        Ok(())
    }

    /// 收到认知脉冲时：用能量执行一轮思考
    pub async fn on_pulse(&mut self, pulse: &CognitivePulseEvent) -> Result<()> {
        if self.state == AgentState::Idle || self.state == AgentState::Sleeping {
            return Ok(());  // 空闲/休眠不消耗能量
        }
        if self.state == AgentState::Terminated {
            return Ok(());
        }

        // 从脉冲中获取能量
        let budget = self.energy.claim(pulse.available_tokens);
        if budget == 0 {
            return Ok(());  // 本脉冲没有可用能量
        }

        // 执行一轮 ReAct
        if let Some(engine) = &mut self.engine {
            self.state = AgentState::Thinking;
            let result = engine.run_turn_with_budget(budget).await?;
            
            // 根据结果更新状态
            match result {
                TurnResult::NeedTool => self.state = AgentState::Acting,
                TurnResult::Complete => self.state = AgentState::Idle,
                TurnResult::NeedReflection => self.state = AgentState::Reflecting,
                TurnResult::Error(_) => self.state = AgentState::Idle,
            }
        }

        Ok(())
    }

    /// spawn 子 agent
    pub async fn spawn_child(&self, child_task: String) -> Result<Pid> {
        if !self.config.can_spawn {
            return Err(anyhow::anyhow!("Agent {} cannot spawn children", self.pid));
        }
        
        let children = self.children.read().await;
        if children.len() >= self.config.max_children {
            return Err(anyhow::anyhow!("Max children ({}) reached", self.config.max_children));
        }

        let child_config = AgentProcessConfig {
            max_tokens_per_pulse: self.config.max_tokens_per_pulse / 2,  // 子 agent 能量减半
            max_children: 0,  // 子 agent 不能再 spawn（防止递归）
            ..self.config.clone()
        };

        let mut child = AgentProcess::new(
            Some(self.pid),
            child_task,
            self.bus.clone(),
            child_config,
        );
        child.start().await?;
        let child_pid = child.pid;

        // 注册子 agent
        drop(children);
        self.children.write().await.push(child_pid);

        // 发布 spawn 事件
        self.bus.publish(Box::new(ConcreteEvent::new(
            EventType::AgentStarted,
            Priority::Normal,
            format!("agent:{}", self.pid),
            Box::new(AgentSpawnedPayload {
                parent: self.pid.as_u64(),
                child: child_pid.as_u64(),
            }),
        ))).await?;

        Ok(child_pid)
    }

    /// 终止 agent
    pub async fn terminate(&mut self) -> Result<()> {
        self.state = AgentState::Terminated;
        
        // 终止所有子 agent
        let children = self.children.read().await;
        for child_pid in children.iter() {
            // 通过 EventBus 通知子 agent 终止
            // ...
        }

        // 发布终止事件
        self.bus.publish(Box::new(ConcreteEvent::new(
            EventType::AgentStopped,
            Priority::Normal,
            format!("agent:{}", self.pid),
            Box::new(AgentStoppedPayload { pid: self.pid.as_u64() }),
        ))).await?;

        Ok(())
    }
}
```

---

## Part 4: Pid — 进程标识

Location: `crates/aletheon-abi/src/agent.rs` (new module)

```rust
/// Agent 进程标识符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pid(u64);

impl Pid {
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_PID: AtomicU64 = AtomicU64::new(1);
        Self(NEXT_PID.fetch_add(1, Ordering::Relaxed))
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }
}
```

---

## Part 5: AgentRegistry 升级

现有 `aletheon-runtime/src/impl/orchestration/registry.rs` 的 `AgentRegistry` 只是 HashMap。升级为管理 AgentProcess 的进程表：

```rust
pub struct AgentRegistry {
    /// 所有活跃的 agent 进程
    processes: RwLock<HashMap<Pid, Arc<Mutex<AgentProcess>>>>,
    /// PID -> task 映射
    task_index: RwLock<HashMap<String, Pid>>,
}

impl AgentRegistry {
    /// 注册并启动一个 agent 进程
    pub async fn spawn(&self, task: String, config: AgentProcessConfig, bus: Arc<dyn EventBus>) -> Result<Pid> {
        let mut process = AgentProcess::new(None, task, bus, config);
        process.start().await?;
        let pid = process.pid;
        self.processes.write().await.insert(pid, Arc::new(Mutex::new(process)));
        Ok(pid)
    }

    /// 获取 agent 进程
    pub async fn get(&self, pid: &Pid) -> Option<Arc<Mutex<AgentProcess>>> {
        self.processes.read().await.get(pid).cloned()
    }

    /// 分发脉冲到所有活跃 agent
    pub async fn dispatch_pulse(&self, pulse: &CognitivePulseEvent) {
        let processes = self.processes.read().await;
        for (_, process) in processes.iter() {
            let mut p = process.lock().await;
            if let Err(e) = p.on_pulse(pulse).await {
                tracing::warn!("Agent pulse error: {}", e);
            }
        }
    }
}
```

---

## Part 6: Integration — Engine 嵌入 AgentProcess

Engine 现在是独立的 struct。改为嵌入 AgentProcess：

```text
之前: Runtime → Engine (直接调用)
之后: Runtime → AgentRegistry → AgentProcess → Engine (内嵌)
```

Engine 新增 `run_turn_with_budget` 方法：

```rust
impl Engine {
    /// 带能量预算的 ReAct 执行
    pub async fn run_turn_with_budget(&mut self, budget: u32) -> Result<TurnResult> {
        // 和 run_turn 相同逻辑，但追踪 token 消耗
        // 超出 budget 时提前终止本轮
        // ...
    }
}
```

---

## Part 7: Daemon 模式下的生命周期

```text
aletheond 启动
    ↓
创建 EventBus + LlmPulse + AgentRegistry
    ↓
LlmPulse.run() 开始脉冲（后台 task）
    ↓
AgentRegistry.spawn("root agent", ...) 创建根 agent
    ↓
每次脉冲 → AgentRegistry.dispatch_pulse()
    ↓
根 agent 可以 spawn 子 agent
    ↓
所有 agent 通过 EventBus 通信
    ↓
aletheond 关闭 → LlmPulse 停止 → 所有 agent terminate
```

---

## Part 8: 和现有 Self-Evolution Loop 的集成

```
LlmPulse 脉冲
    ↓
AgentProcess.on_pulse()
    ↓ Engine.run_turn_with_budget()
    ↓ 执行工具
    ↓ 发出 ToolObservationEvent
    ↓
EventBus ← ToolObservationEvent
    ↓
ToolObservationHandler (BrainCore)
    ↓ 用 LlmScheduler 反思（消耗能量）
    ↓ ReflectionEvent / EvolutionTriggeredEvent
    ↓
MutationApprover (SelfField)
    ↓
MutationExecutor (MetaRuntime)
    ↓
AgentProcess 的 genome 更新
```

Self-Evolution Loop 不变，只是现在运行在 AgentProcess 内部，由 LlmPulse 驱动。

---

## Implementation Order

1. **CognitivePulseEvent** — 新事件类型（abi）
2. **LlmPulse** — 能量脉冲源（brain）
3. **Pid** — 进程标识符（abi）
4. **AgentProcess** — 进程式 agent 实体（runtime）
5. **AgentRegistry 升级** — 进程表管理（runtime）
6. **Engine.run_turn_with_budget** — 带预算的 ReAct（runtime）
7. **Daemon 集成** — aletheond 启动 LlmPulse + AgentRegistry
8. **Demo** — spawn root agent + child agent，观察脉冲驱动

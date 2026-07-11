# Phase 1–2：Agent Process、Operation 与 Chronos

## 1. 核心区分

```text
AgentId：长期身份或 Agent Profile
ProcessId：一次运行实例
OperationId：一次可取消、可计量的工作
SessionId：用户对话连续性
```

不要继续把这些 ID 混用为字符串。

## 2. 类型位置

新增：

```text
crates/fabric/src/types/process.rs
crates/fabric/src/types/operation.rs
crates/fabric/src/types/time.rs
crates/fabric/src/include/process.rs
crates/fabric/src/include/chronos.rs
```

ID 使用 `Uuid` newtype，序列化为字符串：

```rust
pub struct AgentId(pub Uuid);
pub struct ProcessId(pub Uuid);
pub struct OperationId(pub Uuid);
```

## 3. Agent Process

```rust
pub struct ProcessRecord {
    pub process_id: ProcessId,
    pub agent_id: AgentId,
    pub parent: Option<ProcessId>,
    pub profile: AgentProfileId,
    pub state: ProcessState,
    pub space: SpaceId,
    pub mailbox: MailboxId,
    pub namespace: NamespaceId,
    pub created_at: WallTime,
    pub last_heartbeat: MonoTime,
    pub exit: Option<ExitStatus>,
}
```

状态机：

```text
Created → Ready → Running ↔ Waiting → Stopping → Exited
                         └────────────→ Failed
```

状态迁移只能由 `ProcessTable` 完成。

## 4. Operation

```rust
pub struct OperationRecord {
    pub id: OperationId,
    pub owner: ProcessId,
    pub parent: Option<OperationId>,
    pub kind: OperationKind,
    pub state: OperationState,
    pub submitted_at: MonoTime,
    pub deadline: Option<MonoDeadline>,
    pub exit: Option<ExitReason>,
}
```

每个 Turn、ModelCall、CapabilityCall、MemoryConsolidation 都是 Operation。

## 5. Executive 实现

新增：

```text
crates/executive/src/kernel/process/table.rs
crates/executive/src/kernel/process/handle.rs
crates/executive/src/kernel/operation/table.rs
crates/executive/src/kernel/operation/task_group.rs
crates/executive/src/kernel/chronos/system_clock.rs
crates/executive/src/kernel/chronos/timer.rs
crates/executive/src/kernel/supervision/tree.rs
```

最小 API：

```rust
#[async_trait]
pub trait ProcessManager {
    async fn spawn(&self, spec: SpawnSpec) -> Result<ProcessHandle>;
    async fn signal(&self, id: ProcessId, signal: ProcessSignal) -> Result<()>;
    async fn wait(&self, id: ProcessId) -> Result<ExitStatus>;
    async fn inspect(&self, id: ProcessId) -> Result<ProcessSnapshot>;
}

#[async_trait]
pub trait OperationManager {
    async fn submit(&self, req: OperationRequest) -> Result<OperationHandle>;
    async fn cancel(&self, id: OperationId, reason: CancelReason) -> Result<()>;
    async fn wait(&self, id: OperationId) -> Result<OperationResult>;
}
```

## 6. 改造 SubAgentSpawner

当前 `SubAgentSpawner` 只登记 UI handle。迁移方式：

1. `spawn()` 调用 `ProcessManager::spawn()`；
2. UI `SubAgentHandle` 变成 `ProcessSnapshot` 的 View；
3. CancellationToken 来自 Operation task group；
4. `destroy/remove` 改为 `signal(Terminate)` + `wait/reap`；
5. Agent tool 返回 `ProcessId`，而不是自增 `agent-N`。

## 7. Chronos

```rust
pub trait Clock: Send + Sync {
    fn wall_now(&self) -> WallTime;
    fn mono_now(&self) -> MonoTime;
}
```

实现：

```text
SystemClock：std::time::SystemTime + tokio::time::Instant
TestClock：tokio paused time 或显式 VirtualClock
DomainClockAdapter：后续用于 Robot/Simulation
```

规则：

- deadline/timeout/lease 使用 MonoTime；
- 用户时间和审计展示使用 WallTime；
- Dasein TemporalStream 不实现 kernel timer；
- 禁止业务代码直接散落调用 `SystemTime::now()`。

## 8. 结构化并发

任何 `tokio::spawn` 必须登记到 Operation task group：

```rust
pub struct OperationScope {
    pub id: OperationId,
    pub cancel: CancellationToken,
    pub tasks: JoinSet<TaskExit>,
}
```

父 Operation 退出：

```text
cancel children
wait grace period
abort remaining
record structured exits
```

## 9. 测试

```bash
cargo test -p executive process_table
cargo test -p executive operation_tree
cargo test -p executive chronos
cargo test -p executive supervision
```

必须覆盖：

- 非法状态迁移；
- spawn → wait → reap；
- 父取消传播；
- deadline 使用 VirtualClock 可确定测试；
- RestartOnFailure 达到重试上限；
- Process panic 转换为 `ExitReason::Panic`。

## 10. 完成标准

- SubAgent 具有真实执行任务；
- 所有 Turn 都有 OperationId；
- TUI 可读取 Process/Operation snapshot；
- daemon shutdown 后没有孤儿任务；
- timeout 测试不依赖真实 sleep。


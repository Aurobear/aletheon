> Merged from docs/design/resilience/ — code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

# Resilience

> Error handling, panic recovery, rate limiting, and backpressure — the agent's fault tolerance layer.

**Crate:** `dasein`
**Module:** `crates/dasein/src/impl/resilience/`

---

## Part 1: Error Handling


> 统一的错误分类、降级策略和恢复机制。`AgentError` 枚举、错误严重级别、降级链和重试退避已实现。

**关联模块:** [panic 恢复](resilience.md), [限流](resilience.md), [安全模型](../corpus/security.md)
**最后更新:** 2026-06-07

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| AgentError enum | ✅ Implemented | `crates/runtime/src/impl/error.rs` | Error severity, categories, degradation chain |
| ErrorSeverity | ✅ Implemented | `crates/runtime/src/impl/error.rs` | Recoverable / Degraded / Unrecoverable / SecurityViolation |
| DegradationChain | ✅ Implemented | `crates/runtime/src/impl/error.rs` | Retry with backoff, fallback strategies |
| RecoveryEngine | ⬜ Planned | — | Session restore from checkpoint not yet built |

---

## 1. 错误分类

### 1.1 严重级别

```rust
enum ErrorSeverity {
    Recoverable,       // 可自动恢复（重试/降级）
    Degraded,          // 功能降级（本地推理失败→云端）
    Unrecoverable,     // 不可恢复（需用户介入）
    SecurityViolation, // 安全违规（立即停止，写入安全日志）
}
```

### 1.2 错误范畴

```rust
enum ErrorCategory {
    LlmError { provider: String, kind: LlmErrorKind },
    ToolError { tool: String, kind: ToolErrorKind },
    SandboxError { kind: SandboxErrorKind },
    MemoryError { kind: MemoryErrorKind },
    PerceptionError { source: String, kind: PerceptionErrorKind },
    IpcError { kind: IpcErrorKind },
    ConfigError { kind: ConfigErrorKind },
}
```

**每类错误的子类型（示例）：**

| 类别 | 子类型 | 严重级别 | 默认行为 |
|------|--------|----------|----------|
| LlmError | Timeout / RateLimited / InvalidResponse / AuthFailure | Recoverable~Unrecoverable | 重试→降级→AskUser |
| ToolError | Timeout / PermissionDenied / ResourceExhausted / SecurityViolation | Recoverable~SecurityViolation | 重试→Skip→Abort |
| SandboxError | BubblewrapMissing / NamespaceUnavailable / OomKilled | Degraded~Unrecoverable | 降级后端→AskUser |
| MemoryError | StoreFull / QueryFailed / CorruptionDetected | Recoverable~Unrecoverable | 压缩→重建→AskUser |

---

## 2. 降级链 (DegradationChain)

借鉴 Codex 的降级链模式：每个操作可以配置一序列备选策略，按优先级逐个尝试。

### 2.1 降级策略枚举

```rust
enum DegradationStrategy {
    /// 重试 + 指数退避
    Retry { max_attempts: u32, backoff: BackoffStrategy },
    /// 切换到本地模型推理
    FallbackToLocal,
    /// 切换到云端模型推理
    FallbackToCloud,
    /// 减少上下文窗口重试
    ReduceContext,
    /// 跳过当前工具，继续执行
    SkipTool,
    /// 暂停整个推理轮次，询问用户
    AskUser,
}
```

### 2.2 退避策略

```rust
enum BackoffStrategy {
    /// 固定间隔
    Fixed { delay: Duration },
    /// 指数退避：base * 2^n，上限 cap
    Exponential { base: Duration, max: Duration },
    /// 带抖动的指数退避（推荐）：base * 2^n + random(0, jitter)
    ExponentialWithJitter { base: Duration, max: Duration, jitter: Duration },
    /// 线性：base * n
    Linear { base: Duration },
}
```

**推荐值：** 工具调用退避默认 `ExponentialWithJitter { base: 500ms, max: 30s, jitter: 100ms }`。LLM 调用退避默认 `ExponentialWithJitter { base: 1s, max: 60s, jitter: 200ms }`。

### 2.3 降级链执行器

```rust
struct DegradationChain {
    strategies: Vec<DegradationStrategy>,
}

impl DegradationChain {
    async fn execute<F, T>(&self, operation: F) -> Result<T, AgentError>
    where F: Fn() -> Future<Result<T>>
    {
        for strategy in &self.strategies {
            match strategy.try_execute(&operation).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    tracing::warn!(error = %e, strategy = %strategy, "strategy failed, trying next");
                    continue;
                }
            }
        }
        Err(AgentError::AllStrategiesExhausted)
    }
}
```

### 2.4 典型降级链配置

| 操作 | 降级链 | 说明 |
|------|--------|------|
| LLM 推理 | Retry(3) → FallbackToLocal → ReduceContext → AskUser | 先重试，不行切本地，再不行减上下文 |
| 工具执行 | Retry(2) → SkipTool → AskUser | 重试两次，跳过，不行问用户 |
| 沙箱创建 | Retry(1) → 降级后端 → AskUser | 失败后尝试其他沙箱后端 |
| 感知事件 | Retry(0) → DropEvent | 感知事件不重试，直接丢弃 |

---

## 3. 工具错误处理

```rust
async fn handle_tool_error(&self, error: ToolError, context: &ToolContext) -> ToolErrorAction {
    match error.kind {
        ToolErrorKind::Timeout => {
            if context.retry_count < 2 {
                ToolErrorAction::Retry { delay: Duration::from_secs(1) }
            } else {
                ToolErrorAction::Skip { reason: "timeout after retries" }
            }
        }
        ToolErrorKind::PermissionDenied => {
            ToolErrorAction::RequestPermission { required_level: error.required_permission }
        }
        ToolErrorKind::ResourceExhausted => {
            ToolErrorAction::Degrade { alternative: "reduce scope" }
        }
        ToolErrorKind::SecurityViolation => {
            ToolErrorAction::Abort { reason: error.message }
        }
        _ => ToolErrorAction::ReportToUser { error },
    }
}
```

**异常场景处理：**

| 场景 | 处理方式 |
|------|----------|
| 降级链所有策略耗尽 | 返回 `AgentError::AllStrategiesExhausted`，当前推理轮次终止 |
| 安全违规 | 不降级不重试，立即终止并写入安全审计日志 |
| 嵌套错误（降级链本身出错） | 捕获 panic，回退到 `AskUser` |
| LLM RateLimited (429) | 读取 `Retry-After` header 作为退避时间 |
| 静默失败（无返回无错误） | 根据 `CancellationToken` 状态判断是取消还是挂起 |

---

## 4. 错误恢复

```rust
struct RecoveryEngine {
    async fn recover_from_crash(&self, snapshot: &StateSnapshot) -> Result<()> {
        // 1. 恢复会话状态
        self.restore_session(&snapshot.session).await?;
        // 2. 恢复记忆
        self.restore_memory(&snapshot.memory).await?;
        // 3. 重新加载 eBPF 程序（如果之前启用）
        self.reload_ebpf().await?;
        // 4. 重新注册感知源
        self.reregister_perception().await?;
        Ok(())
    }
}
```

**恢复优先级：** 会话状态 > Core Memory > 工具注册 > 感知源 > eBPF 程序。前两项决定能否恢复对话连续性，后三项决定功能完整性。

---

## 5. 与安全模型的集成

| 集成点 | 说明 |
|--------|------|
| LoopDetector | 连续失败超过阈值 → 触发 `Escalate` 而非自动重试 |
| CircuitBreaker | 熔断器打开时所有工具调用直接返回 `ToolError::CircuitBreakerOpen` |
| AuditLogger | 降级链的每一步都写入审计日志，包括 `strategy` 和 `decision` |
| PolicyEngine | 降级策略不得绕过安全策略（如重试时不能跳过权限检查） |

---

## 6. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| Codex | 降级链模式 (DegradationChain)，退避策略枚举 |
| Codex | `AllStrategiesExhausted` 错误类型 |
| Hermes Agent | 工具错误分类 + 上下文注入重试 |
| Hermes Agent | 嵌套错误处理（降级链 panic 捕获） |
| OpenHands | RateLimit 中间件，Retry-After header 处理 |
| OpenCode | LLM provider 错误归一化（横跨多 provider 的错误转换） |
| Claude Code | 工具调用 `CancellationToken` 取消传播 |
| LangGraph | `apply_writes` 原子写入失败 → 回滚到上个 checkpoint |

---

## Implementation Summary

> `AgentError` 枚举已实现，包含错误严重级别、错误范畴、降级链和重试退避策略。`RecoveryEngine`（崩溃后状态恢复）尚未实现。

| Component | Status | Notes |
|-----------|--------|-------|
| AgentError enum | ✅ Implemented | `crates/runtime/src/impl/error.rs` |
| ErrorSeverity | ✅ Implemented | Recoverable / Degraded / Unrecoverable / SecurityViolation |
| DegradationChain | ✅ Implemented | Retry with exponential backoff + jitter |
| ToolErrorAction | ✅ Implemented | Error-driven action selection |
| RecoveryEngine | ⬜ Planned | Session restore from checkpoint |


---

## Part 2: Panic Recovery


> Daemon 容错、自愈和状态恢复。`DaemonGuardian`、三层看门狗、`SafeMode` 和崩溃现场保存均已实现。

**关联模块:** [错误处理](resilience.md), [会话管理](../runtime/session.md)
**最后更新:** 2026-06-07

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| DaemonGuardian + PanicPolicy | ✅ Implemented | `crates/dasein/src/impl/resilience/guardian.rs` | Crash dump, policy dispatch |
| WatchdogTimer (3-layer) | ✅ Implemented | `crates/dasein/src/impl/resilience/watchdog.rs` | L1 30s, L2 10s, L3 5min |
| SafeMode | ✅ Implemented | `crates/dasein/src/impl/resilience/safe_mode.rs` | Auto-exit with cooldown |
| Crash dump | ✅ Implemented | `guardian.rs` | `{crash_dir}/{timestamp}/` with panic_info.json, state_snapshot.json, version.txt |
| RecoveryEngine | ⬜ Planned | — | Full state restore from snapshot |

---

## 1. Daemon 容错策略

借鉴 Erlang/OTP 的 Supervisor Tree 模式：每个子系统是一个"子进程"，有独立的容错策略，失败时由上级监督者决定如何恢复。

### 1.1 策略枚举

```rust
enum PanicPolicy {
    /// 自杀重启，保留状态到磁盘
    RestartWithState,
    /// 自杀重启，从上次 checkpoint 恢复
    RestartFromCheckpoint,
    /// 进入安全模式（只接受诊断查询）
    EnterSafeMode,
    /// 通知用户后退出
    NotifyAndExit,
}
```

### 1.2 DaemonGuardian

```rust
struct DaemonGuardian {
    policy: PanicPolicy,
    watchdog: WatchdogTimer,

    async fn on_panic(&self, panic_info: &PanicInfo) {
        // 1. 保存崩溃现场
        self.save_crash_dump(panic_info).await;

        // 2. 根据策略处理
        match self.policy {
            PanicPolicy::RestartWithState => {
                self.save_state_snapshot().await;
                self.systemd_restart().await;
            }
            PanicPolicy::RestartFromCheckpoint => {
                self.systemd_restart().await;  // 启动时自动从 checkpoint 恢复
            }
            PanicPolicy::EnterSafeMode => {
                self.enter_safe_mode().await;
            }
            PanicPolicy::NotifyAndExit => {
                self.notify_user("Agent 崩溃，请检查日志").await;
                std::process::exit(1);
            }
        }
    }
}
```

### 1.3 子系统监督树

| 子系统 | 监督策略 | 失败影响范围 | 恢复时间目标 |
|--------|----------|-------------|-------------|
| 认知引擎 (ReAct Loop) | RestartFromCheckpoint | 当前推理轮次丢失 | <5s |
| LLM Provider | RestartWithState | 当前请求失败 | <2s |
| 工具系统 | RestartWithState | 正在执行的工具中断 | <3s |
| 感知引擎 | RestartWithState | 感知事件短暂丢失(5s窗口) | <5s |
| 记忆系统 | EnterSafeMode | 记忆不可写，可读 | 需人工 |
| IPC Server | RestartWithState | 所有连接断开 | <3s |

---

## 2. Watchdog 机制

### 2.1 核心实现

systemd 的 `WatchdogSec` 提供内核级看门狗，配合 sd_notify 心跳实现双保险：

```rust
struct WatchdogTimer {
    timeout: Duration,         // systemd WatchdogSec
    last_heartbeat: Arc<Mutex<Instant>>,
}

impl WatchdogTimer {
    async fn heartbeat_loop(&self) {
        loop {
            tokio::time::sleep(self.timeout / 2).await;
            *self.last_heartbeat.lock() = Instant::now();
            // sd_notify("WATCHDOG=1") 通知 systemd
        }
    }

    fn check_alive(&self) -> bool {
        self.last_heartbeat.lock().elapsed() < self.timeout
    }
}
```

### 2.2 心跳健康信号

| 信号 | 含义 | 响应 |
|------|------|------|
| `WATCHDOG=1` | 正常运行 | systemd 重置看门狗计时器 |
| `WATCHDOG=trigger` | 主动触发重启 | systemd 立即执行 `Restart=` 策略 |
| 心跳缺失超过 WatchdogSec | 进程挂起 | systemd 发送 SIGABRT + 重启 |

### 2.3 层级看门狗

借鉴 Codex 的层级 watch 模式：

| 层级 | 机制 | 超时 | 探测目标 |
|------|------|------|----------|
| L1: 进程级 | systemd WatchdogSec | 30s | 整个 aletheon daemon 进程 |
| L2: 事件循环级 | Tokio runtime 检测 | 10s | 事件循环是否阻塞 |
| L3: 推理循环级 | 自定义 deadline | 每轮 5min | 单次推理是否卡死 |

---

## 3. 崩溃恢复协议

### 3.1 恢复流程

```rust
async fn crash_recovery(&self) -> Result<()> {
    // 检查是否有未完成的状态
    if let Some(snapshot) = self.load_latest_snapshot().await? {
        tracing::info!("found crash snapshot, recovering...");

        // 验证快照完整性
        if snapshot.verify_integrity() {
            self.restore_from_snapshot(&snapshot).await?;
            tracing::info!("recovery successful");
        } else {
            tracing::warn!("snapshot corrupted, starting fresh");
            self.start_clean().await?;
        }
    } else {
        self.start_clean().await?;
    }
    Ok(())
}
```

### 3.2 快照完整性验证

| 检查项 | 方法 | 失败处理 |
|--------|------|----------|
| SHA256 哈希 | `snapshot.checksum == computed` | 快照损坏，重新开始 |
| 版本兼容性 | `snapshot.version == current_version` | 版本不匹配，尝试迁移或重新开始 |
| 字段非空 | 关键字段不为 None | 字段缺失，重新开始 |
| 边界时间戳 | `created_at < now() + 1min` | 时间戳异常，忽略 |

### 3.3 崩溃现场保存

panic 发生时，自动保存以下信息到 `{data_dir}/crash/`：

```
crash/{timestamp}/
├── panic_info.json       # panic 消息、位置、回溯
├── state_snapshot.json   # 最后一次安全的运行时状态
├── session_snapshot.json # 活跃会话快照
├── journal_{seq}.jsonl   # 最近 N 条事件日志
└── version.txt           # aletheon daemon 版本 + commit
```

---

## 4. 安全模式 (Safe Mode)

当关键子系统（记忆系统、LLM Provider 等）不可恢复时，aletheon daemon 自动进入安全模式：

| 能力 | 安全模式下 | 说明 |
|------|-----------|------|
| 对话 | ❌ | 新对话不可用 |
| 诊断查询 | ✅ | `interact debug status` 等诊断命令 |
| 工具执行 | ❌ | 不执行任何工具 |
| 感知事件 | ❌ | 不采集/不处理 |
| 崩溃恢复 | ✅ | 持续尝试恢复 |
| 日志查看 | ✅ | 用户可查看完整日志 |

---

## 5. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| Codex | 层级看门狗 (L1进程/L2事件循环/L3推理) |
| Codex | 崩溃现场保存格式 (crash/{timestamp}/ 目录) |
| Hermes Agent | 监督树模式 + PanicPolicy 枚举 |
| systemd | `WatchdogSec` + `sd_notify()` 协议 |
| Erlang/OTP | Supervisor Tree — `RestartWithState` / `EnterSafeMode` 策略 |
| Codex | 快照完整性验证（checksum + version + non-null + timestamp） |
| OpenCode | 安全模式下诊断接口仍可用 |

---

## Implementation Summary

> 结构化 Panic 恢复已实现。`DaemonGuardian` 管理崩溃策略和现场保存，三层 `WatchdogTimer` 提供进程/事件循环/推理循环级监控，`SafeMode` 提供带冷却的自动退出。

| Component | Status | Notes |
|-----------|--------|-------|
| DaemonGuardian + PanicPolicy | ✅ Implemented | `crates/dasein/src/impl/resilience/guardian.rs` |
| WatchdogTimer (3-layer) | ✅ Implemented | `crates/dasein/src/impl/resilience/watchdog.rs` — L1 30s, L2 10s, L3 5min |
| SafeMode | ✅ Implemented | `crates/dasein/src/impl/resilience/safe_mode.rs` — auto-exit with cooldown |
| Crash dump | ✅ Implemented | `{crash_dir}/{timestamp}/` — panic_info.json, state_snapshot.json, version.txt |
| RecoveryEngine | ⬜ Planned | Full state restore from snapshot |


---

## Part 3: Rate Limiting & Backpressure


> Token 速率限制、工具调用频率限制、感知事件洪水防护和背压传播。`TokenRateLimiter`、`ToolCallLimiter`、`FloodProtector` 和 `BackpressureController` 均已实现。

**关联模块:** [错误处理](resilience.md), [自我保护](self-protection.md), [感知层](../corpus/perception.md)
**最后更新:** 2026-06-07

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| TokenRateLimiter | ✅ Implemented | `crates/corpus/src/impl/security/rate_limiting/token_limiter.rs` | Multi-tier token quota |
| ToolCallLimiter | ✅ Implemented | `crates/corpus/src/impl/security/rate_limiting/tool_limiter.rs` | Per-tool and concurrency limits |
| FloodProtector | ✅ Implemented | `crates/corpus/src/impl/security/rate_limiting/flood_protector.rs` | Per-source sliding window |
| BackpressureController | ✅ Implemented | `crates/corpus/src/impl/security/rate_limiting/backpressure.rs` | Signal propagation |

---

## 1. Token 限流

多层 Token 配额管理，覆盖单次推理、小时级和日级限制。

### 1.1 TokenRateLimiter

```rust
struct TokenRateLimiter {
    max_per_turn: u32,   // 单次推理
    max_per_hour: u32,   // 小时级硬限制
    max_per_day: u32,    // 日级硬限制
    usage: Arc<Mutex<TokenUsage>>,
}

impl TokenRateLimiter {
    fn check(&self, requested: u32) -> Result<(), RateLimitError> {
        let usage = self.usage.lock();
        if usage.this_hour + requested > self.max_per_hour {
            return Err(RateLimitError::HourlyExceeded {
                current: usage.this_hour,
                limit: self.max_per_hour,
            });
        }
        Ok(())
    }

    fn throttle_action(&self) -> ThrottleAction {
        let usage = self.usage.lock();
        let ratio = usage.this_hour as f32 / self.max_per_hour as f32;
        match ratio {
            r if r > 0.95 => ThrottleAction::Reject,
            r if r > 0.80 => ThrottleAction::ForceLocalOnly,
            r if r > 0.60 => ThrottleAction::ReduceContext,
            _ => ThrottleAction::None,
        }
    }
}
```

### 1.2 配额分层

| 层级 | 粒度 | 默认值 | 突破方式 |
|------|------|--------|----------|
| Per turn | 单次推理循环 | 100K tokens | 用户显式 `--unlimited` |
| Per hour | 滑动窗口 1h | 500K tokens | 配置文件调整 |
| Per day | 固定 UTC 日 | 5M tokens | 配置文件调整 |
| Provider side | API 自带限制 | 取决于 provider | 读取 `Retry-After` header |

### 1.3 渐进式降级 (ThrottleAction)

| 阈值 | 动作 | 效果 |
|------|------|------|
| < 60% | None | 无限制 |
| 60%~80% | ReduceContext | 减少上下文压缩阈值，缩短响应 |
| 80%~95% | ForceLocalOnly | 强制使用本地模型(llama.cpp)，禁用云端 |
| > 95% | Reject | 拒绝新推理请求，返回 RateLimitExceeded |

---

## 2. 工具调用限流

结合 LoopDetector 的 per-turn 限制，增加 per-tool 和 concurrency 维度。

### 2.1 ToolRateLimiter

```rust
struct ToolRateLimiter {
    max_per_turn: u32,              // 每轮最大工具调用数
    max_concurrent: u32,            // 最大并发数
    per_tool_limits: HashMap<String, u32>, // 每工具专用限制
}

impl ToolRateLimiter {
    fn check_tool_call(&self, tool_name: &str, current_turn_calls: u32, active_calls: u32) -> Result<()> {
        if current_turn_calls >= self.max_per_turn {
            return Err(RateLimitError::TurnLimitExceeded);
        }
        if active_calls >= self.max_concurrent {
            return Err(RateLimitError::ConcurrencyLimitExceeded);
        }
        if let Some(limit) = self.per_tool_limits.get(tool_name) {
            if current_turn_calls >= *limit {
                return Err(RateLimitError::ToolLimitExceeded { tool: tool_name.to_string() });
            }
        }
        Ok(())
    }
}
```

### 2.2 默认 per-tool 限制

| 工具 | 每轮上限 | 理由 |
|------|---------|------|
| `file_read` | 20 | 防止大量小文件读取引发高延迟 |
| `bash_exec` | 5 | shell 命令开销大，副作用不确定 |
| `file_write` | 10 | 写操作需谨慎 |
| `file_search` | 10 | 搜索是读操作但可能消耗大量 CPU |
| `network_info` | 5 | 网络查询可能触发外部请求 |
| `system_status` | 3 | 状态查询快，不需要频繁调用 |

---

## 3. 事件洪水防护

感知层可能产生大量事件（journald 在启动时可每秒产生数千条），需要洪水防护。

### 3.1 EventFloodProtector

```rust
struct EventFloodProtector {
    max_events_per_second: u32,
    window_size: Duration,
    event_counts: Arc<Mutex<SlidingWindow>>,
}

impl EventFloodProtector {
    fn should_process(&self, source: &str) -> bool {
        let mut counts = self.event_counts.lock();
        let rate = counts.rate_for(source);
        if rate > self.max_events_per_second {
            tracing::warn!(source = source, rate = rate, "event flood detected, dropping");
            return false;
        }
        counts.record(source);
        true
    }
}
```

### 3.2 层级事件过滤

| 层级 | 机制 | 说明 |
|------|------|------|
| L1: 源级 | EventFloodProtector | 每个感知源独立限流，超限丢弃 |
| L2: 聚合 | EventAggregator 去重 | 同内容事件在时间窗口内去重(默认 500ms) |
| L3: 批量 | 批量折叠 | 同源同类型事件合并为 Batch 事件 |
| L4: 优先级 | 优先级队列 | 低优先级事件在高负载时延迟处理 |

### 3.3 Per-Source 限流阈值

| 感知源 | 默认阈值 | 突发容差 |
|--------|----------|----------|
| /proc 轮询 | 10 events/s | 20 events/s burst |
| inotify | 50 events/s | 200 events/s burst |
| journald | 100 events/s | 500 events/s burst |
| eBPF (sched) | 500 events/s | 2000 events/s burst |
| eBPF (net) | 200 events/s | 1000 events/s burst |

---

## 4. 背压传播

当下游处理能力不足时，向上游发信号减速。

### 4.1 背压信号

```rust
enum BackpressureSignal {
    /// 上游处理不过来，要求下游减速
    SlowDown { queue_depth: u32 },
    /// 队列满了，丢弃低优先级事件
    DropLowPriority { dropped_count: u32 },
    /// 暂停事件源
    PauseSource { source: String },
}
```

### 4.2 传播路径

```
感知源 → EventAggregator → PerceptionBridge → Engine
  ↑           ↑                  ↑
  │           │                  │
  └───────────┴──────────────────┘ ← 背压信号反向传播
```

| 消费者状态 | 信号 | 源端响应 |
|-----------|------|----------|
| 聚合队列 > 1000 | SlowDown | 轮询间隔加倍 |
| 聚合队列 > 5000 | DropLowPriority | 丢弃 medium/low 事件 |
| Engine 推理中 | PauseSource | 暂停所有事件采集 |
| Engine 空闲 | Resume | 恢复事件采集 |

### 4.3 队列优先级

| 优先级 | 示例 | 丢弃策略 |
|--------|------|----------|
| Critical | 内存 OOM、磁盘故障 | 永不丢弃 |
| High | 安全违规、服务崩溃 | 最后丢弃 |
| Medium | CPU 超限、网络延迟 | 正常丢弃 |
| Low | 周期性状态更新 | 最先丢弃 |

---

## 5. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| OpenHands | 按 user/org 分级的速率限制，Retry-After 处理 |
| Codex | ToolRateLimiter per-tool 限制 + per-turn max |
| Hermes Agent | EventFloodProtector 滑动窗口 + per-source 限流 |
| Hermes Agent | BackpressureSignal 反向传播模式 |
| OpenCode | Token 配额的渐进式降级 (ThrottleAction) |
| Claude Code | Tokens() helper 安全归一化（防 NaN/负值） |
| perception-layer | EventAggregator 去重 + 批量折叠（已实现部分） |

---

## Implementation Summary

> 限流与背压已全面实现。`TokenRateLimiter` 提供多层 Token 配额，`ToolCallLimiter` 提供 per-tool 和并发限制，`FloodProtector` 提供滑动窗口洪水防护，`BackpressureController` 提供背压信号传播。

| Component | Status | Notes |
|-----------|--------|-------|
| TokenRateLimiter | ✅ Implemented | `crates/corpus/src/impl/security/rate_limiting/token_limiter.rs` |
| ToolCallLimiter | ✅ Implemented | `crates/corpus/src/impl/security/rate_limiting/tool_limiter.rs` |
| FloodProtector | ✅ Implemented | `crates/corpus/src/impl/security/rate_limiting/flood_protector.rs` |
| BackpressureController | ✅ Implemented | `crates/corpus/src/impl/security/rate_limiting/backpressure.rs` |


---


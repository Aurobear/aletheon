# 错误处理 (Error Handling)

> 统一的错误分类、降级策略和恢复机制。`AgentError` 枚举、错误严重级别、降级链和重试退避已实现。

**关联模块:** [panic 恢复](panic-recovery.md), [限流](rate-limiting.md), [安全模型](../security/security-model.md)
**最后更新:** 2026-06-07

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| AgentError enum | ✅ Implemented | `crates/agent-core/src/error.rs` | Error severity, categories, degradation chain |
| ErrorSeverity | ✅ Implemented | `crates/agent-core/src/error.rs` | Recoverable / Degraded / Unrecoverable / SecurityViolation |
| DegradationChain | ✅ Implemented | `crates/agent-core/src/error.rs` | Retry with backoff, fallback strategies |
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
| AgentError enum | ✅ Implemented | `crates/agent-core/src/error.rs` |
| ErrorSeverity | ✅ Implemented | Recoverable / Degraded / Unrecoverable / SecurityViolation |
| DegradationChain | ✅ Implemented | Retry with exponential backoff + jitter |
| ToolErrorAction | ✅ Implemented | Error-driven action selection |
| RecoveryEngine | ⬜ Planned | Session restore from checkpoint |

# 限流与背压 (Rate Limiting & Backpressure)

> Token 速率限制、工具调用频率限制、感知事件洪水防护和背压传播。`TokenRateLimiter`、`ToolCallLimiter`、`FloodProtector` 和 `BackpressureController` 均已实现。

**关联模块:** [错误处理](error-handling.md), [自我保护](../security/self-protection.md), [感知层](../perception/perception-layer.md)
**最后更新:** 2026-06-07

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| TokenRateLimiter | ✅ Implemented | `crates/agent-core/src/security/rate_limiting/token_limiter.rs` | Multi-tier token quota |
| ToolCallLimiter | ✅ Implemented | `crates/agent-core/src/security/rate_limiting/tool_limiter.rs` | Per-tool and concurrency limits |
| FloodProtector | ✅ Implemented | `crates/agent-core/src/security/rate_limiting/flood_protector.rs` | Per-source sliding window |
| BackpressureController | ✅ Implemented | `crates/agent-core/src/security/rate_limiting/backpressure.rs` | Signal propagation |

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
| TokenRateLimiter | ✅ Implemented | `crates/agent-core/src/security/rate_limiting/token_limiter.rs` |
| ToolCallLimiter | ✅ Implemented | `crates/agent-core/src/security/rate_limiting/tool_limiter.rs` |
| FloodProtector | ✅ Implemented | `crates/agent-core/src/security/rate_limiting/flood_protector.rs` |
| BackpressureController | ✅ Implemented | `crates/agent-core/src/security/rate_limiting/backpressure.rs` |

# Panic 恢复 (Panic Recovery)

> Daemon 容错、自愈和状态恢复。`DaemonGuardian`、三层看门狗、`SafeMode` 和崩溃现场保存均已实现。

**关联模块:** [错误处理](error-handling.md), [会话管理](../core/session-lifecycle.md)
**最后更新:** 2026-06-07

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| DaemonGuardian + PanicPolicy | ✅ Implemented | `crates/agent-core/src/resilience/guardian.rs` | Crash dump, policy dispatch |
| WatchdogTimer (3-layer) | ✅ Implemented | `crates/agent-core/src/resilience/watchdog.rs` | L1 30s, L2 10s, L3 5min |
| SafeMode | ✅ Implemented | `crates/agent-core/src/resilience/safe_mode.rs` | Auto-exit with cooldown |
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
| L1: 进程级 | systemd WatchdogSec | 30s | 整个 agentd 进程 |
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
└── version.txt           # agentd 版本 + commit
```

---

## 4. 安全模式 (Safe Mode)

当关键子系统（记忆系统、LLM Provider 等）不可恢复时，agentd 自动进入安全模式：

| 能力 | 安全模式下 | 说明 |
|------|-----------|------|
| 对话 | ❌ | 新对话不可用 |
| 诊断查询 | ✅ | `agent-cli debug status` 等诊断命令 |
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
| DaemonGuardian + PanicPolicy | ✅ Implemented | `crates/agent-core/src/resilience/guardian.rs` |
| WatchdogTimer (3-layer) | ✅ Implemented | `crates/agent-core/src/resilience/watchdog.rs` — L1 30s, L2 10s, L3 5min |
| SafeMode | ✅ Implemented | `crates/agent-core/src/resilience/safe_mode.rs` — auto-exit with cooldown |
| Crash dump | ✅ Implemented | `{crash_dir}/{timestamp}/` — panic_info.json, state_snapshot.json, version.txt |
| RecoveryEngine | ⬜ Planned | Full state restore from snapshot |

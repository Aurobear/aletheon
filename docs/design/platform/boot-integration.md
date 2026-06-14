# 开机自举 (Boot Integration)

> Agent 参与系统启动过程，提供启动监控和故障诊断。BootMonitor、ServiceDependencyGraph（含拓扑排序和环检测）、5 阶段延迟加载、启动故障诊断均已实现。

**关联模块:** [系统管理](../perception/system-management.md), [可观测性栈](../observability/observability-stack.md)
**最后更新:** 2026-06-07

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| BootMonitor | ✅ Implemented | `platform/boot.rs` | Boot phase FSM + dependency tracking |
| BootPhase state machine | ✅ Implemented | `platform/boot.rs` | Initializing/Monitoring/Ready/Degraded |
| ServiceDependencyGraph | ✅ Implemented | `platform/boot.rs` | Topological sort + cycle detection (`would_create_cycle`) |
| Lazy loading (5-stage) | ✅ Implemented | `platform/boot.rs` | `LazyLoadStage`: immediate → 500ms → 2s → 5s → on-demand |
| Boot diagnosis | ✅ Implemented | `platform/boot.rs` | `BootDiagnosis`: resource/service/historical checks |
| systemd service | ✅ Exists | `systemd/agentd.service` | Service file |

---

## 1. 启动阶段

```
GRUB/UEFI → initramfs → systemd init → services → user session → agentd
              [Phase 6]    [Phase 1-3]    [...services]     [agent-cli]
```

| 阶段 | Agent 参与方式 | 功能 |
|------|---------------|------|
| initramfs | ❌ 未参与 | 挂载根文件系统前 Agent 不可用 |
| systemd early | ❌ 未参与 | basic.target 前依赖缺失 |
| systemd services | ✅ systemd service | agentd 在 network.target 后启动 |
| user session | ✅ systemd --user / 桌面启动 | agent-cli 提供用户交互 |

---

## 2. systemd 集成

```ini
[Unit]
Description=OS-Agent Daemon
After=network.target dbus.service sysinit.target

[Service]
Type=notify
ExecStart=/usr/bin/agentd --config /etc/agent/agent.toml
Restart=on-failure
RestartSec=5
WatchdogSec=30
ProtectSystem=strict
ReadWritePaths=/var/lib/agent /var/log/agent /run/agent
MemoryMax=2G
CPUQuota=50%

[Install]
WantedBy=multi-user.target
```

### systemd 防护参数说明

| 参数 | 当前值 | 目的 |
|------|--------|------|
| `ProtectSystem=strict` | strict | 只读 /usr /etc，防写系统关键路径 |
| `MemoryMax` | 2G | 防内存泄漏导致 OOM |
| `CPUQuota` | 50% | 防 CPU 耗尽 |
| `WatchdogSec` | 30s | 30s 无心跳自动重启 |
| `Restart=on-failure` | on-failure | 非正常退出自动拉起 |
| `ReadWritePaths` | 白名单 | 仅允许 `data/log/run` 写入 |

---

## 3. 启动阶段管理

```rust
enum BootPhase {
    Initializing,
    Monitoring {
        watched_services: Vec<String>,
        startup_failures: Vec<ServiceFailure>,
    },
    Ready,
    Degraded { issues: Vec<StartupIssue> },
}

struct BootMonitor {
    dependency_graph: ServiceDependencyGraph,
    /// 跟踪目标服务的启动状态
    async fn track_service_startup(&mut self, event: PerceptionEvent) { ... }
    /// 检查 boot 是否完成（所有依赖服务 Ready）
    fn check_boot_complete(&self) -> bool { ... }
    /// 获取启动时间线
    fn boot_timeline(&self) -> Vec<BootEvent> { ... }
}
```

### 监测的服务列表

| 服务 | 依赖 Agent？ | Agent 依赖它？ | 监控原因 |
|------|------------|---------------|----------|
| dbus.service | 否 | 是 | Agent 通信基础 |
| network.target | 否 | 是 | LLM 云端推理需网络 |
| systemd-journald | 否 | 是 | 感知层日志源 |
| NetworkManager | 否 | 否 | 网络状态诊断 |
| docker.service | 否 | 否 | MCP 服务器/沙箱执行 |
| sshd.service | 否 | 否 | 远程访问诊断 |

---

## 4. 启动故障诊断

```rust
async fn diagnose_boot_failure(&self, failures: &[ServiceFailure]) -> BootDiagnosis {
    // 1. 检查 journal 日志（journald）
    // 2. 检查依赖服务状态（systemctl list-dependencies）
    // 3. 检查系统资源状态（disk/cpu/mem）
    // 4. 关联历史类似问题
}
```

### 自动化诊断流程

```
Agent 检测到服务启动失败
    │
    ▼
1. 查询 journalctl -u <service> --since "5 min ago"
    ├── 成功 → 分析日志中的错误模式 (OOM / timeout / config error)
    └── 失败 → journald 可能未运行 → 检查 /var/log/messages
    │
    ▼
2. 检查依赖链
    ├── 依赖服务未启动 → 递归诊断依赖服务
    └── 依赖全部正常 → 服务自身问题
    │
    ▼
3. 检查系统资源
    ├── 磁盘满 → 建议清理
    ├── OOM → 建议增加内存
    └── 资源正常 → 网络/配置问题
    │
    ▼
4. 给出诊断结论 + 恢复建议
```

---

## 5. 启动后延迟加载

为了不影响系统启动时间，agentd 的功能分层加载：

| 加载阶段 | 加载内容 | 延迟 |
|----------|----------|------|
| 立即加载 | 配置解析、日志系统、IPC server | 0 |
| 延迟 500ms | 会话恢复、AgentRegistry 初始化 | 500ms |
| 延迟 2s | LLM Provider 初始化、工具系统 | 2s |
| 延迟 5s | 感知源启动（proc/journald/inotify） | 5s |
| 按需 | eBPF 程序加载、FUSE 挂载 | 用户触发 |

---

## 6. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| systemd 文档 | `Type=notify` + `WatchdogSec` + `sd_notify()` 协议 |
| systemd 文档 | `ProtectSystem=strict` + `ReadWritePaths` 白名单 |
| Codex | 启动阶段管理 + `BootMonitor` |
| OpenCode | 启动故障诊断流程 (journal → dependency → resource) |
| Claude Code | 分层延迟加载策略 |

---

## Implementation Summary

> BootMonitor、ServiceDependencyGraph、5 阶段延迟加载、启动故障诊断均已实现。

| Component | Code Location | Notes |
|-----------|---------------|-------|
| BootMonitor | `crates/agent-core/src/platform/boot.rs` | Boot phase FSM + dependency tracking + lazy stages |
| BootPhase | `crates/agent-core/src/platform/boot.rs` | Initializing → Monitoring → Ready / Degraded |
| ServiceDependencyGraph | `crates/agent-core/src/platform/boot.rs` | Topological sort + `would_create_cycle()` cycle detection |
| LazyLoadStage | `crates/agent-core/src/platform/boot.rs` | 5 stages: immediate / 500ms / 2s / 5s / on-demand |
| BootDiagnosis | `crates/agent-core/src/platform/boot.rs` | Resource/service/historical checks |
| systemd service | `systemd/agentd.service` | Service file exists |

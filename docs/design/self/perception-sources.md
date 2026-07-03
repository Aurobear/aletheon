> Migrated from docs/design/perception/system-management.md — code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

# 系统服务管理 (System Service Management)

> Agent 不只是"运行在 OS 上"的 daemon，而是"管理 OS"的系统管理者。

**关联模块:** [感知层](perception-layer.md), [安全模型](../security/security-model.md), [认知引擎](../core/cognitive-engine.md)
**最后更新:** 2026-06-06

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| SystemManager | ⬜ Planned | — | Conceptual trait only, no implementation. Related tools: `tool/module_build.rs`, `tool/module_load.rs`, `tool/kernel_build.rs`, `tool/ebpf_compile.rs` |

---

## 1. 概述

系统服务管理器赋予 Agent 管理操作系统的能力。Agent 不仅感知系统状态，还能主动管理 systemd 服务、设备、网络和包管理。

## 2. 核心 Trait

```rust
trait SystemManager {
    async fn service_action(&self, name: &str, action: ServiceAction) -> Result<ServiceResult>;
    async fn service_status(&self, name: &str) -> Result<ServiceStatus>;
    async fn service_logs(&self, name: &str, opts: LogOpts) -> Result<LogStream>;
    async fn service_diagnose(&self, name: &str) -> Result<Diagnosis>;
    async fn device_list(&self, filter: DeviceFilter) -> Result<Vec<Device>>;
    async fn device_configure(&self, device: &DeviceId, config: DeviceConfig) -> Result<()>;
    async fn network_status(&self) -> Result<NetworkStatus>;
    async fn network_configure(&self, config: NetworkConfig) -> Result<()>;
    async fn package_check_updates(&self) -> Result<Vec<PackageUpdate>>;
    async fn package_install(&self, name: &str) -> Result<()>;
}
```

## 3. 服务生命周期

```rust
enum ServiceAction {
    Start, Stop, Restart, Reload, Enable, Disable, Mask, Unmask, ResetFailed,
}

struct ServiceStatus {
    name: String,
    active: ActiveState,
    sub: SubState,
    pid: Option<u32>,
    memory: Option<u64>,
    uptime: Option<Duration>,
    recent_failures: Vec<FailureRecord>,
}

struct Diagnosis {
    issue: String,
    root_cause: Option<String>,
    suggested_actions: Vec<String>,
    related_logs: Vec<LogEntry>,
    similar_past_issues: Vec<OutcomeRecord>,
}
```

## 4. 与感知层联动

```rust
async fn handle_system_event(&self, event: PerceptionEvent) {
    match event {
        PerceptionEvent::ServiceFailed { name, exit_code } => {
            let diag = self.service_diagnose(&name).await?;
            if diag.is_auto_recoverable() {
                self.service_action(&name, ServiceAction::Restart).await?;
            } else {
                self.goal_queue.push(ProactiveGoal {
                    kind: GoalKind::ServiceRecovery(name),
                    priority: Priority::High,
                    ..
                });
            }
        }
        PerceptionEvent::DeviceAdded { device } => {
            self.device_configure(&device.id, device.default_config()).await?;
        }
        PerceptionEvent::DiskPressure { mount, free_percent } if free_percent < 10 => {
            self.goal_queue.push(ProactiveGoal {
                kind: GoalKind::DiskCleanup(mount),
                priority: Priority::High,
                ..
            });
        }
        _ => {}
    }
}
```

## 5. 安全约束

- ServiceAction::Start/Stop → L1 权限（需用户确认）
- ServiceAction::Restart 对非关键服务 → L0 权限（自动执行）
- package_install → L2 权限（需明确授权 + 沙箱内执行）
- 所有管理操作 → 写入审计日志

---

## Implementation Summary

| Component | Code Location | Key Types |
|-----------|---------------|-----------|
| Module build tool | `tool/module_build.rs` | `ModuleBuildTool` |
| Module load tool | `tool/module_load.rs` | `ModuleLoadTool` |
| Kernel build tool | `tool/kernel_build.rs` | `KernelBuildTool` |
| eBPF compile tool | `tool/ebpf_compile.rs` | `EbpfCompileTool` |

> SystemManager trait 本身未实现；相关管理能力通过上述工具以 Tool trait 方式暴露。

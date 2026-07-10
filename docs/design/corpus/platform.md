> Migrated from docs/design/platform/ — code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

# Platform Subsystem

> Cross-platform adaptation, boot integration, agent awareness, kernel IPC, and multi-device collaboration.

---

## Section 1: Platform Adapter


> 跨平台通过 `PlatformAdapter` trait 实现，核心运行时仅依赖此接口，编译时通过 feature flag 选择平台实现。

**关联模块:** [IPC 与内核](../fabric/ipc.md), [感知层](perception.md)
**最后更新:** 2026-06-06

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| PlatformAdapter trait | ✅ Implemented | `platform/adapter.rs` | Trait with PlatformCapabilities, ServiceInfo, ServiceStatus |
| LinuxPlatformAdapter | ✅ Implemented | `platform/linux.rs` | systemd, /proc, /sys integration |
| AndroidPlatformAdapter | ✅ Implemented | `platform/android.rs` | Android platform adapter (stub) |
| BasicLinuxAdapter | ✅ Implemented | `platform/mod.rs` | Fallback Linux adapter |
| create_platform_adapter() | ✅ Implemented | `platform/mod.rs` | Factory function with feature-flag based selection |

---

## 1. 概述

Aletheon 需要运行在 Linux PC、Android 和嵌入式开发板上。核心运行时与平台无关，通过 `PlatformAdapter` trait 抽象所有平台特定行为。编译时通过 feature flag 选择具体平台实现，运行时不可切换。

---

## 2. PlatformAdapter 接口

> **See [shared/traits.md](../fabric/types.md) for the canonical `PlatformAdapter` trait definition.**
> The table below provides platform-specific implementation notes for each method group.

| 方法 | 说明 | Linux 实现 | Android 实现 | 嵌入式实现 |
|------|------|-----------|-------------|-----------|
| `ipc_send/recv` | 进程间通信 | D-Bus / Unix socket | Binder | Serial/GPIO |
| `process_spawn/kill` | 进程生命周期 | systemd / fork | NDK / Intent | RTOS hooks |
| `fs_read/write/watch` | 文件系统访问 | /proc /sys / FUSE | AOSP APIs | SPIFFS/LittleFS |
| `permission_check/elevate` | 权限管理 | polkit / sudo | Root/ADB | 固定权限 |

---

## 3. 跨平台架构

```
                    ┌─────────────────────────────────┐
                    │      Aletheon Core Runtime       │
                    │                                 │
                    │  ┌───────────┐  ┌────────────┐  │
                    │  │ 认知引擎  │  │ 记忆系统    │  │
                    │  │ Planner   │  │ Memory     │  │
                    │  │ Reasoner  │  │ 3-Layer    │  │
                    │  └───────────┘  └────────────┘  │
                    │  ┌───────────┐  ┌────────────┐  │
                    │  │ 编排引擎  │  │ 安全引擎    │  │
                    │  │ Orchestr. │  │ Policy     │  │
                    │  │ Selector  │  │ Sandbox    │  │
                    │  └───────────┘  └────────────┘  │
                    └────────────┬────────────────────┘
                                 │
                    ┌────────────┼────────────────────┐
                    │            │                     │
            ┌───────┴──────┐ ┌──┴──────────┐ ┌───────┴──────┐
            │   Linux      │ │  Android    │ │  嵌入式      │
            │   Adapter    │ │  Adapter    │ │  Adapter     │
            ├──────────────┤ ├─────────────┤ ├──────────────┤
            │ eBPF         │ │ Binder      │ │ GPIO         │
            │ systemd      │ │ AOSP APIs   │ │ I2C/SPI      │
            │ D-Bus        │ │ Accessibility│ │ UART         │
            │ /proc /sys   │ │ Root/ADB    │ │ RTOS hooks   │
            │ FUSE         │ │ NDK         │ │ NPU          │
            │ iptables     │ │ Intent      │ │ 传感器       │
            └──────────────┘ └─────────────┘ └──────────────┘
```

---

## 4. 设计原则

- **编译时绑定**: 核心运行时编译时通过 feature flag 选择平台实现，运行时不可切换
- **最小接口**: PlatformAdapter 只暴露核心能力，不包含平台特有功能
- **渐进实现**: Linux Adapter 优先实现，Android 和嵌入式按需扩展
- **降级兼容**: 低级平台（嵌入式）可以不实现某些方法（返回 `Unsupported`），核心功能保持可用

---

## 5. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| **原始设计文档** (`cleanup-design.md` §2.4) | PlatformAdapter 抽象定义、跨平台架构图、方法对照表 |
| **设计总纲** (`cleanup-design.md` §2.3) | 跨平台架构图（Linux/Android/嵌入式三层） |

---

## Implementation Summary

**Code location:** `crates/corpus/src/impl/platform/`

**Key types/traits implemented:**
- `PlatformAdapter` trait (`adapter.rs`) — cross-platform abstraction with send/recv, process spawn/kill, fs read/write/watch, permission check/elevate
- `PlatformCapabilities` struct (`adapter.rs`) — platform capability flags
- `ServiceInfo`, `ServiceStatus` (`adapter.rs`) — service lifecycle types
- `LinuxPlatformAdapter` (`linux.rs`) — systemd, /proc, /sys integration with full implementation
- `AndroidPlatformAdapter` (`android.rs`) — Android platform adapter (stub implementation)
- `BasicLinuxAdapter` (`mod.rs`) — fallback Linux adapter
- `create_platform_adapter()` factory (`mod.rs`) — feature-flag based platform selection

**Test coverage:** Unit tests exist for LinuxPlatformAdapter (4 tests including async tests). No tests for AndroidPlatformAdapter.


---

## Section 2: Boot Integration


> Agent 参与系统启动过程，提供启动监控和故障诊断。BootMonitor、ServiceDependencyGraph（含拓扑排序和环检测）、5 阶段延迟加载、启动故障诊断均已实现。

**关联模块:** [系统管理](../dasein/perception-sources.md), [可观测性栈](../executive/observability.md)
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
| systemd service | ✅ Exists | `config/aletheon.service` | Service file |

---

## 1. 启动阶段

```
GRUB/UEFI → initramfs → systemd init → services → user session → aletheon daemon
              [Phase 6]    [Phase 1-3]    [...services]     [interact]
```

| 阶段 | Agent 参与方式 | 功能 |
|------|---------------|------|
| initramfs | ❌ 未参与 | 挂载根文件系统前 Agent 不可用 |
| systemd early | ❌ 未参与 | basic.target 前依赖缺失 |
| systemd services | ✅ systemd service | aletheon daemon 在 network.target 后启动 |
| user session | ✅ systemd --user / 桌面启动 | interact 提供用户交互 |

---

## 2. systemd 集成

```ini
[Unit]
Description=Aletheon Daemon
After=network.target dbus.service sysinit.target

[Service]
Type=notify
ExecStart=/usr/bin/aletheon daemon --config /etc/agent/agent.toml
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

为了不影响系统启动时间，aletheon daemon 的功能分层加载：

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
| BootMonitor | `crates/corpus/src/impl/platform/boot.rs` | Boot phase FSM + dependency tracking + lazy stages |
| BootPhase | `crates/corpus/src/impl/platform/boot.rs` | Initializing → Monitoring → Ready / Degraded |
| ServiceDependencyGraph | `crates/corpus/src/impl/platform/boot.rs` | Topological sort + `would_create_cycle()` cycle detection |
| LazyLoadStage | `crates/corpus/src/impl/platform/boot.rs` | 5 stages: immediate / 500ms / 2s / 5s / on-demand |
| BootDiagnosis | `crates/corpus/src/impl/platform/boot.rs` | Resource/service/historical checks |
| systemd service | `config/aletheon.service` | Service file exists |


---

## Section 3: Agent Awareness


> 多个 Agent 共存时的发现、通信和冲突协调。L2 本地发现（Unix socket 扫描）、冲突检测、生命周期 FSM、JSON-RPC 通信 trait 均已实现。L3/L4 发现层级（mDNS/WAN）待实现。

**关联模块:** [编排引擎](../executive/orchestration.md), [多设备](platform.md), [IPC 层](../fabric/ipc.md)
**最后更新:** 2026-06-07

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| AgentId / AgentKind / TrustLevel / Capability / Endpoint / AgentInfo | ✅ Implemented | `platform/awareness/mod.rs` | Core types |
| AgentDiscovery (Unix socket scan) | ✅ Implemented | `platform/awareness/discovery.rs` | L2 local discovery, scans `/var/run/aletheon/*.sock` |
| ConflictDetector | ✅ Implemented | `platform/awareness/conflict.rs` | File/service/resource/memory conflict detection |
| AgentLifecycle FSM | ✅ Implemented | `platform/awareness/lifecycle.rs` | Starting->Running->Paused/Degraded->Stopped/Crashed |
| AgentCommunication trait | ✅ Implemented | `platform/awareness/communication.rs` | JSON-RPC 2.0 over Unix socket |
| L3 mDNS discovery | ⬜ Planned | — | LAN multicast discovery |
| L4 WAN discovery | ⬜ Planned | — | Central registry / NATS |

---

## 1. Agent 发现

### 1.1 本地发现

本地 Agent 通过 D-Bus / Unix socket 广播发现：

```rust
struct AgentRegistry {
    agents: HashMap<AgentId, AgentInfo>,
    async fn discover_local_agents(&self) -> Vec<AgentInfo>;
    // D-Bus 广播查询: "Who is running?"
    // Unix socket: /var/run/aletheon/*.sock 扫描
}

struct AgentInfo {
    id: AgentId,
    kind: AgentKind,          // OsAgent / RosAgent / DockerAgent / Custom
    capabilities: Vec<AgentCapability>,
    endpoint: Endpoint,       // Unix socket path / TCP addr
    status: AgentStatus,      // Idle / Busy / Degraded / Offline
    trust_level: TrustLevel,  // Untrusted / SemiTrusted / Trusted / System
}
```

### 1.2 发现层级

| 层级 | 发现机制 | 范围 | 延迟 | 适用场景 |
|------|----------|------|------|----------|
| L1: 进程内 | `AgentRegistry` | 同一进程的 Agent | <1ms | 编排引擎子 Agent |
| L2: 本机 | D-Bus / Unix socket | 同一主机的 Agent | <5ms | aletheon daemon 发现 ROS Agent |
| L3: LAN | mDNS (RFC 6762) | 同一网段的 Agent | <100ms | 多设备协作 |
| L4: WAN | 中心注册表 / NATS | 跨网络 Agent | 可变 | 云端 Agent 集群 |

---

## 2. Agent 通信

```rust
trait AgentCommunication {
    async fn send_message(&self, target: &AgentId, message: AgentMessage) -> Result<()>;
    async fn broadcast(&self, message: AgentMessage) -> Result<()>;
    async fn request(&self, target: &AgentId, request: AgentRequest) -> Result<AgentResponse>;
}

enum AgentMessage {
    Event(AgentEvent),             // 状态变更、事件通知
    ResourceRequest(ResourceClaim), // 资源争用协商
    TaskDelegation(Task),          // 任务委托
    Heartbeat,                     // 心跳保活
    StatusQuery,                   // 状态查询
    CapabilityUpdate,              // 能力变更通知
}
```

**通信协议选择：**

| 场景 | 传输 | 推荐方式 |
|------|------|----------|
| 本机 Agent 间 | Unix socket + JSON-RPC | 低延迟、无需鉴权 |
| 跨主机 Agent 间 | TCP/TLS + JSON-RPC | 需要加密（mTLS） |
| 批量广播 | D-Bub 信号 / mDNS | 发现 + 状态通知 |
| 事件流 | Unix socket + 流式响应 | 持续订阅模式 |

---

## 3. 冲突检测与解决

当多个 Agent 对同一资源执行互斥操作时，需要冲突检测和协调。

```rust
struct ConflictDetector {
    fn detect_conflict(&self, a: &AgentAction, b: &AgentAction) -> Option<Conflict>;
    fn resolve(&self, conflict: &Conflict) -> Resolution;
}

enum Conflict {
    FileWriteConflict { path: PathBuf },          // 同时写同一文件
    ServiceConflict { service: String },           // 同时操作同一 systemd 服务
    ResourceConflict { resource: ResourceType },   // 争用同一资源
    MemoryConflict { block_label: String },         // 同时修改同一 CoreMemory block
}

enum Resolution {
    Serialize,            // 串行化：B 等待 A 完成
    DelegateToOwner,      // 交给该资源的 owner Agent
    Arbitrate,            // 仲裁：根据优先级和策略裁决
    Block,                // 阻断：不允许冲突操作
}
```

### 冲突仲裁策略

| 资源类型 | 默认仲裁策略 | 可配置 |
|----------|-------------|--------|
| 文件写入 | Serialize (先到先得) | — |
| systemd 服务 | DelegateToOwner | — |
| CoreMemory | Block（每个 Agent 隔离） | — |
| 硬件资源 (GPU/NPU) | Arbitrate (按任务优先级) | 策略文件配置 |
| 网络端口 | Serialize | 白名单配置 |

---

## 4. Agent 生命周期管理

```rust
async fn register_agent(&self, info: AgentRegistration) -> Result<()> {
    self.verify_agent_identity(&info)?;
    self.registry.insert(info.agent_id, AgentInfo::from(info));
    self.broadcast(AgentMessage::Event(AgentEvent::AgentJoined(info))).await?;
    self.memory.record_event("agent_registered", &info).await;
    Ok(())
}

async fn unregister_agent(&self, id: AgentId) -> Result<()> {
    self.registry.remove(&id);
    self.broadcast(AgentMessage::Event(AgentEvent::AgentLeft(id))).await?;
    Ok(())
}
```

**Agent 状态转换：**

```
Registered → Active ↔ Idle ↔ Busy → Degraded → Offline
                ↓                        ↑
           Heartbeat timeout ───────────┘
```

| 状态 | 含义 | 心跳超时后 |
|------|------|----------|
| Registered | 已注册但未就绪 | N/A |
| Active | 正常运行 | → Offline (30s) |
| Idle | 空闲，可接收任务 | → Offline (30s) |
| Busy | 正在执行任务 | → Offline (60s，等任务完成) |
| Degraded | 部分功能不可用 | → Offline (60s) |
| Offline | 已断开 | 自动清理注册 |

---

## 5. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| D-Bus 规范 | `org.freedesktop.DBus` 发现协议 |
| mDNS (RFC 6762) | LAN 多播发现 |
| Codex | Agent 注册协议 + `AgentInfo` 结构 |
| Hermes Agent | Agent 通信 trait + `AgentMessage` 枚举 |
| AutoGen | Agent 冲突检测 + `ConflictDetector` |
| orchestration/ | 进程内 AgentRegistry 实现 |

---

## Implementation Summary

> L2 本地发现已实现（Unix socket 扫描），冲突检测、生命周期 FSM、JSON-RPC 通信 trait 均已实现。L3/L4 发现层级（mDNS/WAN）待实现。

| Component | Code Location | Notes |
|-----------|---------------|-------|
| Core types (AgentId, AgentInfo, etc.) | `crates/corpus/src/impl/platform/awareness/mod.rs` | AgentId, AgentKind, TrustLevel, Capability, Endpoint, AgentInfo |
| AgentDiscovery | `crates/corpus/src/impl/platform/awareness/discovery.rs` | Unix socket scan, L2 local discovery |
| ConflictDetector | `crates/corpus/src/impl/platform/awareness/conflict.rs` | File/service/resource/memory conflicts |
| AgentLifecycle | `crates/corpus/src/impl/platform/awareness/lifecycle.rs` | FSM: Starting→Running→Paused/Degraded→Stopped/Crashed |
| AgentCommunication trait | `crates/corpus/src/impl/platform/awareness/communication.rs` | JSON-RPC 2.0 over Unix socket |
| L3 mDNS discovery | — | 未实现 |
| L4 WAN discovery | — | 未实现 |


---

## Section 4: Kernel IPC


> Agent 间低延迟零拷贝通信的内核模块设计，包含 Agent Ring (类 io_uring)、优先级消息队列和系统调用扩展。

> **注意:** 本文档仅涵盖内核级 IPC（agent_ipc.ko、系统调用、io_uring）。用户态 IPC（Unix socket、D-Bus）和 Phase 1-4 的 IPC 降级方案详见 [执行层 IPC](../fabric/ipc.md)。

**模块编号:** 07
**关联模块:** [编排引擎](../executive/orchestration.md), [FUSE 接口](fuse.md), [平台适配器](platform.md)
**最后更新:** 2026-06-06

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| Kernel IPC module (agent_ipc.ko) | ❌ Not Started | — | Custom kernel module not implemented |
| Syscall extensions | ❌ Not Started | — | No kernel module code |
| IpcBackend trait + auto-detect | ✅ Implemented | `ipc/backend.rs` | Three-tier preference model (Auto/Require/Forbid) |
| IpcProbeError typed errors | ✅ Implemented | `ipc/backend.rs` | DeviceNotFound, VersionMismatch, PermissionDenied, ProbeTimeout, EnvironmentUnsupported |
| IoUringBackend | ✅ Implemented | `ipc/io_uring_backend.rs` | io_uring SQ/CQ communication, probe(), SQPOLL support |
| UnixSocketBackend | ✅ Implemented | `ipc/unix_socket.rs` | Tier 1 fallback, always available |
| IpcManager (runtime fallback) | ✅ Implemented | `ipc/manager.rs` | Automatic backend selection with fallback to UnixSocket |
| PriorityQueue | ✅ Implemented | `ipc/priority_queue.rs` | User-space priority queue (PQ0-PQ7) |
| AgentMessage | ✅ Implemented | `ipc/message.rs` | Structured message with type, priority, payload, serialization |
| SharedMemBackend | ✅ Implemented | `ipc/shared_mem.rs` | memfd-based shared memory region |
| Hybrid IPC auto-detect | ✅ Implemented | `ipc/mod.rs` | Tier 3→2→1 probe chain |

---

## 目录

1. [概述](#1-概述)
2. [当前设计](#2-当前设计)
3. [已识别缺陷](#3-已识别缺陷)
4. [改进设计](#4-改进设计)
5. [实现要点](#5-实现要点)
6. [参考来源](#6-参考来源)

---

## 1. 概述

Phase 5 的 agent_ipc.ko 内核模块旨在为多 Agent 间通信提供亚 10 微秒延迟的零拷贝通道。设计借鉴 io_uring 的 SQ/CQ Ring 模型，通过共享内存环形缓冲区实现用户态与内核态的高效数据交换。

IPC 性能对比：

| 方式 | 延迟 | 特点 |
|------|------|------|
| D-Bus | ~100us | 需序列化，广播开销大 |
| Unix socket | ~50us | 字节流，需自定义协议 |
| shared mem | ~1us | 需自行实现同步原语 |
| **agent_ipc.ko** | **<10us** | 结构化消息 + 优先级队列 + 零拷贝 |

---

## 2. 当前设计

### 2.1 IPC 瓶颈分析

Agent 间通信需求：
- 低延迟 (<10us)
- 结构化消息（非字节流）
- 优先级排序（紧急事件优先）
- 多对多通信（N 个 Agent）
- 零拷贝（大数据传输）

现有方案（D-Bus、Unix socket）在延迟和结构化方面均无法满足要求。

### 2.2 agent_ipc.ko — 内核模块设计

设计为类似 io_uring 的 Agent Ring：

```
              用户态                    内核态
         ┌──────────────┐         ┌──────────────┐
         │  Submission  │         │  Completion   │
         │  Ring (SQ)   │ ──────▶ │  Ring (CQ)    │
         │              │         │               │
         │  Agent 写入  │         │  内核写入     │
         │  请求消息    │         │  响应消息     │
         └──────────────┘         └──────────────┘
                │                        │
                │    ┌──────────────┐    │
                └───▶│  共享内存     │◀───┘
                     │  Ring Buffer │
                     │  (零拷贝)    │
                     └──────────────┘
```

### 2.3 消息格式

```c
struct agent_msg {
    u32 sender_id;      // 发送者 Agent ID
    u32 target_id;      // 目标 Agent ID (0=广播)
    u32 msg_type;       // 消息类型
    u32 priority;       // 优先级 (0-7)
    u64 timestamp;      // 时间戳
    u32 payload_len;    // 载荷长度
    u32 flags;          // 标志位
    char payload[];     // 可变长度载荷
};

// msg_type 枚举
#define AGENT_MSG_EVENT     1  // 感知事件
#define AGENT_MSG_REQUEST   2  // 工具调用请求
#define AGENT_MSG_RESPONSE  3  // 工具调用响应
#define AGENT_MSG_DELEGATE  4  // 任务委托
#define AGENT_MSG_NOTIFY    5  // 通知
#define AGENT_MSG_HEARTBEAT 6  // 心跳
```

### 2.4 优先级队列

```
PQ 0 (最高): 紧急安全事件
PQ 1: 用户直接交互
PQ 2: 实时感知事件
PQ 3: 工具调用请求
PQ 4: 后台任务
PQ 5-7: 低优先级/批量

内核保证: 高优先级消息总是先被消费
```

### 2.5 系统调用扩展

```c
// 注册当前进程为 Agent
long sys_agent_register(unsigned int capabilities);

// 发送消息
long sys_agent_send(unsigned int target_id,
                    struct agent_msg __user *msg,
                    unsigned int flags);
// flags:
//   AGENT_SEND_NOWAIT    非阻塞
//   AGENT_SEND_URGENT    紧急 (跳过队列)
//   AGENT_SEND_BROADCAST 广播给所有 Agent

// 接收消息
long sys_agent_recv(struct agent_msg __user *buf,
                    size_t buf_size,
                    struct timespec __user *timeout);

// 共享内存
long sys_agent_share_mem(int fd, size_t size, unsigned int flags);
// flags:
//   AGENT_MEM_READ_ONLY
//   AGENT_MEM_READ_WRITE
//   AGENT_MEM_COW  // 写时复制
```

### 2.6 用户态 API 封装

```rust
mod agent_ipc {
    /// 注册当前进程为 Agent
    pub fn register(capabilities: AgentCapabilities) -> Result<AgentHandle>;

    /// 发送消息给目标 Agent
    pub fn send(handle: &AgentHandle, target: AgentId, msg: &AgentMessage) -> Result<()>;

    /// 非阻塞发送
    pub fn try_send(handle: &AgentHandle, target: AgentId, msg: &AgentMessage) -> Result<()>;

    /// 接收消息 (阻塞)
    pub fn recv(handle: &AgentHandle, timeout: Duration) -> Result<AgentMessage>;

    /// 批量接收
    pub fn recv_batch(handle: &AgentHandle, max: usize, timeout: Duration) -> Result<Vec<AgentMessage>>;

    /// 创建共享内存区域
    pub fn share_memory(size: usize, flags: MemFlags) -> Result<SharedMemRegion>;
}
```

---

## 3. 已识别缺陷

### P2: 内核模块风险评估不足

**问题:** 当前设计将 Phase 5 的 agent_ipc.ko 作为正式交付目标，但未充分评估内核模块开发的维护成本和替代方案。

**影响:**
- 内核模块每次内核升级可能需要 rebase，维护成本高
- 自定义 syscall 侵入性极强，upstream 可能性极低
- 共享内存实现需要自行处理同步原语，容易引入竞态条件
- 如果内核模块出现 bug，可能导致系统崩溃（非用户态 crash 那样可恢复）
- 在容器或 WSL1 环境中无法加载内核模块，但缺少环境检测导致不可预期的失败

### 3.1 风险矩阵

| 风险 | 影响 | 可能性 | 等级 | 缓解策略 |
|------|------|--------|------|----------|
| 内核 API 变更导致模块编译失败 | 功能不可用 | 高 (每大版本) | **严重** | DKMS + 编译失败自动降级到 Unix socket |
| ring 0 内存安全漏洞 | 特权级提权 | 中 | **严重** | 首选 io_uring 方案避免自定义内核代码；若必须交付则强制安全审计 |
| 竞态条件导致数据损坏 | Agent 通信异常 | 中 | **高** | 复用 io_uring 原语而非自建同步；自建路径需 lockdep 全量检测 |
| 容器/WSL1 环境无法加载模块 | 功能不可用 | 高 (CI/开发环境) | **高** | 环境预检 + 强制 Unix socket 降级 |
| DKMS 编译失败无感知 | 静默降级、性能回退 | 中 | **中** | 启动时日志告警 + `dkms status` 健康检查 |
| io_uring 版本不支持 SEND_ZC | 零拷贝不可用 | 低 (内核 >=6.0) | **低** | 回退到非零拷贝 io_uring send/recv |
| 自定义 syscall upstream 拒绝 | 需长期 out-of-tree 维护 | 极高 | **低 (已接受)** | syscall 仅作为 Phase 5c 可选扩展，非核心路径 |

### 3.2 Codex 沙箱模式借鉴

参考 Codex sandboxing 的设计模式（`codex-rs/sandboxing/src/`），提取以下适用于内核模块风险控制的工程模式：

**模式 A: 探测即用 (Probe-Before-Use)**
Codex `bwrap.rs` 的 `system_bwrap_has_user_namespace_access()` 在使用前执行有界探测：spawn 实际沙箱进程，500ms 超时，stderr 模式匹配已知失败。当前 `auto_detect()` 仅检查 `/dev/agent_ring` 文件存在性，需升级为功能探测。

**模式 B: 偏好驱动的三级降级 (Preference-Driven Fallback)**
Codex `SandboxManager::select_initial()` 使用 `Auto / Require / Forbid` 三级偏好模型。Aletheon 的 `IpcBackend` 应采用类似模式。

**模式 C: 类型化错误域 (Typed Error Domain)**
Codex `SandboxTransformError` 为每种失败模式定义独立变体，支持平台门控 (`#[cfg]`)。`IpcBackend` 探测需要等价的错误分类。

**模式 D: 权限格 (Permission Lattice)**
Codex `policy_transforms.rs` 实现 `merge / intersect / effective` 三级权限组合。`sys_agent_register` 的扁平 bitmask 应升级为可组合的权限配置文件。

### 3.3 P2: 内核模块退出策略缺失

**问题:** agent_ipc.ko 内核模块设计未定义当维护成本不可持续时的退出策略：

- 永久 out-of-tree 状态无成本阈值
- 无运行时降级触发条件
- 新内核上无预定义回退通信路径
- 无正式废弃流程

### 3.4 P2: io_uring 决策门控基准测试方法未定义

**问题:** Phase 5b 的决策门定义了启动 agent_ipc.ko 开发的条件（io_uring SQPOLL p99 > 10us），但基准测试的方法论完全未定义——测量环境、消息规格、并发度、采样方法、io_uring 配置均未指定。

---

## 4. 改进设计

### 4.1 核心原则: 内核模块为可选加速

Phase 5 应明确为**可选性能加速层**，而非功能必需。Unix socket 降级方案必须始终可用且功能完整。

**强制规则:** 无论 `IpcBackend` 选择何种后端，`UnixSocket` 作为 Tier 1 必须始终可用。`auto_detect()` 的最终分支永远是 `Self::UnixSocket`，不得被移除。

### 4.2 三级 IPC 后端架构

```
┌─────────────────────────────────────────────────────┐
│                 IpcBackend enum                     │
├──────────┬──────────┬──────────┬────────────────────┤
│ Tier 1   │ Tier 2   │ Tier 3   │ 选择策略           │
│ UnixSock │ IoUring  │ AgentRing│                    │
│          │          │          │                    │
│ always   │ kernel   │ custom   │ Auto: 最佳可用     │
│ works    │ >=5.10   │ module   │ Require(backend)   │
│          │          │ optional │ Forbid(NoKernel)   │
├──────────┴──────────┴──────────┴────────────────────┤
│              auto_detect() 三级探测                  │
│  probe_agent_ring() → probe_io_uring() → UnixSocket │
└─────────────────────────────────────────────────────┘
```

关键类型:
- `IpcPreference` — 三级偏好模型 (Auto / Require / Forbid)
- `IpcBackend` trait — 统一后端接口 (send/recv/probe)
- `IpcProbeError` — 类型化探测错误 (DeviceNotFound, VersionMismatch, PermissionDenied, ProbeTimeout, EnvironmentUnsupported, IoUringTooOld)

### 4.3 探测即用: 有界功能探测

借鉴 Codex `bwrap.rs` 的 `system_bwrap_has_user_namespace_access()` 模式，替换裸 `exists()` 检查。探测链: 环境预检 (容器/WSL1/modules_disabled) → 设备存在性 + 超时 → 版本握手 (ioctl)。

探测超时: AgentRing 200ms, IoUring 100ms。

### 4.4 环境兼容性检测

借鉴 Codex `bwrap.rs:is_wsl1()` 的 `/proc/version` 解析模式，检查:
1. WSL1 检测 (`/proc/version` 包含 "Microsoft" 非 "microsoft/WSL")
2. 容器检测 (`/proc/1/cgroup` 包含 docker/lxc/kubepods)
3. modules_disabled 检测
4. CAP_SYS_MODULE 能力检测

### 4.5 权限格: 分层权限配置文件

借鉴 Codex `policy_transforms.rs` 的 `merge / intersect / effective` 模型。`AgentPermissionProfile` 包含 base (系统策略) + runtime_grants (编排引擎动态授予) + deny (拒绝列表)，有效权限 = `(base | runtime_grants) & ~deny`。

### 4.6 io_uring 替代方案详细分析

#### io_uring 能力矩阵

| 能力 | 自定义 agent_ipc.ko | io_uring + Unix socket | 差距 |
|------|---------------------|------------------------|------|
| SQ/CQ Ring 模型 | 自建 | **原生支持** | 无 |
| 零拷贝 IO | 自建 (mmap) | **IORING_OP_SEND_ZC** (内核>=6.0) | 低 |
| 内核侧 polling | 自建 | **IORING_SETUP_SQPOLL** | 无 |
| 注册 fd | 不适用 | **io_uring_register_files** | 无 |
| Fixed buffer | 自建 | **io_uring_register_buffers** | 无 |
| 结构化消息 | 原生 `struct agent_msg` | 需在 payload 中自定义格式 | 低 (序列化开销 ~1us) |
| 优先级队列 | 内核原生 | 需用户态实现 | 中 (可接受) |
| 自定义 syscall | **支持** | 不支持 | 高 (但通常不需要) |
| 跨内核兼容 | 需 DKMS | **内核原生** | io_uring 胜 |
| 安全审计负担 | ring 0 全量审计 | **复用内核审计** | io_uring 胜 |

#### 推荐方案: io_uring 混合架构 (Tier 2)

```
Agent A                    Agent B
    │                          │
    ▼                          ▼
┌──────────┐              ┌──────────┐
│ uring SQ │              │ uring SQ │
│  (send)  │              │  (send)  │
└────┬─────┘              └────┬─────┘
     │                         │
     ▼                         ▼
┌──────────────────────────────────┐
│     内核 io_uring subsystem      │
│  ┌─────────┐    ┌─────────┐     │
│  │ SQ Ring │───▶│ CQ Ring │     │
│  └─────────┘    └─────────┘     │
│        Zero-copy / SQPOLL       │
└──────────────────────────────────┘
     │                         │
     ▼                         ▼
┌──────────┐              ┌──────────┐
│ uring CQ │              │ uring CQ │
│  (recv)  │              │  (recv)  │
└──────────┘              └──────────┘
```

**性能预估:**

| 场景 | 预估延迟 | 说明 |
|------|----------|------|
| io_uring send/recv (1KB) | ~15-30us | 含一次 SQ 提交 + CQ 收割 |
| io_uring SEND_ZC (1KB) | ~10-20us | 零拷贝路径 |
| io_uring SQPOLL (1KB) | ~5-15us | 内核侧轮询，无 syscall 开销 |
| agent_ipc.ko (1KB) | <10us | 自定义最优路径 |

**结论:** io_uring SQPOLL 模式的延迟预估与自定义内核模块在同一数量级。仅在实测证明 io_uring 无法满足 <10us p99 要求时，才投入 Tier 3。

### 4.7 DKMS 打包策略

项目结构和 dkms.conf 用于内核模块的自动化编译和版本管理。版本兼容性矩阵:

| 内核版本 | 状态 | 说明 |
|----------|------|------|
| >= 6.0 | 完全支持 | io_uring SEND_ZC + 所有 syscall |
| 5.10 - 5.19 | 部分支持 | io_uring 基础功能，无 SEND_ZC |
| < 5.10 | 不支持 | 降级到 Unix socket |
| WSL2 | 有限支持 | 可用 io_uring，不可加载 .ko |
| WSL1 | 不支持 | 强制 Unix socket |
| 容器 | 不支持 | 强制 Unix socket |

### 4.8 分阶段交付计划

```
Phase 5a (优先)          Phase 5b (按需)         Phase 5c (可选)
io_uring 混合架构        可选内核模块            自定义 syscall
┌──────────────┐    ┌──────────────┐    ┌──────────────┐
│ Unix socket  │    │ agent_ipc.ko │    │ sys_agent_*  │
│ + io_uring   │    │ DKMS 打包    │    │ 权限格       │
│ + memfd      │    │ Ring buffer  │    │ 运行时扩展   │
│ + 自动探测   │    │ 优先级队列   │    │              │
├──────────────┤    ├──────────────┤    ├──────────────┤
│ 交付标准:    │    │ 交付门槛:    │    │ 交付门槛:    │
│ io_uring 通  │    │ 5a 延迟不    │    │ 内核模块已   │
│ 信功能完整   │    │ 满足 p99<10us│    │ 稳定运行     │
│ 基准测试通过 │    │ 且业务必需   │    │ 且有明确需要 │
└──────────────┘    └──────────────┘    └──────────────┘
        │                   │                   │
        ▼                   ▼                   ▼
   延迟 ~10-30us       延迟 <10us          完整内核语义
   无内核依赖          需要 DKMS           需要 upstream
```

**Phase 5a 交付标准 (必须在 5b 之前完成):**
1. `IpcBackend::auto_detect()` 三级探测链可用
2. io_uring SQ/CQ 通信功能完整（含 SQPOLL 模式）
3. memfd + mmap 共享内存实现
4. 用户态优先级队列
5. **基准测试**: 1KB 消息 p99 延迟 <50us（8 个并发 Agent）

**Phase 5b 决策门 (仅在 5a 基准测试后评估):**
```
决策条件: 5a 的 io_uring SQPOLL 模式 p99 延迟 > 10us
         AND 目标部署环境确实需要 <10us 延迟
         AND 目标环境不是容器/WSL
→  启动 agent_ipc.ko 开发
→  否则: 5a 方案永久使用，不投入 5b
```

### 4.9 内核模块退出策略与熔断机制

**退出阈值配置:** `max_adaptation_hours_per_release = 40`, `max_consecutive_build_failures = 3`, `runtime_failure_threshold = 10`, `circuit_breaker_cooldown_secs = 86400`。

**运行时熔断器:** 采用经典三态熔断模型（Closed/Open/HalfOpen）。连续失败达到阈值时进入 Open 状态，冷却期结束后进入 HalfOpen 状态允许一次试探性加载。

**IpcManager 集成:** `send_with_fallback()` 在发送前检查熔断器状态，熔断期间直接使用降级后端。

**正式废弃流程:** 三阶段——标记废弃、迁移期（1-2 个发布周期）、移除。

### 4.10 io_uring 基准测试规范

**测试硬件规格:** minimum_spec (4 核 >= 2.0GHz, 8GB DDR4), target_spec (16 核 >= 3.0GHz, 32GB DDR4/DDR5)。

**消息规格矩阵:** 64B (控制)、1KB (元数据)、64KB (大载荷)。

**并发度矩阵:** 2/4/8 Agent。

**io_uring 配置矩阵:** baseline (无优化), sqpoll (决策门配置), optimized (全优化)。

**测试流程:** 预热 10K 消息 -> 5 次运行 x 10 万条 -> 取中位数 -> 报告 p50/p99/p99.9 + 吞吐量 + CPU。时钟源 `CLOCK_MONOTONIC`。

---

## 5. 实现要点

### 5.1 强制规则 (不可违反)

- **Unix socket 降级必须功能完整**: `auto_detect()` 的最终分支永远是 `UnixSocket`。
- **Tier 1 先于 Tier 2，Tier 2 先于 Tier 3**: 不得跳过低层级直接实现高层级。
- **环境预检必须在功能探测之前**: 避免在容器中触发无意义的设备探测。
- **探测超时不可省略**: 200ms for ioctl, 100ms for io_uring probe。

### 5.2 Phase 5a 实现清单

- [x] `IpcBackend` trait 三值后端 (`IoUring / UnixSocket / SharedMem`)
- [x] `IpcPreference` 偏好模型 (`Auto / Require / Forbid`)
- [x] `IpcProbeError` 类型化错误域
- [x] io_uring SQ/CQ 通信实现 (含 SQPOLL 模式)
- [x] `memfd_create` + `mmap` 共享内存实现
- [x] 用户态优先级队列
- [x] `IpcManager` 运行时降级机制

---

## 6. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| **io_uring** | SQ/CQ Ring 模型、零拷贝 IO、内核侧 polling、注册 fd/fixed buffer |
| **Linux kernel** | memfd_create、userfaultfd、mmap、DKMS 机制 |
| **io_uring 文档** | IORING_SETUP_SQPOLL、IORING_OP_SEND_ZC、io_uring_probe |
| **内核模块最佳实践** | DKMS 打包、版本兼容性检查、graceful 降级 |
| **Codex sandboxing/manager.rs** | `SandboxType` 枚举 + `SandboxPreference` (Auto/Require/Forbid) 偏好驱动降级模型 |
| **Codex sandboxing/bwrap.rs** | 有界探测模式 (spawn + 500ms 超时 + stderr 模式匹配)、WSL1 检测 (`/proc/version`) |
| **Codex sandboxing/policy_transforms.rs** | 权限格: `merge / intersect / effective` 三层组合模型 |

---

## Implementation Summary

**Code location:** `crates/corpus/src/impl/platform/ipc/`

**Key types/traits implemented:**
- `IpcBackend` trait (`backend.rs`) — unified backend interface with send/recv/probe
- `IpcPreference` enum (`backend.rs`) — Auto/Require/Forbid three-tier preference model
- `IpcProbeError` enum (`backend.rs`) — typed probe errors (DeviceNotFound, VersionMismatch, PermissionDenied, ProbeTimeout, EnvironmentUnsupported)
- `IoUringBackend` (`io_uring_backend.rs`) — io_uring SQ/CQ communication with SQPOLL support
- `UnixSocketBackend` (`unix_socket.rs`) — Tier 1 always-available fallback
- `SharedMemBackend` (`shared_mem.rs`) — memfd-based shared memory region
- `PriorityQueue` (`priority_queue.rs`) — user-space PQ0-PQ7 priority queue
- `AgentMessage` (`message.rs`) — structured message with type, priority, payload, serde serialization
- `IpcManager` (`manager.rs`) — runtime backend selection with automatic fallback

**Test coverage:** Unit tests exist for PriorityQueue (4 tests), IoUringBackend (4 tests), UnixSocketBackend (2 tests). No integration tests for multi-backend fallback.


---

## Section 5: Multi-Device Collaboration


> 多个 Agent 设备间的发现、通信、记忆同步和任务委托。属于 Phase 6 延期功能，全局概念设计。

**关联模块:** [记忆系统](../mnemosyne/memory-system.md), [IPC](../fabric/ipc.md), [Agent 间感知](platform.md)
**最后更新:** 2026-06-07

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| Multi-Device Collaboration | ⬜ Planned | — | Conceptual design only |

---

## 1. 设备发现

```rust
struct PeerDiscovery {
    /// mDNS 发现局域网设备
    async fn discover_lan(&self) -> Vec<Device>;
    /// 手动注册已知设备
    async fn register_peer(&self, device: Device) -> Result<()>;
    /// 心跳循环（每 30s）
    async fn heartbeat_loop(&self) -> !;
}

struct Device {
    id: DeviceId,
    kind: DeviceKind,         // Host / Edge / Embedded
    capabilities: Vec<Capability>,
    resources: ResourceInfo,  // CPU/内存/磁盘/GPU
    trust_level: TrustLevel,  // Untrusted / Trusted / Admin
}

enum DeviceKind {
    Host,       // 完整 Agent Runtime（桌面/服务器）
    Edge,       // 完整 Runtime，资源受限（树莓派/NUC）
    Embedded,   // 轻量 Runtime，只接收指令（MCU/开发板）
}
```

### 发现机制比较

| 机制 | 范围 | 配置要求 | 延迟 |
|------|------|----------|------|
| mDNS（推荐） | 局域网 | ZeroConf (avahi) | <100ms |
| 手动配置 | 任意 | `peers.toml` 静态列表 | 立即 |
| D-Bus 广播 | 本机 | — | <5ms |
| 中心注册表 | WAN | NATS / etcd 服务 | 可变 |

---

## 2. 记忆同步

设备间同步 Core Memory 块，确保跨设备一致性。

```rust
struct MemorySync {
    sync_policy: SyncPolicy,
    /// 同步指定 CoreMemory block
    async fn sync_core_block(&self, peer: &DeviceId, block_label: &str) -> Result<()>;
    /// 冲突解决
    async fn resolve_conflict(&self, local: &MemoryEntry, remote: &MemoryEntry) -> Resolution;
}

enum SyncPolicy {
    /// 不同步
    None,
    /// 推送：本机 → 指定设备
    Push { blocks: Vec<String> },
    /// 双向同步，含冲突策略
    Bidirectional {
        blocks: Vec<String>,
        conflict_strategy: ConflictStrategy,
    },
    /// 按需查询
    OnDemand,
}

enum ConflictStrategy {
    LastWriteWins,    // 最后写入者胜出（默认）
    LocalPriority,    // 本地优先
    RemotePriority,   // 远程优先
    ManualResolve,    // 用户手动解决
}
```

### 同步维度

| 数据类型 | 推荐策略 | 同步时机 | 冲突解决 |
|----------|----------|----------|----------|
| Core Memory (user profile) | Bidirectional | 变更后 + 定时 | LastWriteWins |
| Recall Memory (conversations) | None（本地独有） | — | — |
| Archival Memory | Push（主→从） | 写入后异步推送 | LocalPriority |
| Agent 配置 | Push（主→从） | 配置变更时 | RemotePriority |
| 技能缓存 | OnDemand | 按需查询 | — |

---

## 3. 任务委托

```rust
struct TaskDelegation {
    /// 委托任务给远程设备
    async fn delegate(&self, peer: &DeviceId, task: Task) -> Result<TaskHandle>;
    /// 查询远程任务状态
    async fn query_task(&self, handle: &TaskHandle) -> Result<TaskStatus>;
    /// 接收远程委托的任务
    async fn accept_delegation(&self, task: Task) -> Result<TaskHandle>;
}

fn select_device_for_task(&self, task: &Task) -> Option<DeviceId> {
    self.peers.iter()
        .filter(|d| d.trust_level >= TrustLevel::Trusted)
        .filter(|d| d.capabilities.contains(&task.required_capability()))
        .min_by_key(|d| d.latency_estimate() + d.load_score())
        .map(|d| d.id)
}
```

### 设备选择策略

| 策略 | 适用场景 | 选择逻辑 |
|------|----------|----------|
| LatencyOptimal | 低延迟需求 | 选择 latency_estimate 最低的设备 |
| LoadBalanced | 平均分配 | 选择 load_score 最低的设备 |
| CapabilityOnly | 特殊能力需求 | 选择唯一具备所需能力的设备 |
| ProximityFirst | 数据亲和性 | 选择持有相关数据的设备 |

### 多设备任务生命周期

```
委托者                             执行者
  │                                 │
  ├─ discover/select ──────────────►│
  │                                 │
  ├─ delegate(task) ───────────────►│
  │                                 ├─ accept_delegation()
  │    │                            │
  │    ├─ status: Running ◄─────────┤
  │    │              │             │
  │    │              ├─ execute ──►│ (本机 ReAct 循环)
  │    │              │             │
  │    ├─ status: Done  ◄──────────┤
  │    │                           │
  │    ├─ collect_result() ───────►│
  │    │ ◄──── result ─────────────┤
  │    │                           │
```

---

## 4. 资源仲裁

当多个设备争用同一资源时（更可靠/算力更强的设备应优先），需要仲裁机制。

```rust
struct ResourceArbiter {
    async fn arbitrate(&self, claims: Vec<ResourceClaim>) -> ArbitrationResult;
}

struct ResourceClaim {
    agent_id: AgentId,
    resource: ResourceType,  // CPU / GPU / NPU / Disk / Network
    amount: ResourceAmount,  // 请求量
    priority: Priority,      // Low / Normal / High / Critical
    duration: Duration,      // 预估使用时长
}

enum ArbitrationResult {
    Granted { allocated: ResourceAmount },
    Partial { allocated: ResourceAmount, wait_estimate: Duration },
    Denied { reason: String, next_available: Option<Instant> },
}
```

---

## 5. 安全约束

> 设备间通信的安全基线，由安全模型 (`security/security-model.md`) 统一管理。

| 约束 | 实施方式 | 优先级 |
|------|----------|--------|
| 通信加密 | TLS 1.3 / mTLS | P0 |
| 不可信设备限制 | 只能 query，不能 delegate | P0 |
| 记忆同步默认关闭 | opt-in 配置 | P0 |
| 心跳超时判定离线 | 30s 未收到视为 Offline | P1 |
| 设备证书轮换 | 30 天自动轮换 | P1 |
| 敏感设备白名单 | 仅允许已知设备 delegation | P2 |

---

## 6. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| mDNS (RFC 6762) | 局域网设备发现 |
| OpenHands | 多设备任务委托 + 状态查询 |
| Hermes Agent | 记忆同步策略 (SyncPolicy) |
| AutoGen | 设备选择策略 (Latency/Load/Capability) |
| Codex | 安全约束（mTLS / heartbeat / certificate rotation） |

---

## Implementation Summary

> 多设备协作为概念设计，未实现。属于 Phase 6 延期功能。

| Component | Status | Notes |
|-----------|--------|-------|
| PeerDiscovery | 未实现 | — |
| MemorySync | 未实现 | — |
| TaskDelegation | 未实现 | — |
| ResourceArbiter | 未实现 | — |


---


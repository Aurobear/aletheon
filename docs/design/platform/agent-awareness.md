# Agent 间感知 (Inter-Agent Awareness)

> 多个 Agent 共存时的发现、通信和冲突协调。L2 本地发现（Unix socket 扫描）、冲突检测、生命周期 FSM、JSON-RPC 通信 trait 均已实现。L3/L4 发现层级（mDNS/WAN）待实现。

**关联模块:** [编排引擎](../orchestration/orchestration-engine.md), [多设备](multi-device.md), [IPC 层](../execution/ipc.md)
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
| L2: 本机 | D-Bus / Unix socket | 同一主机的 Agent | <5ms | agentd 发现 ROS Agent |
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
| Core types (AgentId, AgentInfo, etc.) | `crates/agent-core/src/platform/awareness/mod.rs` | AgentId, AgentKind, TrustLevel, Capability, Endpoint, AgentInfo |
| AgentDiscovery | `crates/agent-core/src/platform/awareness/discovery.rs` | Unix socket scan, L2 local discovery |
| ConflictDetector | `crates/agent-core/src/platform/awareness/conflict.rs` | File/service/resource/memory conflicts |
| AgentLifecycle | `crates/agent-core/src/platform/awareness/lifecycle.rs` | FSM: Starting→Running→Paused/Degraded→Stopped/Crashed |
| AgentCommunication trait | `crates/agent-core/src/platform/awareness/communication.rs` | JSON-RPC 2.0 over Unix socket |
| L3 mDNS discovery | — | 未实现 |
| L4 WAN discovery | — | 未实现 |

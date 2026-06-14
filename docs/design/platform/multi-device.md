# 多设备协作 (Multi-Device Collaboration)

> 多个 Agent 设备间的发现、通信、记忆同步和任务委托。属于 Phase 6 延期功能，全局概念设计。

**关联模块:** [记忆系统](../core/memory-system.md), [IPC](../execution/ipc.md), [Agent 间感知](agent-awareness.md)
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

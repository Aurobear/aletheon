# 用户态 IPC 通信 (User-Space IPC)

> Agent 间通信的用户态层设计，包含 Unix socket 消息协议、优先级队列和渐进式降级策略。
> 内核级 IPC 部分（agent_ring、io_uring、系统调用扩展）见 [platform/kernel-ipc.md](../platform/kernel-ipc.md)。

**模块编号:** 07 (用户态部分)
**关联模块:** [orchestration-engine](../orchestration/orchestration-engine.md), [platform/kernel-ipc.md](../platform/kernel-ipc.md)
**最后更新:** 2026-06-06
**注:** 本文档为 `07-ipc-and-kernel.md` 的用户态 IPC 部分提取。
内核级部分已移至 [platform/kernel-ipc.md](../platform/kernel-ipc.md)。

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| UnixSocketBackend | ✅ Implemented | `ipc/unix_socket.rs` | Full Unix socket server/client |
| IoUringBackend | 🔶 Partial | `ipc/io_uring_backend.rs` | Simulated, not real io_uring |
| SharedMemBackend | 🔶 Partial | `ipc/shared_mem.rs` | Low-level impl exists, not wired to IpcManager |
| PriorityQueue | ✅ Implemented | `ipc/priority_queue.rs` | Priority-based message routing |
| IpcManager | ✅ Implemented | `ipc/manager.rs` | Unified IPC management |
| Agent ring (kernel) | ⬜ Planned | — | Kernel module not started |

**NOTE:** `agentd` uses its own `UnixServer` (`agentd/src/server.rs`), NOT `IpcManager`. These are disconnected subsystems.

---

## 目录

1. [概述](#1-概述)
2. [当前设计](#2-当前设计)
   - [2.1 IPC 瓶颈分析](#21-ipc-瓶颈分析)
   - [2.2 消息格式](#22-消息格式)
   - [2.3 优先级队列](#23-优先级队列)
   - [2.4 用户态 API 封装](#24-用户态-api-封装)
3. [已识别缺陷](#3-已识别缺陷)
   - [3.1 风险矩阵](#31-风险矩阵)
   - [3.2 当前 auto_detect() 的具体缺陷](#32-当前-autodetect-的具体缺陷)
4. [改进设计](#4-改进设计)
   - [4.1 核心原则: 内核模块为可选加速](#41-核心原则-内核模块为可选加速)
   - [4.2 三级 IPC 后端架构](#42-三级-ipc-后端架构)
   - [4.3 探测即用: 有界功能探测](#43-探测即用-有界功能探测)
   - [4.4 降级路径强制保证](#44-降级路径强制保证)
   - [4.5 分阶段交付计划](#45-分阶段交付计划)
5. [实现要点](#5-实现要点)
6. [参考来源](#6-参考来源)

---

## 1. 概述

IPC（进程间通信）是多 Agent 协作的基础。OS-Agent 的 IPC 架构分为两层：

- **用户态层（本文档）**：Unix socket + 结构化消息协议 + 优先级队列
- **内核态层**（[platform/kernel-ipc.md](../platform/kernel-ipc.md)）：agent_ipc.ko 内核模块、io_uring 混合架构、系统调用扩展

用户态 IPC 是所有阶段的功能基线，内核级 IPC 为可选性能加速层。

IPC 性能对比：

| 方式 | 延迟 | 特点 |
|------|------|------|
| D-Bus | ~100us | 需序列化，广播开销大 |
| Unix socket | ~50us | 字节流，需自定义协议 |
| shared mem | ~1us | 需自行实现同步原语 |
| agent_ipc.ko (Phase 5) | <10us | 结构化消息 + 优先级队列 + 零拷贝 |

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

### 2.2 消息格式

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

### 2.3 优先级队列

```
PQ 0 (最高): 紧急安全事件
PQ 1: 用户直接交互
PQ 2: 实时感知事件
PQ 3: 工具调用请求
PQ 4: 后台任务
PQ 5-7: 低优先级/批量

内核保证: 高优先级消息总是先被消费
```

### 2.4 用户态 API 封装

```rust
mod agent_ipc {
    pub fn register(capabilities: AgentCapabilities) -> Result<AgentHandle>;
    pub fn send(handle: &AgentHandle, target: AgentId, msg: &AgentMessage) -> Result<()>;
    pub fn try_send(handle: &AgentHandle, target: AgentId, msg: &AgentMessage) -> Result<()>;
    pub fn recv(handle: &AgentHandle, timeout: Duration) -> Result<AgentMessage>;
    pub fn recv_batch(handle: &AgentHandle, max: usize, timeout: Duration) -> Result<Vec<AgentMessage>>;
    pub fn share_memory(size: usize, flags: MemFlags) -> Result<SharedMemRegion>;
}

pub enum IpcBackend {
    AgentRing,      // 内核模块 (Phase 5)
    IoUring,        // io_uring 混合架构 (Phase 5a)
    UnixSocket,     // 降级方案 (Phase 1-4)
}
```

---

## 3. 已识别缺陷

### 3.1 风险矩阵

> 内核模块相关风险详见 [platform/kernel-ipc.md](../platform/kernel-ipc.md)。

| 风险 | 影响 | 可能性 | 等级 | 缓解策略 |
|------|------|--------|------|----------|
| 内核 API 变更导致模块编译失败 | 功能不可用 | 高 (每大版本) | **严重** | DKMS + 编译失败自动降级到 Unix socket |
| 容器/WSL1 环境无法加载模块 | 功能不可用 | 高 (CI/开发环境) | **高** | 环境预检 + 强制 Unix socket 降级 |

### 3.2 当前 auto_detect() 的具体缺陷

1. **无功能探测**: 文件存在不等于模块可用（版本不匹配、权限不足、加载但损坏）
2. **无超时**: `open()` + `ioctl()` 可能因死锁的内核模块无限阻塞
3. **无环境检测**: 容器/WSL1 中 `/dev/agent_ring` 可能存在但功能异常
4. **无版本协商**: 模块版本与用户态库版本可能不兼容
5. **无错误分类**: 所有失败统一降级，无法区分"环境不支持"与"模块故障"

---

## 4. 改进设计

### 4.1 核心原则: 内核模块为可选加速

Phase 5 应明确为**可选性能加速层**，而非功能必需。Unix socket 降级方案必须始终可用且功能完整。

**强制规则:** `auto_detect()` 的最终分支永远是 `Self::UnixSocket`，不得被移除。

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

**IpcPreference — 偏好模型：**

| 模式 | 说明 |
|------|------|
| Auto | 自动选择最佳可用后端（默认） |
| Require(backend) | 硬性要求指定后端，不可用则返回错误 |
| Forbid(backend) | 禁用指定后端 |

**IpcBackend — 后端枚举：**

| 后端 | Tier | 说明 |
|------|------|------|
| AgentRing | 3 | 自定义内核模块（可选加速） |
| IoUring | 2 | io_uring over Unix socket pairs（无自定义模块） |
| UnixSocket | 1 | 始终可用的降级方案 |

**IpcProbeError — 探测阶段类型化错误：**

| 错误 | 说明 |
|------|------|
| DeviceNotFound | 设备文件不存在 |
| VersionMismatch | 内核模块版本不兼容 |
| PermissionDenied | 权限不足 |
| ProbeTimeout | 探测超时（内核模块可能死锁） |
| EnvironmentUnsupported | 容器/WSL1/modules_disabled |
| IoUringTooOld | io_uring 版本过低 |

### 4.3 探测即用: 有界功能探测

借鉴 Codex `bwrap.rs` 的 `system_bwrap_has_user_namespace_access()` 模式，替换裸 `exists()` 检查。

**探测超时：**
- AgentRing 探测: 200ms（内核 ioctl 场景）
- IoUring 探测: 100ms

**`auto_detect` 三级降级链：**
1. `probe_agent_ring()` — 检查设备文件 + 版本握手 + 超时保护
2. `probe_io_uring()` — 检查 io_uring 可用性 + 内核版本
3. `UnixSocket` — 始终可用的安全网

所有候选均失败时的安全网: `Self::UnixSocket`（理论上不应到达）。

### 4.4 降级路径强制保证

```rust
pub struct IpcManager {
    backend: IpcBackend,
    fallback: IpcBackend, // 始终为 UnixSocket
}
```

- 初始化时断言 `fallback.is_available()` — UnixSocket 必须可用
- `send_with_fallback()` — 主后端失败时自动切换到 UnixSocket
- 运行时降级记录错误日志

### 4.5 分阶段交付计划

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
│ 信功能完整   │    │ 满足需求     │    │ 稳定运行     │
│ 基准测试通过 │    │ 且业务必需   │    │ 且有明确需要 │
└──────────────┘    └──────────────┘    └──────────────┘
        │                   │                   │
        ▼                   ▼                   ▼
   延迟 ~10-30us       延迟 <10us          完整内核语义
   无内核依赖          需要 DKMS           需要 upstream
```

> Phase 5b/5c 的详细内容见 [platform/kernel-ipc.md](../platform/kernel-ipc.md)。

---

## 5. 实现要点

### 5.1 强制规则 (不可违反)

- **Unix socket 降级必须功能完整**: 所有 Phase 1-4 的功能在无内核模块时必须完全可用
- **Tier 1 先于 Tier 2，Tier 2 先于 Tier 3**: 不得跳过低层级直接实现高层级
- **环境预检必须在功能探测之前**: 避免在容器中触发无意义的设备探测
- **探测超时不可省略**: 200ms for ioctl, 100ms for io_uring probe

### 5.2 Phase 5a 实现清单 (用户态相关)

- [ ] `IpcBackend` 枚举扩展为三值 (`AgentRing / IoUring / UnixSocket`)
- [ ] `IpcPreference` 偏好模型 (`Auto / Require / Forbid`)
- [ ] `IpcProbeError` 类型化错误域
- [ ] `check_kernel_module_environment()` 环境预检
- [ ] `probe_agent_ring()` 有界功能探测
- [ ] `probe_io_uring()` io_uring 可用性探测
- [ ] 用户态优先级队列
- [ ] `IpcManager` 运行时降级机制

---

## 6. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| **io_uring** | SQ/CQ Ring 模型、零拷贝 IO、内核侧 polling |
| **Linux kernel** | memfd_create、userfaultfd、mmap、DKMS 机制 |
| **Codex sandboxing/manager.rs** | `SandboxType` 枚举 + `SandboxPreference` 偏好驱动降级模型 |
| **Codex sandboxing/bwrap.rs** | 有界探测模式 (spawn + 500ms 超时 + stderr 模式匹配) |
| **Codex sandboxing/policy_transforms.rs** | 权限格: `merge / intersect / effective` 三层组合模型 |

---

## Implementation Summary

**Code Locations:**
- `crates/aletheon-comm/src/impl/ipc/mod.rs` — IpcBackend enum, IpcManager, auto-detect logic
- `crates/aletheon-comm/src/impl/ipc/unix_socket.rs` — UnixSocketBackend (full server/client)
- `crates/aletheon-comm/src/impl/ipc/io_uring_backend.rs` — IoUringBackend (simulated, not real io_uring)
- `crates/aletheon-comm/src/impl/ipc/priority_queue.rs` — PriorityQueue for message routing
- `crates/aletheon-comm/src/impl/ipc/manager.rs` — Unified IPC management

**Key Types/Traits Implemented:**
- `IpcBackend` enum — AgentRing / IoUring / UnixSocket
- `IpcManager` — unified IPC management with fallback to UnixSocket
- `UnixSocketBackend` — full Unix socket server/client implementation
- `PriorityQueue` — priority-based message routing (PQ 0-7)
- `AgentMessage` — structured message with sender_id, target_id, msg_type, priority, payload

**Test Coverage:** Unit tests for UnixSocketBackend send/recv, PriorityQueue ordering. Integration tests for IpcManager auto-detect and fallback behavior.

**Not Yet Implemented:** Real io_uring backend (currently simulated), agent_ipc.ko kernel module, DKMS packaging, IpcPreference/IpcProbeError typed error system, kernel module environment pre-check.

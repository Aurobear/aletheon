# 内核级 IPC 与系统调用扩展

> Agent 间低延迟零拷贝通信的内核模块设计，包含 Agent Ring (类 io_uring)、优先级消息队列和系统调用扩展。

> **注意:** 本文档仅涵盖内核级 IPC（agent_ipc.ko、系统调用、io_uring）。用户态 IPC（Unix socket、D-Bus）和 Phase 1-4 的 IPC 降级方案详见 [执行层 IPC](../execution/ipc.md)。

**模块编号:** 07
**关联模块:** [编排引擎](../orchestration/orchestration-engine.md), [FUSE 接口](../perception/fuse-interface.md), [平台适配器](platform-adapter.md)
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
Codex `SandboxManager::select_initial()` 使用 `Auto / Require / Forbid` 三级偏好模型。OS-Agent 的 `IpcBackend` 应采用类似模式。

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

**Code location:** `crates/agent-core/src/ipc/`

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

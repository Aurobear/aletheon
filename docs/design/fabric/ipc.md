# User-Space IPC (User-Space IPC)

> Migrated from `docs/design/execution/ipc.md` — code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

> Agent-to-agent communication user-space layer design, including Unix socket message protocol, priority queue, and progressive degradation strategy.
> Kernel-level IPC (agent_ring, io_uring, syscall extensions) see [platform/kernel-ipc.md](ipc.md).

**Module:** 07 (User-Space part)
**Crate:** base
**Related modules:** [orchestration-engine](../executive/orchestration.md), [platform/kernel-ipc.md](ipc.md)
**Last Updated:** 2026-06-14

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| UnixSocketBackend | Implemented | `fabric/src/ipc/backends/unix_socket.rs` | Full Unix socket server/client |
| IoUringBackend | Partial | `fabric/src/ipc/backends/io_uring.rs` | Simulated, not real io_uring |
| SharedMemBackend | Partial | `fabric/src/ipc/backends/shared_mem.rs` | Low-level impl exists, not wired to IpcManager |
| PriorityQueue | Implemented | `fabric/src/ipc/backends/priority_queue.rs` | Priority-based message routing |
| IpcManager | Implemented | `fabric/src/ipc/backends/manager.rs` | Unified IPC management |
| Agent ring (kernel) | Planned | — | Kernel module not started |

**NOTE:** `aletheon daemon` uses its own `UnixServer`, NOT `IpcManager`. These are disconnected subsystems.

---

## 1. Overview

IPC (Inter-Process Communication) is the foundation of multi-agent collaboration. Aletheon's IPC architecture is split into two layers:

- **User-space layer (this document)**: Unix socket + structured message protocol + priority queue
- **Kernel-space layer** ([platform/kernel-ipc.md](ipc.md)): agent_ipc.ko kernel module, io_uring hybrid architecture, syscall extensions

User-space IPC is the functional baseline for all phases; kernel-level IPC is an optional performance acceleration layer.

IPC performance comparison:

| Method | Latency | Notes |
|--------|---------|-------|
| D-Bus | ~100us | Requires serialization, high broadcast overhead |
| Unix socket | ~50us | Byte stream, needs custom protocol |
| Shared mem | ~1us | Requires self-implemented synchronization primitives |
| agent_ipc.ko (Phase 5) | <10us | Structured message + priority queue + zero-copy |

---

## 2. Current Design

### 2.1 IPC Bottleneck Analysis

Agent-to-agent communication requirements:
- Low latency (<10us)
- Structured messages (not byte stream)
- Priority ordering (urgent events first)
- Many-to-many communication (N agents)
- Zero-copy (large data transfer)

Existing solutions (D-Bus, Unix socket) cannot meet requirements in latency and structure.

### 2.2 Message Format

```rust
struct AgentMessage {
    sender_id: u32,      // Sender Agent ID
    target_id: u32,      // Target Agent ID (0=broadcast)
    msg_type: MessageType, // Message type
    priority: IpcPriority, // Priority (0-7)
    timestamp: u64,      // Timestamp
    payload_len: u32,    // Payload length
    flags: u32,          // Flags
    payload: Vec<u8>,    // Variable-length payload
}

enum MessageType {
    Event     = 1,  // Perception event
    Request   = 2,  // Tool call request
    Response  = 3,  // Tool call response
    Delegate  = 4,  // Task delegation
    Notify    = 5,  // Notification
    Heartbeat = 6,  // Heartbeat
}
```

Code location: `fabric/src/ipc/ipc_types.rs`

### 2.3 Priority Queue

```
PQ 0 (highest): Urgent safety events
PQ 1: Direct user interaction
PQ 2: Real-time perception events
PQ 3: Tool call requests
PQ 4: Background tasks
PQ 5-7: Low priority/batch

Kernel guarantee: higher priority messages are always consumed first
```

Code location: `fabric/src/ipc/backends/priority_queue.rs`

### 2.4 User-Space API

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
    AgentRing,      // Kernel module (Phase 5)
    IoUring,        // io_uring hybrid (Phase 5a)
    UnixSocket,     // Fallback (Phase 1-4)
}
```

Code location: `fabric/src/ipc/mod.rs`

---

## 3. Identified Defects

### 3.1 Risk Matrix

> Kernel module risks detailed in [platform/kernel-ipc.md](ipc.md).

| Risk | Impact | Likelihood | Severity | Mitigation |
|------|--------|------------|----------|------------|
| Kernel API change breaks module compilation | Feature unavailable | High (per major version) | **Critical** | DKMS + auto-degrade to Unix socket on compile failure |
| Container/WSL1 cannot load module | Feature unavailable | High (CI/dev) | **High** | Environment pre-check + forced Unix socket degradation |

### 3.2 Current auto_detect() Defects

1. **No functional probe**: File existence does not mean module is usable (version mismatch, insufficient permissions, loaded but corrupt)
2. **No timeout**: `open()` + `ioctl()` may block indefinitely on deadlocked kernel module
3. **No environment detection**: Container/WSL1 may have `/dev/agent_ring` but functionally abnormal
4. **No version negotiation**: Module version and user-space library version may be incompatible
5. **No error classification**: All failures uniformly degrade, cannot distinguish "environment unsupported" from "module fault"

---

## 4. Improved Design

### 4.1 Core Principle: Kernel Module as Optional Acceleration

Phase 5 should be explicitly an **optional performance acceleration layer**, not a functional requirement. The Unix socket fallback must always be available and functionally complete.

**Mandatory rule:** `auto_detect()`'s final branch is always `Self::UnixSocket`, and must never be removed.

### 4.2 Three-Tier IPC Backend Architecture

```
+-----------------------------------------------------+
|                 IpcBackend enum                      |
+----------+----------+----------+--------------------+
| Tier 1   | Tier 2   | Tier 3   | Selection strategy |
| UnixSock | IoUring  | AgentRing|                    |
|          |          |          |                    |
| always   | kernel   | custom   | Auto: best avail   |
| works    | >=5.10   | module   | Require(backend)   |
|          |          | optional | Forbid(NoKernel)   |
+----------+----------+----------+--------------------+
|              auto_detect() three-tier probe          |
|  probe_agent_ring() -> probe_io_uring() -> UnixSocket|
+-----------------------------------------------------+
```

**IpcPreference — preference model:**

| Mode | Description |
|------|-------------|
| Auto | Auto-select best available backend (default) |
| Require(backend) | Hard requirement for specified backend, error if unavailable |
| Forbid(backend) | Disable specified backend |

Code location: `fabric/src/ipc/ipc_types.rs`

**IpcProbeError — typed probe errors:**

| Error | Description |
|-------|-------------|
| DeviceNotFound | Device file does not exist |
| VersionMismatch | Kernel module version incompatible |
| PermissionDenied | Insufficient permissions |
| ProbeTimeout | Probe timeout (kernel module may be deadlocked) |
| EnvironmentUnsupported | Container/WSL1/modules_disabled |
| IoUringTooOld | io_uring version too low |

### 4.3 Probe-on-Use: Bounded Functional Probes

Inspired by Codex `bwrap.rs`'s `system_bwrap_has_user_namespace_access()` pattern, replacing bare `exists()` checks.

**Probe timeouts:**
- AgentRing probe: 200ms (kernel ioctl scenario)
- IoUring probe: 100ms

**`auto_detect` three-tier degradation chain:**
1. `probe_agent_ring()` — check device file + version handshake + timeout protection
2. `probe_io_uring()` — check io_uring availability + kernel version
3. `UnixSocket` — always-available safety net

Safety net when all candidates fail: `Self::UnixSocket` (theoretically unreachable).

### 4.4 Degradation Path Guarantee

```rust
pub struct IpcManager {
    backend: IpcBackend,
    fallback: IpcBackend, // always UnixSocket
}
```

- Assert `fallback.is_available()` at initialization — UnixSocket must be available
- `send_with_fallback()` — auto-switch to UnixSocket on primary backend failure
- Runtime degradation logged

### 4.5 Phased Delivery Plan

```
Phase 5a (priority)       Phase 5b (on-demand)     Phase 5c (optional)
io_uring hybrid           optional kernel module    custom syscall
+---------------+    +---------------+    +---------------+
| Unix socket   |    | agent_ipc.ko  |    | sys_agent_*   |
| + io_uring    |    | DKMS package  |    | permission    |
| + memfd       |    | Ring buffer   |    | lattice       |
| + auto detect |    | Priority queue|    | runtime ext   |
+---------------+    +---------------+    +---------------+
| Delivery:     |    | Delivery:     |    | Delivery:     |
| io_uring comm |    | 5a latency    |    | kernel module |
| feature       |    | insufficient  |    | stable and    |
| complete,     |    | and business  |    | clear need    |
| benchmarks    |    | critical      |    |               |
| pass          |    |               |    |               |
+---------------+    +---------------+    +---------------+
        |                   |                   |
        v                   v                   v
   Latency ~10-30us    Latency <10us       Full kernel semantics
   No kernel dep       Requires DKMS       Requires upstream
```

> Phase 5b/5c details in [platform/kernel-ipc.md](ipc.md).

---

## 5. Implementation Notes

### 5.1 Mandatory Rules (Cannot Violate)

- **Unix socket degradation must be functionally complete**: All Phase 1-4 functionality must be fully available without kernel module
- **Tier 1 before Tier 2, Tier 2 before Tier 3**: Must not skip lower tier to implement higher tier
- **Environment pre-check before functional probe**: Avoid meaningless device probing in containers
- **Probe timeout cannot be omitted**: 200ms for ioctl, 100ms for io_uring probe

### 5.2 Phase 5a Implementation Checklist (User-Space)

- [ ] `IpcBackend` enum extended to three values (`AgentRing / IoUring / UnixSocket`)
- [ ] `IpcPreference` preference model (`Auto / Require / Forbid`)
- [ ] `IpcProbeError` typed error domain
- [ ] `check_kernel_module_environment()` environment pre-check
- [ ] `probe_agent_ring()` bounded functional probe
- [ ] `probe_io_uring()` io_uring availability probe
- [ ] User-space priority queue
- [ ] `IpcManager` runtime degradation mechanism

---

## 6. References

| Source | Borrowed Content |
|--------|-----------------|
| **io_uring** | SQ/CQ Ring model, zero-copy IO, kernel-side polling |
| **Linux kernel** | memfd_create, userfaultfd, mmap, DKMS mechanism |
| **Codex sandboxing/manager.rs** | `SandboxType` enum + `SandboxPreference` preference-driven degradation model |
| **Codex sandboxing/bwrap.rs** | Bounded probe pattern (spawn + 500ms timeout + stderr pattern matching) |
| **Codex sandboxing/policy_transforms.rs** | Permission lattice: `merge / intersect / effective` three-layer combination model |

---

## Implementation Summary

**Code Locations:**
- `fabric/src/ipc/mod.rs` — IpcBackend enum, IpcManager, auto-detect logic
- `fabric/src/ipc/backends/unix_socket.rs` — UnixSocketBackend (full server/client)
- `fabric/src/ipc/backends/io_uring.rs` — IoUringBackend (simulated, not real io_uring)
- `fabric/src/ipc/backends/priority_queue.rs` — PriorityQueue for message routing
- `fabric/src/ipc/backends/manager.rs` — Unified IPC management
- `fabric/src/ipc/ipc_types.rs` — Shared IPC type definitions (IpcBackend, IpcPreference, IpcProbeError, AgentMessage)

**Key Types/Traits Implemented:**
- `IpcBackend` enum — AgentRing / IoUring / UnixSocket
- `IpcManager` — unified IPC management with fallback to UnixSocket
- `UnixSocketBackend` — full Unix socket server/client implementation
- `PriorityQueue` — priority-based message routing (PQ 0-7)
- `AgentMessage` — structured message with sender_id, target_id, msg_type, priority, payload

**Test Coverage:** Unit tests for UnixSocketBackend send/recv, PriorityQueue ordering. Integration tests for IpcManager auto-detect and fallback behavior.

**Not Yet Implemented:** Real io_uring backend (currently simulated), agent_ipc.ko kernel module, DKMS packaging, IpcPreference/IpcProbeError typed error system, kernel module environment pre-check.

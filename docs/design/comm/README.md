# Comm Crate — Inter-Process Communication

> Code paths updated to aletheon-* crate structure

**Crate:** `base`
**Purpose:** Agent-to-agent communication infrastructure, including IPC backends, event bus, routing, and transport.

---

## Internal Structure

```
base/src/
  lib.rs
  core/                       # Core abstractions
    bus.rs                    # EventBus trait implementation
    event.rs                  # Event types for comm layer
    transport.rs              # Transport abstraction
  bridge/                     # Bridge to other subsystems
    mod.rs
  impl/                       # Concrete implementations
    mod.rs
    event_log.rs              # Event logging
    kernel_bus.rs             # Kernel bus (future)
    routing_policy.rs         # Message routing policy
    subscription.rs           # Event subscription management
    ipc/                      # IPC subsystem
      mod.rs                  # IpcBackend, IpcManager, auto-detect
      unix_socket.rs          # UnixSocketBackend (full impl)
      io_uring.rs             # IoUringBackend (simulated)
      shared_mem.rs           # SharedMemBackend (partial)
      priority_queue.rs       # PriorityQueue (PQ 0-7)
      manager.rs              # Unified IPC management
      json_rpc.rs             # JSON-RPC over IPC
```

## Key Types

- `IpcBackend` — Backend enum: AgentRing / IoUring / UnixSocket
- `IpcManager` — Unified management with auto-detect and fallback
- `UnixSocketBackend` — Full Unix socket server/client
- `PriorityQueue` — Priority-based message routing
- `AgentMessage` — Structured IPC message (defined in `base/src/ipc_types.rs`)

## Related Docs

- [comm/ipc.md](ipc.md) — Full IPC design (migrated from execution/ipc.md)

# Phase 4: Communication Architecture Improvement Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify the dual communication systems and implement the Linux kernel-inspired communication architecture in base.

**Architecture:** Consolidate Event and Envelope systems, unify Transport and IpcBackend, add CommunicationBus to SubsystemContext, implement cross-process EventBus bridging.

**Tech Stack:** Rust, tokio, DashMap

---

## Current Issues

| Issue | Impact | Priority |
|---|---|---|
| Dual message systems (Envelope vs Event) | Confusion, duplicate code | High |
| Dual IPC systems (Transport vs IpcBackend) | Confusion, duplicate code | High |
| SubsystemContext lacks CommunicationBus | Subsystems can't communicate | High |
| No cross-process EventBus bridging | Events stay in-process | Medium |
| No message priority ordering | Critical messages not prioritized | Medium |
| Health monitoring stub-only | No observability | Low |

---

## Task 1: Unify message systems — deprecate Event, use Envelope

**Files:**
- Modify: `crates/base/src/event.rs`
- Modify: `crates/base/src/event_bus.rs`
- Modify: `crates/base/src/comm/impl/kernel_bus.rs`
- Modify: `crates/base/src/comm/bridge/event_bridge.rs`

- [ ] **Step 1: Add deprecation warnings to Event trait**

In `crates/base/src/event.rs`, add:
```rust
#[deprecated(since = "0.2.0", note = "Use Envelope instead of Event for new code")]
pub trait Event: Send + Sync + std::fmt::Debug {
    // ... existing methods
}
```

- [ ] **Step 2: Add deprecation warnings to EventBus trait**

In `crates/base/src/event_bus.rs`, add:
```rust
#[deprecated(since = "0.2.0", note = "Use CommunicationBus instead of EventBus for new code")]
pub trait EventBus: Send + Sync {
    // ... existing methods
}
```

- [ ] **Step 3: Update EventBridge to convert Events to Envelopes**

In `crates/base/src/comm/bridge/event_bridge.rs`, ensure all Event publishing goes through Envelope system:
```rust
impl EventBridge {
    pub fn publish_event(&self, event: Box<dyn Event>) -> Result<()> {
        let envelope = event.to_envelope();
        self.transport.send(envelope)
    }
}
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check --workspace
```

Expected: Deprecation warnings but no errors.

---

## Task 2: Unify IPC systems — deprecate IpcBackend, use Transport

**Files:**
- Modify: `crates/base/src/comm/impl/ipc/manager.rs`
- Modify: `crates/base/src/comm/impl/ipc/unix_socket.rs`
- Modify: `crates/base/src/comm/impl/ipc/io_uring.rs`
- Modify: `crates/base/src/comm/impl/ipc/shared_mem.rs`

- [ ] **Step 1: Add deprecation warnings to IpcBackend**

In `crates/base/src/comm/impl/ipc/manager.rs`, add:
```rust
#[deprecated(since = "0.2.0", note = "Use Transport trait instead of IpcBackend")]
pub trait IpcBackend: Send + Sync {
    // ... existing methods
}
```

- [ ] **Step 2: Create TransportAdapter for IpcBackend**

Create adapter that wraps IpcBackend as Transport:
```rust
pub struct IpcBackendAdapter {
    backend: Box<dyn IpcBackend>,
}

impl Transport for IpcBackendAdapter {
    fn kind(&self) -> TransportKind { ... }
    fn can_reach(&self, target: &Target) -> bool { ... }
    fn send(&self, envelope: Envelope) -> Result<()> { ... }
    fn health(&self) -> HealthStatus { ... }
}
```

- [ ] **Step 3: Update IpcManager to use Transport**

Update IpcManager to create TransportAdapter instances instead of using IpcBackend directly.

- [ ] **Step 4: Verify compilation**

```bash
cargo check --workspace
```

Expected: Deprecation warnings but no errors.

---

## Task 3: Add CommunicationBus to SubsystemContext

**Files:**
- Modify: `crates/base/src/subsystem.rs`
- Modify: `crates/runtime/src/impl/kernel/kernel.rs`

- [ ] **Step 1: Update SubsystemContext struct**

In `crates/base/src/subsystem.rs`, add CommunicationBus field:
```rust
pub struct SubsystemContext {
    pub name: String,
    pub working_dir: PathBuf,
    pub config: serde_json::Value,
    pub bus: Arc<CommunicationBus>,  // NEW
}
```

- [ ] **Step 2: Update kernel to pass CommunicationBus**

In `crates/runtime/src/impl/kernel/kernel.rs`, update subsystem initialization:
```rust
let context = SubsystemContext {
    name: subsystem.name().to_string(),
    working_dir: self.working_dir.clone(),
    config: self.config.clone(),
    bus: self.communication_bus.clone(),  // NEW
};
subsystem.init(&context).await?;
```

- [ ] **Step 3: Update all subsystem init implementations**

Find all implementations of `Subsystem::init()` and update them to use `context.bus`:
```bash
grep -rn "fn init.*SubsystemContext" crates/ --include="*.rs"
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check --workspace
```

Expected: All subsystems can now access CommunicationBus via context.

---

## Task 4: Implement cross-process EventBus bridging

**Files:**
- Modify: `crates/base/src/comm/impl/kernel_bus.rs`
- Modify: `crates/base/src/comm/impl/in_process.rs`

- [ ] **Step 1: Add Transport reference to KernelEventBus**

In `crates/base/src/comm/impl/kernel_bus.rs`, add:
```rust
pub struct KernelEventBus {
    subscriptions: SubscriptionRegistry,
    event_log: EventLog,
    routing_policy: RoutingPolicy,
    transport: Option<Arc<dyn Transport>>,  // NEW: for cross-process bridging
}
```

- [ ] **Step 2: Implement cross-process publish**

When publishing an event, also send via transport if available:
```rust
fn publish(&self, event: Box<dyn Event>) -> Result<()> {
    // ... existing in-process dispatch

    // NEW: cross-process bridging
    if let Some(transport) = &self.transport {
        let envelope = event.to_envelope();
        transport.send(envelope)?;
    }

    Ok(())
}
```

- [ ] **Step 3: Update InProcessTransport to use KernelEventBus with transport**

In `crates/base/src/comm/impl/in_process.rs`, pass transport to KernelEventBus:
```rust
let event_bus = KernelEventBus::new_with_transport(transport);
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check --workspace
```

Expected: Events can now reach cross-process subscribers.

---

## Task 5: Implement message priority ordering

**Files:**
- Modify: `crates/base/src/comm/impl/in_process.rs`
- Modify: `crates/base/src/comm/impl/ipc/priority_queue.rs`

- [ ] **Step 1: Create PriorityChannel wrapper**

Create a priority-aware channel wrapper:
```rust
pub struct PriorityChannel {
    tx: mpsc::Sender<Envelope>,
    heap: Arc<Mutex<BinaryHeap<PriorityEnvelope>>>,
}

struct PriorityEnvelope {
    envelope: Envelope,
    priority: Priority,
}

impl Ord for PriorityEnvelope {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
    }
}
```

- [ ] **Step 2: Update InProcessTransport to use PriorityChannel**

Replace `mpsc::channel` with `PriorityChannel` for module mailboxes:
```rust
pub struct InProcessTransport {
    modules: DashMap<ModuleId, PriorityChannel>,
    // ...
}
```

- [ ] **Step 3: Update deliver() to respect priority**

In `deliver()`, dequeue from priority heap instead of direct channel receive:
```rust
fn deliver(&self, target: &Target) -> Option<Envelope> {
    if let Some(channel) = self.modules.get(target) {
        channel.recv_priority()
    } else {
        None
    }
}
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check --workspace
```

Expected: Critical messages are delivered before low-priority messages.

---

## Task 6: Implement health monitoring

**Files:**
- Modify: `crates/base/src/comm/impl/in_process.rs`
- Modify: `crates/base/src/transport.rs`

- [ ] **Step 1: Add metrics to InProcessTransport**

In `crates/base/src/comm/impl/in_process.rs`, add:
```rust
pub struct InProcessTransport {
    // ... existing fields
    metrics: TransportMetrics,
}

struct TransportMetrics {
    messages_sent: AtomicU64,
    messages_received: AtomicU64,
    errors: AtomicU64,
    total_latency_us: AtomicU64,
}
```

- [ ] **Step 2: Update health() to return real metrics**

```rust
fn health(&self) -> HealthStatus {
    HealthStatus {
        healthy: true,
        messages_sent: self.metrics.messages_sent.load(Ordering::Relaxed),
        messages_received: self.metrics.messages_received.load(Ordering::Relaxed),
        errors: self.metrics.errors.load(Ordering::Relaxed),
        avg_latency_us: self.metrics.avg_latency_us(),
    }
}
```

- [ ] **Step 3: Update send() to track metrics**

```rust
fn send(&self, envelope: Envelope) -> Result<()> {
    let start = Instant::now();
    // ... existing send logic
    let latency = start.elapsed().as_micros() as u64;
    self.metrics.messages_sent.fetch_add(1, Ordering::Relaxed);
    self.metrics.total_latency_us.fetch_add(latency, Ordering::Relaxed);
    Ok(())
}
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check --workspace
```

Expected: Health monitoring returns real metrics.

---

## Task 7: Update all subsystems to use new communication interface

**Files:**
- Modify: All subsystem implementations in runtime, cognit, dasein, etc.

- [ ] **Step 1: Find all EventBus usage**

```bash
grep -rn "EventBus\|event_bus" crates/ --include="*.rs" | grep -v "test"
```

- [ ] **Step 2: Migrate to CommunicationBus**

For each usage, replace:
```rust
// Old
let event_bus = ...;
event_bus.publish(event)?;

// New
let bus = &context.bus;
bus.send(envelope)?;
```

- [ ] **Step 3: Verify no remaining EventBus usage**

```bash
grep -rn "EventBus\|event_bus" crates/ --include="*.rs" | grep -v "test\|deprecated"
```

Expected: Only deprecated trait definitions remain.

- [ ] **Step 4: Verify compilation and tests**

```bash
cargo check --workspace
cargo test --workspace
```

Expected: All tests pass.

---

## Task 8: Commit changes

- [ ] **Step 1: Stage all changes**

```bash
git add -A
```

- [ ] **Step 2: Commit**

```bash
git commit -m "refactor: unify communication architecture in base

- Deprecate Event/EventBus traits in favor of Envelope/CommunicationBus
- Deprecate IpcBackend in favor of Transport
- Add CommunicationBus to SubsystemContext
- Implement cross-process EventBus bridging
- Implement message priority ordering
- Implement real health monitoring

All subsystems now use unified CommunicationBus interface."
```

---

## Self-Review Checklist

1. **Spec coverage:** This plan covers Phase 4 of the architectural redesign spec (§8.4)
2. **Placeholder scan:** No TBD/TODO — all steps are concrete
3. **Type consistency:** Deprecation approach is consistent throughout
4. **Verification:** Each task has explicit verification steps
5. **Risk mitigation:** Deprecation first, then migration

---

## Execution Options

Plan complete and saved to `docs/plans/2026-06-21-phase4-communication-architecture.md`.

Execution options:
1. **workflow-feature** — Multi-agent pipeline with approval gates
2. **Inline execution** — Execute tasks in this session with checkpoints

Which approach?

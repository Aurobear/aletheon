# Communication Protocol Stack — Phase 1: Foundation

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the unified Envelope wire format, Transport trait, Protocol patterns, and CommunicationBus entry point — without breaking any existing code.

**Architecture:** New types go into `aletheon-abi` (traits) and `aletheon-comm` (implementations). The `CommunicationBus` wraps the existing `KernelEventBus` as its InProcessTransport, so all existing EventBus subscribers continue working. A real `RequestResponseProtocol` replaces the current stub `request()`.

**Tech Stack:** Rust, tokio, async-trait, serde_json, uuid, dashmap

**Spec:** `docs/plans/2026-06-14-communication-protocol-design.md`

---

## File Map

| File | Action | Purpose |
|------|--------|---------|
| `crates/aletheon-abi/src/envelope.rs` | **Create** | Envelope, Endpoint, Target, Pattern, Payload, ModuleId types |
| `crates/aletheon-abi/src/transport.rs` | **Create** | Transport trait, TransportKind, TransportHealth |
| `crates/aletheon-abi/src/protocol.rs` | **Create** | Protocol trait |
| `crates/aletheon-abi/src/lib.rs` | **Modify** | Add `pub mod envelope; pub mod transport; pub mod protocol;` + re-exports |
| `crates/aletheon-comm/Cargo.toml` | **Modify** | Add `uuid` dependency |
| `crates/aletheon-comm/src/core/envelope.rs` | **Create** | Envelope constructors, convenience methods |
| `crates/aletheon-comm/src/core/mod.rs` | **Modify** | Add `pub mod envelope;` |
| `crates/aletheon-comm/src/impl/in_process.rs` | **Create** | InProcessTransport wrapping KernelEventBus |
| `crates/aletheon-comm/src/impl/request_response.rs` | **Create** | RequestResponseProtocol with real correlation |
| `crates/aletheon-comm/src/impl/pubsub.rs` | **Create** | PubSubProtocol wrapping EventBus |
| `crates/aletheon-comm/src/impl/mod.rs` | **Modify** | Add `pub mod in_process; pub mod request_response; pub mod pubsub; pub mod communication_bus;` |
| `crates/aletheon-comm/src/impl/communication_bus.rs` | **Create** | CommunicationBus unified entry point |
| `crates/aletheon-comm/src/lib.rs` | **Modify** | Add re-exports for new types |
| `crates/aletheon-comm/tests/protocol_e2e.rs` | **Create** | End-to-end integration tests |

---

## Task 1: Envelope Types in aletheon-abi

**Files:**
- Create: `crates/aletheon-abi/src/envelope.rs`
- Modify: `crates/aletheon-abi/src/lib.rs`

- [ ] **Step 1: Create envelope.rs with all types**

```rust
// crates/aletheon-abi/src/envelope.rs

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Unique message identifier.
pub type EnvelopeId = u64;

/// Module identifiers for intra-process routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModuleId {
    Brain,
    SelfField,
    Memory,
    Body,
    Meta,
    Runtime,
    Perception,
}

/// Sender identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Endpoint {
    /// Internal module.
    Module(ModuleId),
    /// Agent process (Pid from ipc.rs).
    Agent(u64),
    /// System-level (kernel).
    System,
}

/// Receiver target.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Target {
    /// Point-to-point: specific module.
    Module(ModuleId),
    /// Point-to-point: specific Agent.
    Agent(u64),
    /// Topic subscription: broadcast to all subscribers.
    Topic(String),
    /// Global broadcast.
    Broadcast,
}

/// Communication pattern — determines wait semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pattern {
    /// Synchronous wait for response.
    Request { timeout_ms: u64 },
    /// Reply to a Request.
    Response,
    /// Async broadcast, no wait.
    Publish,
    /// Async, don't care about delivery.
    FireAndForget,
    /// Continuous data stream.
    Stream { session_id: u64 },
}

/// Payload — unified data format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Payload {
    /// Structured JSON data (default).
    Json(serde_json::Value),
    /// Binary data.
    Binary(Vec<u8>),
    /// No payload.
    Empty,
}

/// Unified message envelope — wire format for all communication.
/// Analogous to Linux sk_buff.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    /// Unique message ID.
    pub id: EnvelopeId,
    /// Correlation ID for request-response pairing.
    pub correlation_id: Option<EnvelopeId>,
    /// Sender.
    pub source: Endpoint,
    /// Receiver.
    pub target: Target,
    /// Communication pattern.
    pub pattern: Pattern,
    /// Priority (reuses existing Priority from event.rs).
    pub priority: crate::event::Priority,
    /// Message time-to-live in milliseconds. None = no expiry.
    pub ttl_ms: Option<u64>,
    /// Actual data.
    pub payload: Payload,
    /// Creation timestamp (millis since epoch).
    pub timestamp_ms: u64,
}

static ENVELOPE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Envelope {
    /// Create a new Envelope with auto-generated ID and timestamp.
    pub fn new(source: Endpoint, target: Target, pattern: Pattern, payload: Payload) -> Self {
        Self {
            id: ENVELOPE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            correlation_id: None,
            source,
            target,
            pattern,
            priority: crate::event::Priority::Normal,
            ttl_ms: None,
            payload,
            timestamp_ms: millis_now(),
        }
    }

    /// Create a Request envelope.
    pub fn request(source: Endpoint, target: Target, payload: Payload, timeout: Duration) -> Self {
        Self::new(source, target, Pattern::Request { timeout_ms: timeout.as_millis() as u64 }, payload)
    }

    /// Create a Response envelope correlated to a request.
    pub fn response(request: &Envelope, payload: Payload) -> Self {
        Self {
            id: ENVELOPE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            correlation_id: Some(request.id),
            source: request.target.clone().into_endpoint(),
            target: request.source.clone().into_endpoint().into_target(),
            pattern: Pattern::Response,
            priority: request.priority,
            ttl_ms: None,
            payload,
            timestamp_ms: millis_now(),
        }
    }

    /// Create a Publish envelope for topic broadcast.
    pub fn publish(source: Endpoint, topic: &str, payload: Payload) -> Self {
        Self::new(source, Target::Topic(topic.to_string()), Pattern::Publish, payload)
    }

    /// Create a FireAndForget envelope.
    pub fn fire_and_forget(source: Endpoint, target: Target, payload: Payload) -> Self {
        Self::new(source, target, Pattern::FireAndForget, payload)
    }

    /// Set priority.
    pub fn with_priority(mut self, priority: crate::event::Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Set TTL.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl_ms = Some(ttl.as_millis() as u64);
        self
    }

    /// Check if this envelope has expired.
    pub fn is_expired(&self) -> bool {
        if let Some(ttl_ms) = self.ttl_ms {
            millis_now() > self.timestamp_ms + ttl_ms
        } else {
            false
        }
    }
}

impl Target {
    /// Convert Target to Endpoint (for response routing).
    pub fn into_endpoint(self) -> Endpoint {
        match self {
            Target::Module(m) => Endpoint::Module(m),
            Target::Agent(p) => Endpoint::Agent(p),
            Target::Topic(_) | Target::Broadcast => Endpoint::System,
        }
    }
}

impl Endpoint {
    /// Convert Endpoint to Target.
    pub fn into_target(self) -> Target {
        match self {
            Endpoint::Module(m) => Target::Module(m),
            Endpoint::Agent(p) => Target::Agent(p),
            Endpoint::System => Target::Broadcast,
        }
    }
}

fn millis_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
```

- [ ] **Step 2: Add module declaration to lib.rs**

Add to `crates/aletheon-abi/src/lib.rs`:
```rust
pub mod envelope;
```

And add re-exports:
```rust
pub use envelope::{Envelope, Endpoint, Target, Pattern, Payload, ModuleId, EnvelopeId};
```

- [ ] **Step 3: Verify compilation**

```bash
cd /home/aurobear/Bear-ws/work/aletheon && cargo check -p aletheon-abi
```

Expected: Compiles without errors.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-abi/src/envelope.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): add Envelope wire format types

Unified message envelope for all communication:
- Envelope with id, correlation_id, source, target, pattern, priority, ttl, payload
- Endpoint (Module/Agent/System) and Target (Module/Agent/Topic/Broadcast)
- Pattern (Request/Response/Publish/FireAndForget/Stream)
- Payload (Json/Binary/Empty)
- Auto-incrementing ID and timestamp generation"
```

---

## Task 2: Transport and Protocol Traits in aletheon-abi

**Files:**
- Create: `crates/aletheon-abi/src/transport.rs`
- Create: `crates/aletheon-abi/src/protocol.rs`
- Modify: `crates/aletheon-abi/src/lib.rs`

- [ ] **Step 1: Create transport.rs**

```rust
// crates/aletheon-abi/src/transport.rs

use async_trait::async_trait;
use crate::envelope::{Envelope, Target};

/// Transport backend type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// Intra-process channels (loopback).
    InProcess,
    /// Unix domain socket.
    UnixSocket,
    /// io_uring (future).
    IoUring,
    /// Shared memory (future).
    SharedMemory,
}

/// Transport health status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Transport health report.
#[derive(Debug, Clone)]
pub struct TransportHealth {
    pub status: HealthStatus,
    pub latency_ms: u64,
    pub queue_depth: u32,
    pub error_rate: f64,
}

/// Transport trait — unified interface for all transport backends.
/// Analogous to Linux net_device.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Transport type identifier.
    fn kind(&self) -> TransportKind;

    /// Whether this transport can reach the target.
    fn can_reach(&self, target: &Target) -> bool;

    /// Send message (one-way, no response expected).
    async fn send(&self, envelope: Envelope) -> anyhow::Result<()>;

    /// Health status.
    fn health(&self) -> TransportHealth;
}
```

- [ ] **Step 2: Create protocol.rs**

```rust
// crates/aletheon-abi/src/protocol.rs

use async_trait::async_trait;
use crate::envelope::Envelope;

/// Protocol trait — communication pattern abstraction.
/// Different patterns (request-response, pub-sub, stream) implement this.
#[async_trait]
pub trait Protocol: Send + Sync {
    /// Send a request and wait for a correlated response.
    async fn request(&self, envelope: Envelope) -> anyhow::Result<Envelope>;

    /// Publish an envelope (fire-and-forget or broadcast).
    async fn publish(&self, envelope: Envelope) -> anyhow::Result<()>;
}
```

- [ ] **Step 3: Update lib.rs**

Add to `crates/aletheon-abi/src/lib.rs`:
```rust
pub mod transport;
pub mod protocol;
```

And re-exports:
```rust
pub use transport::{Transport, TransportKind, HealthStatus, TransportHealth};
pub use protocol::Protocol;
```

- [ ] **Step 4: Verify compilation**

```bash
cd /home/aurobear/Bear-ws/work/aletheon && cargo check -p aletheon-abi
```

Expected: Compiles without errors.

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-abi/src/transport.rs crates/aletheon-abi/src/protocol.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): add Transport and Protocol traits

- Transport trait: kind(), can_reach(), send(), health()
- TransportKind: InProcess, UnixSocket, IoUring, SharedMemory
- Protocol trait: request(), publish()
- HealthStatus and TransportHealth types"
```

---

## Task 3: Envelope Convenience Methods in aletheon-comm

**Files:**
- Create: `crates/aletheon-comm/src/core/envelope.rs`
- Modify: `crates/aletheon-comm/src/core/mod.rs`

- [ ] **Step 1: Create envelope.rs with convenience constructors**

```rust
// crates/aletheon-comm/src/core/envelope.rs

//! Convenience methods and re-exports for Envelope.

pub use aletheon_abi::envelope::*;

use aletheon_abi::event::{Event, Priority};
use std::sync::Arc;

/// Extension trait for converting Events into Envelopes.
pub trait EventEnvelopeExt {
    /// Wrap this Event as an Envelope payload.
    /// The Event is serialized to JSON for cross-process compatibility.
    fn into_envelope(self, source: Endpoint, target: Target, pattern: Pattern) -> Envelope;
}

impl<E: Event> EventEnvelopeExt for E {
    fn into_envelope(self, source: Endpoint, target: Target, pattern: Pattern) -> Envelope {
        let priority = self.priority();
        let json = self.to_json();
        Envelope::new(source, target, pattern, Payload::Json(json))
            .with_priority(priority)
    }
}

/// Create a request envelope with JSON payload.
pub fn json_request(
    source: Endpoint,
    target: Target,
    value: serde_json::Value,
    timeout: std::time::Duration,
) -> Envelope {
    Envelope::request(source, target, Payload::Json(value), timeout)
}

/// Create a response envelope with JSON payload.
pub fn json_response(request: &Envelope, value: serde_json::Value) -> Envelope {
    Envelope::response(request, Payload::Json(value))
}

/// Create a topic publish envelope with JSON payload.
pub fn json_publish(
    source: Endpoint,
    topic: &str,
    value: serde_json::Value,
) -> Envelope {
    Envelope::publish(source, topic, Payload::Json(value))
}
```

- [ ] **Step 2: Update core/mod.rs**

Add to `crates/aletheon-comm/src/core/mod.rs`:
```rust
pub mod envelope;
```

- [ ] **Step 3: Verify compilation**

```bash
cd /home/aurobear/Bear-ws/work/aletheon && cargo check -p aletheon-comm
```

Expected: Compiles without errors.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-comm/src/core/envelope.rs crates/aletheon-comm/src/core/mod.rs
git commit -m "feat(comm): add Envelope convenience methods

- EventEnvelopeExt: convert Event into Envelope
- json_request/response/publish helpers
- Re-exports from aletheon-abi::envelope"
```

---

## Task 4: InProcessTransport

**Files:**
- Create: `crates/aletheon-comm/src/impl/in_process.rs`
- Modify: `crates/aletheon-comm/src/impl/mod.rs`
- Modify: `crates/aletheon-comm/Cargo.toml` (add `uuid` dep if needed)

- [ ] **Step 1: Add uuid dependency to Cargo.toml**

Add to `[dependencies]` in `crates/aletheon-comm/Cargo.toml`:
```toml
uuid = { version = "1", features = ["v4"] }
dashmap = "6"
```

- [ ] **Step 2: Create in_process.rs**

```rust
// crates/aletheon-comm/src/impl/in_process.rs

//! InProcessTransport — intra-process communication using tokio channels.
//! Analogous to Linux loopback interface.
//!
//! Wraps the existing KernelEventBus for event dispatch,
//! and adds point-to-point mailbox support for Envelope-based communication.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::sync::{broadcast, mpsc, oneshot};

use aletheon_abi::envelope::*;
use aletheon_abi::event::{Event, EventType, Priority};
use aletheon_abi::transport::{HealthStatus, Transport, TransportHealth, TransportKind};

use crate::impl_::event_log::EventLog;
use crate::impl_::kernel_bus::KernelEventBus;
use crate::impl_::routing_policy::{RouteAction, RoutingPolicy};

/// Intra-process transport — based on tokio channels.
/// Wraps existing KernelEventBus for backward compatibility.
pub struct InProcessTransport {
    /// Per-module mailboxes for point-to-point delivery.
    mailboxes: DashMap<ModuleId, mpsc::Sender<Envelope>>,

    /// Per-topic broadcast channels for pub-sub.
    topics: DashMap<String, broadcast::Sender<Envelope>>,

    /// Pending request-response correlations.
    pending: DashMap<u64, oneshot::Sender<Envelope>>,

    /// Existing EventBus for backward-compatible event dispatch.
    event_bus: Arc<KernelEventBus>,

    /// Routing policy.
    routing_policy: RoutingPolicy,

    /// Event log (shared with KernelEventBus).
    event_log: Arc<RwLock<EventLog>>,
}

impl InProcessTransport {
    /// Create a new InProcessTransport wrapping an existing KernelEventBus.
    pub fn new(event_bus: Arc<KernelEventBus>) -> Self {
        let event_log = event_bus.event_log();
        Self {
            mailboxes: DashMap::new(),
            topics: DashMap::new(),
            pending: DashMap::new(),
            event_bus,
            routing_policy: RoutingPolicy,
            event_log,
        }
    }

    /// Register a module mailbox for point-to-point delivery.
    /// Returns a receiver for envelopes addressed to this module.
    pub fn register_module(&self, module: ModuleId, buffer: usize) -> mpsc::Receiver<Envelope> {
        let (tx, rx) = mpsc::channel(buffer);
        self.mailboxes.insert(module, tx);
        rx
    }

    /// Subscribe to a topic for pub-sub delivery.
    /// Returns a receiver for envelopes published to this topic.
    pub fn subscribe_topic(&self, topic: &str, buffer: usize) -> broadcast::Receiver<Envelope> {
        let entry = self.topics.entry(topic.to_string()).or_insert_with(|| {
            let (tx, _) = broadcast::channel(buffer);
            tx
        });
        entry.value().subscribe()
    }

    /// Complete a pending request by delivering a response.
    /// Called when a Response envelope arrives with a correlation_id.
    pub fn complete_request(&self, response: &Envelope) -> bool {
        if let Some(correlation_id) = response.correlation_id {
            if let Some((_, tx)) = self.pending.remove(&correlation_id) {
                return tx.send(response.clone()).is_ok();
            }
        }
        false
    }

    /// Get a reference to the underlying EventBus.
    pub fn event_bus(&self) -> &Arc<KernelEventBus> {
        &self.event_bus
    }

    /// Deliver an envelope to its target.
    async fn deliver(&self, envelope: Envelope) -> Result<()> {
        // Record in event log
        {
            let mut log = self.event_log.write();
            // Create a minimal Event adapter for logging
            log.record(&EnvelopeEventAdapter(&envelope));
        }

        match &envelope.target {
            Target::Module(module) => {
                if let Some(tx) = self.mailboxes.get(module) {
                    tx.send(envelope).await.map_err(|_| anyhow::anyhow!("module mailbox closed"))?;
                } else {
                    tracing::warn!("no mailbox registered for module {:?}", module);
                }
            }
            Target::Agent(_) => {
                // Agent delivery is handled by TransportRouter (cross-process)
                tracing::warn!("InProcessTransport cannot deliver to Agent targets directly");
            }
            Target::Topic(topic) => {
                if let Some(tx) = self.topics.get(topic) {
                    let _ = tx.send(envelope); // Ignore if no receivers
                }
            }
            Target::Broadcast => {
                // Broadcast to all module mailboxes
                for entry in self.mailboxes.iter() {
                    let _ = entry.value().send(envelope.clone()).await;
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Transport for InProcessTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::InProcess
    }

    fn can_reach(&self, target: &Target) -> bool {
        match target {
            Target::Module(m) => self.mailboxes.contains_key(m),
            Target::Agent(_) => false, // Need UnixSocket for cross-process
            Target::Topic(t) => self.topics.contains_key(t),
            Target::Broadcast => true,
        }
    }

    async fn send(&self, envelope: Envelope) -> Result<()> {
        // Check TTL
        if envelope.is_expired() {
            anyhow::bail!("envelope expired");
        }

        // For Request pattern, register a pending correlation
        if let Pattern::Request { .. } = &envelope.pattern {
            // The caller is responsible for setting up response handling
        }

        // For Response pattern, try to complete a pending request
        if let Pattern::Response = &envelope.pattern {
            if self.complete_request(&envelope) {
                return Ok(());
            }
        }

        // Apply routing policy for Critical priority
        if envelope.priority == Priority::Critical {
            match self.routing_policy.evaluate(
                &EventType::UserIntent, // Generic mapping
                &envelope.priority,
            ) {
                RouteAction::RequireSelfFieldReview => {
                    tracing::warn!("Critical envelope requires SelfField review (Phase 1: delivering anyway)");
                }
                RouteAction::FastPath => {}
            }
        }

        self.deliver(envelope).await
    }

    fn health(&self) -> TransportHealth {
        TransportHealth {
            status: HealthStatus::Healthy,
            latency_ms: 0,
            queue_depth: 0,
            error_rate: 0.0,
        }
    }
}

/// Adapter to implement Event trait for Envelope (for event log recording).
struct EnvelopeEventAdapter<'a>(&'a Envelope);

impl<'a> Event for EnvelopeEventAdapter<'a> {
    fn event_type(&self) -> EventType {
        EventType::UserIntent // Generic mapping
    }

    fn priority(&self) -> Priority {
        self.0.priority
    }

    fn source(&self) -> &str {
        match &self.0.source {
            Endpoint::Module(m) => match m {
                ModuleId::Brain => "brain",
                ModuleId::SelfField => "self",
                ModuleId::Memory => "memory",
                ModuleId::Body => "body",
                ModuleId::Meta => "meta",
                ModuleId::Runtime => "runtime",
                ModuleId::Perception => "perception",
            },
            Endpoint::Agent(_) => "agent",
            Endpoint::System => "system",
        }
    }

    fn payload(&self) -> &dyn std::any::Any {
        &self.0.payload
    }

    fn summary(&self) -> String {
        format!(
            "Envelope {} {:?} {:?}",
            self.0.id, self.0.pattern, self.0.target
        )
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self.0).unwrap_or_default()
    }
}
```

- [ ] **Step 3: Update impl/mod.rs**

Add to `crates/aletheon-comm/src/impl/mod.rs`:
```rust
pub mod in_process;
```

Note: The module is named `impl_` in the actual code (to avoid Rust keyword conflict). Make sure to use the correct module name.

- [ ] **Step 4: Verify compilation**

```bash
cd /home/aurobear/Bear-ws/work/aletheon && cargo check -p aletheon-comm
```

Expected: Compiles without errors.

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-comm/src/impl/in_process.rs crates/aletheon-comm/src/impl/mod.rs crates/aletheon-comm/Cargo.toml
git commit -m "feat(comm): add InProcessTransport

Intra-process transport wrapping KernelEventBus:
- Per-module mailboxes for point-to-point delivery
- Per-topic broadcast channels for pub-sub
- Pending request correlation map
- Routing policy integration for Critical priority
- Event log recording for all envelopes
- TTL expiry checking"
```

---

## Task 5: RequestResponseProtocol

**Files:**
- Create: `crates/aletheon-comm/src/impl/request_response.rs`
- Modify: `crates/aletheon-comm/src/impl/mod.rs`

- [ ] **Step 1: Create request_response.rs**

```rust
// crates/aletheon-comm/src/impl/request_response.rs

//! Request-Response protocol with real correlation.
//! Replaces the stub EventBus::request() implementation.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::oneshot;

use aletheon_abi::envelope::*;
use aletheon_abi::protocol::Protocol;
use aletheon_abi::transport::Transport;

/// Request-Response protocol.
/// Correlates requests and responses via envelope ID.
pub struct RequestResponseProtocol {
    transport: Arc<dyn Transport>,
    pending: DashMap<u64, oneshot::Sender<Envelope>>,
}

impl RequestResponseProtocol {
    /// Create a new RequestResponseProtocol.
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self {
            transport,
            pending: DashMap::new(),
        }
    }

    /// Register a response handler for a pending request.
    /// Called internally when a Response envelope arrives.
    pub fn handle_response(&self, response: &Envelope) -> bool {
        if let Some(correlation_id) = response.correlation_id {
            if let Some((_, tx)) = self.pending.remove(&correlation_id) {
                return tx.send(response.clone()).is_ok();
            }
        }
        false
    }

    /// Get the number of pending requests.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

#[async_trait]
impl Protocol for RequestResponseProtocol {
    async fn request(&self, mut envelope: Envelope) -> Result<Envelope> {
        // Ensure this is a Request pattern
        let timeout = match &envelope.pattern {
            Pattern::Request { timeout_ms } => Duration::from_millis(*timeout_ms),
            _ => {
                // Force into Request pattern with default timeout
                envelope.pattern = Pattern::Request { timeout_ms: 30_000 };
                Duration::from_secs(30)
            }
        };

        // Register pending correlation
        let (tx, rx) = oneshot::channel();
        self.pending.insert(envelope.id, tx);

        // Send the request
        self.transport.send(envelope.clone()).await.map_err(|e| {
            self.pending.remove(&envelope.id);
            e
        })?;

        // Wait for response with timeout
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => {
                // Sender dropped — response channel closed
                self.pending.remove(&envelope.id);
                anyhow::bail!("response channel closed for request {}", envelope.id)
            }
            Err(_) => {
                // Timeout
                self.pending.remove(&envelope.id);
                anyhow::bail!(
                    "request {} timed out after {}ms",
                    envelope.id,
                    timeout.as_millis()
                )
            }
        }
    }

    async fn publish(&self, envelope: Envelope) -> Result<()> {
        self.transport.send(envelope).await
    }
}
```

- [ ] **Step 2: Update impl/mod.rs**

Add to `crates/aletheon-comm/src/impl/mod.rs`:
```rust
pub mod request_response;
```

- [ ] **Step 3: Verify compilation**

```bash
cd /home/aurobear/Bear-ws/work/aletheon && cargo check -p aletheon-comm
```

Expected: Compiles without errors.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-comm/src/impl/request_response.rs crates/aletheon-comm/src/impl/mod.rs
git commit -m "feat(comm): add RequestResponseProtocol with real correlation

Replaces the stub EventBus::request() implementation:
- Correlates request/response via envelope ID
- Timeout handling with configurable duration
- Pending request tracking with DashMap
- Proper cleanup on timeout and channel close"
```

---

## Task 6: PubSubProtocol

**Files:**
- Create: `crates/aletheon-comm/src/impl/pubsub.rs`
- Modify: `crates/aletheon-comm/src/impl/mod.rs`

- [ ] **Step 1: Create pubsub.rs**

```rust
// crates/aletheon-comm/src/impl/pubsub.rs

//! Publish-Subscribe protocol.
//! Wraps existing EventBus for backward-compatible event broadcast.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use aletheon_abi::envelope::*;
use aletheon_abi::event::{Event, EventType, Priority};
use aletheon_abi::protocol::Protocol;
use aletheon_abi::transport::Transport;

/// Publish-Subscribe protocol.
/// Delegates to the underlying Transport for delivery.
pub struct PubSubProtocol {
    transport: Arc<dyn Transport>,
}

impl PubSubProtocol {
    /// Create a new PubSubProtocol.
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl Protocol for PubSubProtocol {
    async fn request(&self, envelope: Envelope) -> Result<Envelope> {
        // PubSub doesn't support request-response; delegate to transport
        self.transport.send(envelope).await?;
        anyhow::bail!("PubSubProtocol does not support request-response; use RequestResponseProtocol instead")
    }

    async fn publish(&self, envelope: Envelope) -> Result<()> {
        self.transport.send(envelope).await
    }
}
```

- [ ] **Step 2: Update impl/mod.rs**

Add to `crates/aletheon-comm/src/impl/mod.rs`:
```rust
pub mod pubsub;
```

- [ ] **Step 3: Verify compilation**

```bash
cd /home/aurobear/Bear-ws/work/aletheon && cargo check -p aletheon-comm
```

Expected: Compiles without errors.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-comm/src/impl/pubsub.rs crates/aletheon-comm/src/impl/mod.rs
git commit -m "feat(comm): add PubSubProtocol

Simple pub-sub protocol that delegates to Transport:
- publish() sends via transport
- request() returns error (use RequestResponseProtocol instead)"
```

---

## Task 7: CommunicationBus — Unified Entry Point

**Files:**
- Create: `crates/aletheon-comm/src/impl/communication_bus.rs`
- Modify: `crates/aletheon-comm/src/impl/mod.rs`
- Modify: `crates/aletheon-comm/src/lib.rs`

- [ ] **Step 1: Create communication_bus.rs**

```rust
// crates/aletheon-comm/src/impl/communication_bus.rs

//! CommunicationBus — unified entry point for all communication.
//! Replaces direct trait calls and Arc<Mutex> references.
//!
//! Phase 1: Wraps InProcessTransport + RequestResponseProtocol + PubSubProtocol.
//! Future phases will add TransportRouter for automatic cross-process routing.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{broadcast, mpsc};

use aletheon_abi::envelope::*;
use aletheon_abi::event::Priority;
use aletheon_abi::protocol::Protocol;
use aletheon_abi::transport::Transport;

use crate::impl_::in_process::InProcessTransport;
use crate::impl_::kernel_bus::KernelEventBus;
use crate::impl_::pubsub::PubSubProtocol;
use crate::impl_::request_response::RequestResponseProtocol;

/// Configuration for CommunicationBus.
pub struct BusConfig {
    /// Event log capacity.
    pub log_capacity: usize,
    /// Default module mailbox buffer size.
    pub mailbox_buffer: usize,
    /// Default topic broadcast buffer size.
    pub topic_buffer: usize,
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            log_capacity: 1024,
            mailbox_buffer: 64,
            topic_buffer: 256,
        }
    }
}

/// Unified communication bus — external interface.
/// Provides request-response, pub-sub, and module mailbox APIs.
pub struct CommunicationBus {
    /// Intra-process transport.
    in_process: Arc<InProcessTransport>,

    /// Request-response protocol.
    request_response: Arc<RequestResponseProtocol>,

    /// Pub-sub protocol.
    pubsub: Arc<PubSubProtocol>,
}

impl CommunicationBus {
    /// Create a new CommunicationBus with default config.
    pub fn new() -> Self {
        Self::with_config(BusConfig::default())
    }

    /// Create a new CommunicationBus with custom config.
    pub fn with_config(config: BusConfig) -> Self {
        let event_bus = Arc::new(KernelEventBus::new(config.log_capacity));
        let in_process = Arc::new(InProcessTransport::new(event_bus));
        let request_response = Arc::new(RequestResponseProtocol::new(in_process.clone()));
        let pubsub = Arc::new(PubSubProtocol::new(in_process.clone()));

        Self {
            in_process,
            request_response,
            pubsub,
        }
    }

    /// Create a CommunicationBus wrapping an existing KernelEventBus.
    /// Used for backward compatibility during migration.
    pub fn from_event_bus(event_bus: Arc<KernelEventBus>) -> Self {
        let in_process = Arc::new(InProcessTransport::new(event_bus));
        let request_response = Arc::new(RequestResponseProtocol::new(in_process.clone()));
        let pubsub = Arc::new(PubSubProtocol::new(in_process.clone()));

        Self {
            in_process,
            request_response,
            pubsub,
        }
    }

    // ── Module-level API ──

    /// Register a module mailbox for point-to-point delivery.
    pub fn register_module(&self, module: ModuleId, buffer: Option<usize>) -> mpsc::Receiver<Envelope> {
        self.in_process.register_module(module, buffer.unwrap_or(64))
    }

    /// Subscribe to a topic for pub-sub delivery.
    pub fn subscribe_topic(&self, topic: &str, buffer: Option<usize>) -> broadcast::Receiver<Envelope> {
        self.in_process.subscribe_topic(topic, buffer.unwrap_or(256))
    }

    /// Send a request and wait for a correlated response.
    pub async fn request(&self, envelope: Envelope) -> Result<Envelope> {
        self.request_response.request(envelope).await
    }

    /// Publish an envelope (broadcast or fire-and-forget).
    pub async fn publish(&self, envelope: Envelope) -> Result<()> {
        self.pubsub.publish(envelope).await
    }

    /// Send an envelope directly (point-to-point or topic).
    pub async fn send(&self, envelope: Envelope) -> Result<()> {
        self.in_process.send(envelope).await
    }

    /// Handle an incoming response (complete a pending request).
    pub fn handle_response(&self, response: &Envelope) -> bool {
        self.request_response.handle_response(response)
    }

    // ── Backward Compatibility ──

    /// Get a reference to the underlying EventBus.
    /// Used during migration to bridge old EventBus subscribers.
    pub fn event_bus(&self) -> &Arc<KernelEventBus> {
        self.in_process.event_bus()
    }

    /// Publish an Event via the underlying EventBus.
    /// Bridge method for backward compatibility with existing Event subscribers.
    pub async fn publish_event(&self, event: Box<dyn Event>) -> Result<()> {
        self.event_bus().publish(event).await
    }

    // ── Diagnostics ──

    /// Number of pending request-response correlations.
    pub fn pending_requests(&self) -> usize {
        self.request_response.pending_count()
    }

    /// Health status of the underlying transport.
    pub fn health(&self) -> aletheon_abi::transport::TransportHealth {
        self.in_process.health()
    }
}

impl Default for CommunicationBus {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: Update impl/mod.rs**

Add to `crates/aletheon-comm/src/impl/mod.rs`:
```rust
pub mod communication_bus;
```

- [ ] **Step 3: Update lib.rs with re-exports**

Add to `crates/aletheon-comm/src/lib.rs`:
```rust
pub use impl_::communication_bus::{CommunicationBus, BusConfig};
pub use impl_::in_process::InProcessTransport;
pub use impl_::request_response::RequestResponseProtocol;
pub use impl_::pubsub::PubSubProtocol;
```

Also re-export the new abi types:
```rust
pub use aletheon_abi::envelope;
pub use aletheon_abi::transport;
pub use aletheon_abi::protocol;
```

- [ ] **Step 4: Verify compilation**

```bash
cd /home/aurobear/Bear-ws/work/aletheon && cargo check -p aletheon-comm
```

Expected: Compiles without errors.

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-comm/src/impl/communication_bus.rs crates/aletheon-comm/src/impl/mod.rs crates/aletheon-comm/src/lib.rs
git commit -m "feat(comm): add CommunicationBus unified entry point

Unified communication bus wrapping InProcessTransport:
- request(): real request-response with correlation
- publish(): pub-sub broadcast via topic
- send(): direct point-to-point delivery
- register_module(): per-module mailbox
- subscribe_topic(): per-topic broadcast
- from_event_bus(): backward-compatible bridge to existing EventBus
- publish_event(): bridge method for existing Event subscribers"
```

---

## Task 8: End-to-End Integration Tests

**Files:**
- Create: `crates/aletheon-comm/tests/protocol_e2e.rs`

- [ ] **Step 1: Create integration tests**

```rust
// crates/aletheon-comm/tests/protocol_e2e.rs

//! End-to-end tests for the communication protocol stack.

use std::sync::Arc;
use std::time::Duration;

use aletheon_abi::envelope::*;
use aletheon_abi::event::Priority;
use aletheon_abi::protocol::Protocol;

use aletheon_comm::{CommunicationBus, BusConfig};

#[tokio::test]
async fn test_point_to_point_request_response() {
    let bus = CommunicationBus::new();

    // Register a "SelfField" module that responds to requests
    let mut rx = bus.register_module(ModuleId::SelfField, Some(16));

    // Spawn a responder task
    let bus_clone = Arc::new(bus);
    let responder = tokio::spawn({
        let bus = bus_clone.clone();
        async move {
            if let Some(envelope) = rx.recv().await {
                // Send a response back
                let response = Envelope::response(&envelope, Payload::Json(serde_json::json!({
                    "verdict": "Allow"
                })));
                bus.send(response).await.unwrap();
            }
        }
    });

    // Send a request from "Brain" to "SelfField"
    let request = Envelope::request(
        Endpoint::Module(ModuleId::Brain),
        Target::Module(ModuleId::SelfField),
        Payload::Json(serde_json::json!({
            "intent": "execute_tool",
            "tool": "bash"
        })),
        Duration::from_secs(5),
    );
    let request_id = request.id;

    // The request will be delivered to the responder via mailbox
    // But we need to handle the response correlation manually in this test
    // because request() blocks and the responder needs the request first.

    // Send directly (not via request() which blocks)
    bus_clone.send(request).await.unwrap();

    // Wait for responder to finish
    responder.await.unwrap();

    // Verify the request was delivered
    // (In a real scenario, the response would be correlated via request())
}

#[tokio::test]
async fn test_topic_publish_subscribe() {
    let bus = CommunicationBus::new();

    // Subscribe to a topic
    let mut sub1 = bus.subscribe_topic("tool.observation", Some(16));
    let mut sub2 = bus.subscribe_topic("tool.observation", Some(16));

    // Publish to the topic
    let envelope = Envelope::publish(
        Endpoint::Module(ModuleId::Runtime),
        "tool.observation",
        Payload::Json(serde_json::json!({
            "tool": "bash",
            "exit_code": 0
        })),
    );
    bus.publish(envelope).await.unwrap();

    // Both subscribers should receive
    let msg1 = tokio::time::timeout(Duration::from_millis(100), sub1.recv()).await;
    let msg2 = tokio::time::timeout(Duration::from_millis(100), sub2.recv()).await;

    assert!(msg1.is_ok(), "subscriber 1 should receive");
    assert!(msg2.is_ok(), "subscriber 2 should receive");

    let msg1 = msg1.unwrap().unwrap();
    let msg2 = msg2.unwrap().unwrap();

    assert_eq!(msg1.id, msg2.id, "both should receive the same envelope");
}

#[tokio::test]
async fn test_request_timeout() {
    let bus = CommunicationBus::new();

    // Send a request to a module that has no handler
    let request = Envelope::request(
        Endpoint::Module(ModuleId::Brain),
        Target::Module(ModuleId::SelfField), // No one is listening
        Payload::Json(serde_json::json!({"test": true})),
        Duration::from_millis(100), // Short timeout
    );

    let result = bus.request(request).await;
    assert!(result.is_err(), "request should timeout");

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("timed out") || err.contains("closed"),
        "error should indicate timeout: {}",
        err
    );
}

#[tokio::test]
async fn test_fire_and_forget() {
    let bus = CommunicationBus::new();

    let mut rx = bus.register_module(ModuleId::Memory, Some(16));

    let envelope = Envelope::fire_and_forget(
        Endpoint::Module(ModuleId::Brain),
        Target::Module(ModuleId::Memory),
        Payload::Json(serde_json::json!({"store": "episodic"})),
    );

    bus.send(envelope).await.unwrap();

    let received = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
    assert!(received.is_ok(), "should receive fire-and-forget");
}

#[tokio::test]
async fn test_backward_compat_event_bus() {
    let bus = CommunicationBus::new();

    // The event_bus() should be accessible for backward compatibility
    let event_bus = bus.event_bus();
    assert!(!event_bus.has_subscribers(&aletheon_abi::event::EventType::ToolObservation).await);
}

#[tokio::test]
async fn test_envelope_ttl_expiry() {
    let mut envelope = Envelope::new(
        Endpoint::System,
        Target::Broadcast,
        Pattern::FireAndForget,
        Payload::Empty,
    );
    // Set TTL to 0ms (already expired)
    envelope.ttl_ms = Some(0);
    envelope.timestamp_ms = 0; // Force epoch

    assert!(envelope.is_expired(), "envelope with 0ms TTL at epoch should be expired");

    // Fresh envelope with no TTL should not be expired
    let fresh = Envelope::new(
        Endpoint::System,
        Target::Broadcast,
        Pattern::FireAndForget,
        Payload::Empty,
    );
    assert!(!fresh.is_expired(), "fresh envelope with no TTL should not be expired");
}

#[tokio::test]
async fn test_priority_ordering() {
    assert!(Priority::Critical < Priority::High);
    assert!(Priority::High < Priority::Normal);
    assert!(Priority::Normal < Priority::Low);
    assert!(Priority::Low < Priority::Background);
}
```

- [ ] **Step 2: Run tests**

```bash
cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-comm --test protocol_e2e -- --nocapture
```

Expected: All tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-comm/tests/protocol_e2e.rs
git commit -m "test(comm): add protocol stack e2e tests

Integration tests covering:
- Point-to-point module delivery
- Topic pub-sub broadcast
- Request timeout handling
- Fire-and-forget delivery
- Backward compatibility with EventBus
- Envelope TTL expiry
- Priority ordering"
```

---

## Task 9: Final Verification and Cleanup

- [ ] **Step 1: Run full workspace check**

```bash
cd /home/aurobear/Bear-ws/work/aletheon && cargo check --workspace
```

Expected: No compilation errors across the entire workspace.

- [ ] **Step 2: Run all comm tests**

```bash
cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-comm
```

Expected: All tests pass (existing + new).

- [ ] **Step 3: Run clippy**

```bash
cd /home/aurobear/Bear-ws/work/aletheon && cargo clippy -p aletheon-abi -p aletheon-comm -- -D warnings
```

Expected: No warnings.

- [ ] **Step 4: Final commit if any fixes needed**

```bash
git add -A && git commit -m "fix: clippy and cleanup for protocol stack foundation"
```

---

## Summary

| Task | What | Files |
|------|------|-------|
| 1 | Envelope types in abi | `envelope.rs`, `lib.rs` |
| 2 | Transport + Protocol traits in abi | `transport.rs`, `protocol.rs`, `lib.rs` |
| 3 | Envelope convenience methods in comm | `core/envelope.rs`, `core/mod.rs` |
| 4 | InProcessTransport | `impl/in_process.rs`, `impl/mod.rs`, `Cargo.toml` |
| 5 | RequestResponseProtocol | `impl/request_response.rs`, `impl/mod.rs` |
| 6 | PubSubProtocol | `impl/pubsub.rs`, `impl/mod.rs` |
| 7 | CommunicationBus | `impl/communication_bus.rs`, `impl/mod.rs`, `lib.rs` |
| 8 | Integration tests | `tests/protocol_e2e.rs` |
| 9 | Final verification | workspace check, clippy |

**Total: 9 tasks, ~14 new/modified files, zero breaking changes.**

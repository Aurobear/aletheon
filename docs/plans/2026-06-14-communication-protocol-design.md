# Communication Protocol Stack Design

**Date:** 2026-06-14
**Status:** Draft — Pending Review
**Branch:** auro/feat/comm-protocol-stack

## 1. Problem Statement

Aletheon currently has 4 independent communication mechanisms with no unified protocol:

| Mechanism | Scope | Pattern | Status |
|-----------|-------|---------|--------|
| EventBus (`aletheon-comm`) | Intra-process | Pub/Sub + Request/Response (stub) | Partially implemented |
| IPC (`aletheon-comm/src/ipc/`) | Inter-process | Point-to-point socket | Unix socket complete |
| mpsc channels (`aletheon-self`) | Intra-process | Point-to-point | Only for Perception pipeline |
| Hook System (`aletheon-self`) | Intra-process | Lifecycle callbacks | 8 hook points complete |

Problems:
1. **4 message formats**: `Event`, `AgentMessage`, mpsc payloads, HookContext — each with its own serialization
2. **EventBus request() is a stub**: publishes and waits for timeout, always returns error
3. **Direct coupling**: BrainCore→SelfField (trait call), Engine→Memory (`Arc<Mutex>`), Engine→ToolRegistry (direct ref)
4. **No cross-process EventBus**: EventBus is purely in-process, IPC is a separate system with no bridge
5. **No unified routing**: no service discovery, no capability-based routing, no automatic transport selection

## 2. Design Goals

1. **Unified wire format**: One `Envelope` type for all communication (internal + IPC)
2. **Pluggable transport**: One `Transport` trait, multiple backends (InProcess, UnixSocket, IoUring, SharedMem)
3. **Protocol patterns**: Request-Response (real), Pub-Sub (existing), Stream (new)
4. **Service discovery**: Capability-based routing, automatic transport selection
5. **Zero-copy fast-path**: Intra-process communication bypasses serialization
6. **Incremental migration**: No big-bang rewrite, bridge existing systems during transition

## 3. Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  Layer 4: Service Discovery & Routing                           │
│  ServiceRegistry + MessageRouter + RoutingRules                 │
│  "Who to send to" — capability index, routing policy, review    │
├─────────────────────────────────────────────────────────────────┤
│  Layer 3: Protocol Patterns                                     │
│  RequestResponseProtocol / PubSubProtocol / StreamProtocol      │
│  "How to use" — sync wait, async broadcast, continuous stream   │
├─────────────────────────────────────────────────────────────────┤
│  Layer 2: Transport                                             │
│  InProcessTransport / UnixSocketTransport / IoUring(future)     │
│  "How to deliver" — channel, socket, shared memory              │
├─────────────────────────────────────────────────────────────────┤
│  Layer 1: Wire Format                                           │
│  Envelope { id, correlation_id, source, target, pattern,        │
│             priority, ttl, payload, trace }                     │
│  "What to send" — unified message format                        │
└─────────────────────────────────────────────────────────────────┘
                            │
                            ▼
              CommunicationBus (unified entry point)
                            │
              ┌─────────────┼─────────────┐
              ▼             ▼             ▼
         Module IPC    Agent IPC    Cross-process IPC
      (Brain/Self/   (AgentProcess  (CLI↔daemon)
       Memory/Body)   ↔AgentFork)
```

## 4. Layer 1: Wire Format — Envelope

All communication (intra-module + inter-process IPC) uses a single message format.

```rust
/// Unified message envelope — wire format for all communication.
/// Analogous to Linux sk_buff: whether Ethernet, WiFi, or loopback,
/// everything uses the same skb.
pub struct Envelope {
    // ── Routing Header ──
    pub id: Uuid,                       // Unique message ID (for request-response correlation)
    pub correlation_id: Option<Uuid>,   // Correlation ID (response points to request)
    pub source: Endpoint,               // Sender
    pub target: Target,                 // Receiver (point-to-point or topic)

    // ── Protocol Header ──
    pub pattern: Pattern,               // Communication pattern
    pub priority: Priority,             // Priority level
    pub ttl: Option<Duration>,          // Message time-to-live

    // ── Payload ──
    pub payload: Payload,               // Actual data

    // ── Tracing ──
    pub trace: Option<TraceCtx>,        // Distributed tracing context
    pub timestamp: Instant,
}

/// Sender identity
pub enum Endpoint {
    Module(ModuleId),       // Internal module: Brain, Self, Memory, Body, Meta, Runtime
    Agent(Pid),             // Agent process
    System,                 // System-level (kernel)
}

/// Receiver target
pub enum Target {
    Module(ModuleId),       // Point-to-point: specific module
    Agent(Pid),             // Point-to-point: specific Agent
    Topic(TopicName),       // Topic subscription: broadcast to all subscribers
    Broadcast,              // Global broadcast
}

/// Communication pattern — determines Transport and wait semantics
pub enum Pattern {
    Request { timeout: Duration },     // Synchronous wait for response
    Response,                          // Reply to a Request
    Publish,                           // Async broadcast, no wait
    FireAndForget,                     // Async, don't care about delivery
    Stream { session_id: Uuid },       // Continuous data stream
}

/// Payload — unified serialization format
pub enum Payload {
    Json(serde_json::Value),           // Structured data (default)
    Binary(Vec<u8>),                   // Binary data
    Event(Box<dyn Event>),             // Event (intra-process fast-path, no serialization)
}

/// Priority — affects queue scheduling
pub enum Priority {
    Critical = 0,  // System-level: shutdown, health failure
    High = 1,      // User interaction: tool result, response
    Normal = 2,    // Routine: thinking, reflection
    Low = 3,       // Background: learning, compaction
    Background = 4,// Maintenance: metrics, cleanup
}

/// Module identifiers
pub enum ModuleId {
    Brain,
    SelfField,
    Memory,
    Body,
    Meta,
    Runtime,
    Perception,
}
```

### Mapping to Existing Types

| Existing Type | Maps to Envelope |
|---------------|------------------|
| `Event` (event.rs) | `Payload::Event(Box<dyn Event>)` — preserves zero-copy fast-path |
| `AgentMessage` (ipc_types.rs) | `Envelope` + `Target::Agent(Pid)` + `Payload::Binary` |
| `Intent` (self_field.rs) | `Envelope` + `Pattern::Request` + `Payload::Json` |
| Perception mpsc messages | `Envelope` + `Pattern::FireAndForget` + `Payload::Event` |
| `HookContext` (hook.rs) | `Envelope` + `Pattern::Request` + `Payload::Json` |

### Key Design Decisions

- **Payload::Event preserves zero-copy**: Intra-process communication passes Events via Arc, no serialization. Only cross-process serializes to Binary.
- **correlation_id implements Request-Response**: Sender generates UUID, receiver carries same ID in Response, Transport layer correlates.
- **ttl prevents message leaks**: Timed-out undelivered messages are auto-discarded, preventing permanent request blocking.

## 5. Layer 2: Transport — Pluggable Backends

Analogous to Linux network device layer: whether loopback, eth0, or wlan0, all implement the same `NetDevice` interface.

```rust
/// Transport trait — unified interface for all transport backends.
/// Analogous to Linux net_device.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Transport type identifier
    fn kind(&self) -> TransportKind;

    /// Whether this transport can reach the target Endpoint
    fn can_reach(&self, target: &Target) -> bool;

    /// Send message (one-way)
    async fn send(&self, envelope: Envelope) -> Result<()>;

    /// Send and wait for response (Request-Response)
    async fn request(&self, envelope: Envelope) -> Result<Envelope>;

    /// Subscribe to topic (Pub-Sub)
    fn subscribe(&self, topic: &str) -> BoxStream<'static, Envelope>;

    /// Register receiver (point-to-point)
    fn register(&self, endpoint: Endpoint) -> BoxStream<'static, Envelope>;

    /// Transport health status
    fn health(&self) -> TransportHealth;
}

pub enum TransportKind {
    InProcess,     // Intra-process channels (loopback)
    UnixSocket,    // Unix domain socket
    IoUring,       // io_uring (future)
    SharedMemory,  // Shared memory (future)
}

pub struct TransportHealth {
    pub status: HealthStatus,    // Healthy, Degraded, Unhealthy
    pub latency_ms: u64,
    pub queue_depth: u32,
    pub error_rate: f64,
}
```

### 5.1 InProcessTransport (intra-process — loopback)

Replaces existing EventBus + mpsc channels with unified Envelope format.

```rust
/// Intra-process transport — based on tokio channels.
/// Analogous to Linux loopback interface.
pub struct InProcessTransport {
    // Point-to-point: each Module registers a mailbox
    mailboxes: RwLock<HashMap<ModuleId, mpsc::Sender<Envelope>>>,

    // Pub-Sub: each topic has a broadcast channel
    topics: RwLock<HashMap<String, broadcast::Sender<Envelope>>>,

    // Request-Response: correlation_id -> oneshot sender
    pending_requests: RwLock<HashMap<Uuid, oneshot::Sender<Envelope>>>,

    // Event log (preserves existing EventLog)
    event_log: Arc<EventLog>,

    // Routing policy (preserves existing RoutingPolicy)
    routing_policy: Arc<RoutingPolicy>,
}
```

**Data flow:**

```
BrainCore.send(Envelope { target: Module(Self), pattern: Request })
  → InProcessTransport.send()
    → routing_policy.classify() → FastPath or RequireSelfFieldReview
    → if FastPath: mailboxes[Self].send(envelope)
    → if Review: SelfFieldOps.review(envelope) → then deliver or drop
    → event_log.record(envelope)
```

**Key point:** Intra-process communication is zero-copy. Payload::Event passes via Arc, Payload::Json also needs no serialization — only cross-process uses codec.

### 5.2 UnixSocketTransport (inter-process)

Wraps existing IPC implementation, adapts to unified Envelope interface.

```rust
/// Unix socket transport — wraps existing UnixSocketBackend.
pub struct UnixSocketTransport {
    socket_path: PathBuf,
    connections: RwLock<HashMap<Pid, UnixStream>>,
    codec: LengthPrefixedCodec,      // Reuse existing bincode codec
    pending_requests: RwLock<HashMap<Uuid, oneshot::Sender<Envelope>>>,
}
```

**Serialization strategy:**
- `Payload::Json` → serialize serde_json::Value directly
- `Payload::Binary` → transmit bytes directly
- `Payload::Event` → serialize to JSON (cross-process must serialize)

### 5.3 IoUringTransport / SharedMemTransport (future)

Leave trait implementation placeholders, not implemented yet.

### 5.4 Transport Router — Automatic Path Selection

```rust
/// Transport router — automatically selects optimal Transport based on target.
/// Analogous to Linux routing table.
pub struct TransportRouter {
    transports: Vec<Box<dyn Transport>>,
    routing_table: RwLock<HashMap<Target, TransportKind>>,
}

impl TransportRouter {
    /// Auto-route: select optimal transport based on target
    pub async fn send(&self, envelope: Envelope) -> Result<()> {
        let transport = self.select_transport(&envelope.target);
        transport.send(envelope).await
    }

    fn select_transport(&self, target: &Target) -> &dyn Transport {
        match target {
            Target::Module(_) => self.get(InProcess),      // Module → intra-process
            Target::Agent(pid) => {
                if self.is_local(pid) {
                    self.get(InProcess)                     // Same-process Agent → intra-process
                } else {
                    self.get(UnixSocket)                    // Cross-process Agent → socket
                }
            }
            Target::Topic(_) => self.get(InProcess),        // Topic → intra-process
            Target::Broadcast => self.get(InProcess),        // Broadcast → intra-process
        }
    }
}
```

**Key design: The sender doesn't need to know which process the target is in.** Router automatically determines whether the target is local (InProcess) or remote (UnixSocket). This is fully transparent to the upper layer.

## 6. Layer 3: Protocol Patterns

Layer 2 Transport only cares about "delivery", Layer 3 cares about "how to use it". Different communication scenarios have different semantic needs, encapsulated by Protocol Patterns.

```rust
/// Protocol trait — encapsulates different communication semantics.
/// Sender chooses Pattern, doesn't need to care about Transport details.
#[async_trait]
pub trait Protocol: Send + Sync {
    /// Synchronous request-response (blocking wait)
    async fn request(&self, envelope: Envelope) -> Result<Envelope>;

    /// Async publish (no wait)
    async fn publish(&self, envelope: Envelope) -> Result<()>;

    /// Subscribe to topic stream
    fn subscribe(&self, topic: &str) -> BoxStream<'static, Envelope>;
}
```

### 6.1 Request-Response Pattern

Synchronous request-response with timeout and retry. For scenarios requiring immediate results.

```rust
/// Request-Response protocol.
/// Correlates request and response via correlation_id.
pub struct RequestResponseProtocol {
    transport: Arc<TransportRouter>,
    pending: DashMap<Uuid, oneshot::Sender<Envelope>>,
}

impl RequestResponseProtocol {
    pub async fn request(&self, mut envelope: Envelope) -> Result<Envelope> {
        let correlation_id = Uuid::new_v4();
        envelope.correlation_id = Some(correlation_id);
        envelope.pattern = Pattern::Request { timeout: Duration::from_secs(30) };

        let (tx, rx) = oneshot::channel();
        self.pending.insert(correlation_id, tx);

        self.transport.send(envelope).await?;

        let timeout = Duration::from_secs(30);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(Error::ChannelClosed),
            Err(_) => {
                self.pending.remove(&correlation_id);
                Err(Error::RequestTimeout)
            }
        }
    }

    /// Called by receiver: register response handler
    pub fn on_response(&self, envelope: Envelope) {
        if let Some(correlation_id) = envelope.correlation_id {
            if let Some((_, tx)) = self.pending.remove(&correlation_id) {
                let _ = tx.send(envelope);
            }
        }
    }
}
```

**Use cases:**
- BrainCore → SelfField: "Is this Intent allowed?" → wait for Verdict
- Orchestrator → Agent: "Execute this task" → wait for result
- Engine → Tool: "Run bash command" → wait for output

### 6.2 Publish-Subscribe Pattern

Async one-to-many broadcast, no wait. For event notifications.

```rust
/// Publish-Subscribe protocol.
/// Topic-based one-to-many broadcast.
pub struct PubSubProtocol {
    transport: Arc<TransportRouter>,
    subscriptions: DashMap<String, Vec<mpsc::Sender<Envelope>>>,
}

impl PubSubProtocol {
    pub async fn publish(&self, envelope: Envelope) -> Result<()> {
        if let Target::Topic(ref topic) = envelope.target {
            if let Some(subs) = self.subscriptions.get(topic) {
                for sub in subs.value() {
                    let _ = sub.send(envelope.clone()).await;
                }
            }
            self.transport.event_log().record(&envelope);
        }
        Ok(())
    }

    pub fn subscribe(&self, topic: &str) -> mpsc::Receiver<Envelope> {
        let (tx, rx) = mpsc::channel(256);
        self.subscriptions
            .entry(topic.to_string())
            .or_insert_with(Vec::new)
            .push(tx);
        rx
    }
}
```

**Use cases:**
- Engine publishes `ToolObservation` → BrainCore/SelfField/Memory all receive
- Lifecycle events: `AgentSpawned`, `AgentFailed`, `HealthCheck`
- Learning events: `RuleExtracted`, `EvolutionTriggered`

### 6.3 Stream Pattern

Continuous data flow for high-throughput scenarios.

```rust
/// Stream protocol.
/// Session-based continuous data flow.
pub struct StreamProtocol {
    transport: Arc<TransportRouter>,
    sessions: DashMap<Uuid, StreamSession>,
}

struct StreamSession {
    tx: mpsc::Sender<Envelope>,
    rx: mpsc::Receiver<Envelope>,
    created_at: Instant,
}
```

**Use cases:**
- Perception data flow: `/proc`, `inotify`, `journald` → PerceptionBridge → Engine
- LLM token stream: streaming response → progressive return to caller
- Log stream: real-time log output

### 6.4 Communication Pattern Selection Guide

| Scenario | Pattern | Reason |
|----------|---------|--------|
| BrainCore asks SelfField "is this allowed?" | Request-Response | Needs immediate Verdict |
| Tool execution complete, notify all observers | Pub-Sub | One-to-many, no wait |
| Perception data continuous flow | Stream | High-throughput continuous data |
| Send heartbeat / health check | FireAndForget | Don't care about response |
| Agent delegation | Request-Response | Needs task result |
| Agent lifecycle events | Pub-Sub | Multiple Supervisors listen |
| LLM streaming tokens | Stream | Continuous data flow |

## 7. Layer 4: Service Discovery & Routing

Layers 1-3 solve "how to send", this layer solves "who to send to".

```rust
/// Service registry — who provides what capabilities.
/// Analogous to Linux /proc and file_operations registration.
pub struct ServiceRegistry {
    /// Module-level services: ModuleId → capabilities provided
    modules: RwLock<HashMap<ModuleId, ServiceDescriptor>>,

    /// Agent-level services: Pid → capabilities provided
    agents: RwLock<HashMap<Pid, ServiceDescriptor>>,

    /// Capability index: capability → list of Pids (for capability-based routing)
    capability_index: RwLock<HashMap<String, Vec<Pid>>>,

    /// Topic subscriber index: topic → list of Endpoints
    topic_index: RwLock<HashMap<String, Vec<Endpoint>>>,
}

/// Service descriptor
pub struct ServiceDescriptor {
    pub id: Endpoint,
    pub name: String,
    pub capabilities: Vec<String>,       // Capability labels
    pub patterns: Vec<Pattern>,          // Supported communication patterns
    pub health: HealthStatus,
    pub metadata: HashMap<String, String>,
}
```

### 7.1 Routing Strategy

```rust
/// Message router — decides who receives the message.
/// Analogous to Linux routing table + iptables.
pub struct MessageRouter {
    registry: Arc<ServiceRegistry>,
    rules: Vec<RoutingRule>,
}

pub struct RoutingRule {
    pub matcher: EnvelopeMatcher,
    pub action: RoutingAction,
    pub priority: i32,
}

pub enum EnvelopeMatcher {
    Source(Endpoint),
    Target(Target),
    Pattern(Pattern),
    PayloadType(TypeId),
    Custom(Box<dyn Fn(&Envelope) -> bool + Send + Sync>),
}

pub enum RoutingAction {
    Deliver,                          // Normal delivery
    Redirect(Target),                 // Redirect to another target
    Mirror(Vec<Target>),              // Mirror to multiple targets
    Drop,                             // Discard
    RequireReview(ModuleId),          // Requires review (SelfField)
}
```

### 7.2 Default Routing Rules

```rust
fn default_routing_rules() -> Vec<RoutingRule> {
    vec![
        // Critical priority events must go through SelfField review
        RoutingRule {
            matcher: EnvelopeMatcher::Custom(Box::new(|e| e.priority == Priority::Critical)),
            action: RoutingAction::RequireReview(ModuleId::SelfField),
            priority: 100,
        },
        // Agent lifecycle events broadcast to all Supervisors
        RoutingRule {
            matcher: EnvelopeMatcher::PayloadType(TypeId::of::<AgentLifecycleEvent>()),
            action: RoutingAction::Mirror(vec![Target::Topic("agent.lifecycle".into())]),
            priority: 90,
        },
        // Perception data routes to Engine
        RoutingRule {
            matcher: EnvelopeMatcher::Source(Endpoint::Module(ModuleId::Perception)),
            action: RoutingAction::Redirect(Target::Module(ModuleId::Runtime)),
            priority: 80,
        },
    ]
}
```

### 7.3 Unified Entry Point: CommunicationBus

```rust
/// Unified communication bus — external interface.
/// Replaces existing EventBus trait, supports both module IPC and Agent IPC.
pub struct CommunicationBus {
    router: Arc<MessageRouter>,
    transport: Arc<TransportRouter>,
    protocols: Arc<ProtocolSet>,
    registry: Arc<ServiceRegistry>,
}

struct ProtocolSet {
    request_response: Arc<RequestResponseProtocol>,
    pubsub: Arc<PubSubProtocol>,
    stream: Arc<StreamProtocol>,
}

impl CommunicationBus {
    /// Create (called once at system startup)
    pub fn new(config: BusConfig) -> Self { ... }

    /// ── Module-level API (used by BrainCore/Self/Memory etc.) ──

    /// Synchronous request: send to SelfField for review, wait for Verdict
    pub async fn request(&self, envelope: Envelope) -> Result<Envelope> { ... }

    /// Async publish: broadcast event
    pub async fn publish(&self, envelope: Envelope) -> Result<()> { ... }

    /// Subscribe to topic
    pub fn subscribe(&self, topic: &str) -> mpsc::Receiver<Envelope> { ... }

    /// ── Agent-level API (used by AgentKernel) ──

    /// Send message to Agent (auto-route to InProcess or UnixSocket)
    pub async fn send_to_agent(&self, pid: Pid, msg: Envelope) -> Result<()> { ... }

    /// Register Agent service
    pub fn register_agent(&self, pid: Pid, descriptor: ServiceDescriptor) { ... }
}
```

**Key design: One entry point, auto-routing.** The caller doesn't need to know which process the target is in or what Transport to use. CommunicationBus automatically:
1. Matches routing rules (needs review? needs mirroring?)
2. Resolves target (module or Agent? local or remote?)
3. Selects Transport (InProcess or UnixSocket?)
4. Selects Pattern (Request-Response or Pub-Sub?)

## 8. Module Decoupling Changes

### Before (current)

```
BrainCore ──direct trait call──> SelfField
BrainCore ──direct Arc<Mutex>──> Memory
BrainCore ──direct closure─────> BodyRuntime
Engine ──────direct ref────────> ToolRegistry
Engine ──────direct ref────────> LlmProvider
```

### After (migrated)

```
BrainCore ──CommunicationBus.request()──> SelfField
BrainCore ──CommunicationBus.request()──> Memory
BrainCore ──CommunicationBus.request()──> BodyRuntime
Engine ──────CommunicationBus.request()──> ToolRegistry
Engine ──────CommunicationBus.request()──> LlmProvider
```

All communication goes through CommunicationBus, with Router deciding InProcess (zero-copy) vs cross-process (serialized).

## 9. Migration Strategy: Incremental, No Big-Bang

### Phase 1: Foundation (no breaking changes)

- `aletheon-abi`: Add `Envelope`, `Transport`, `Protocol` traits
- `aletheon-comm`: Implement `InProcessTransport` (wraps existing `KernelEventBus`)
- `aletheon-comm`: Implement `RequestResponseProtocol` (fixes `request()` stub)
- `CommunicationBus`: Initial implementation, bridges to existing EventBus

Phase 1 is zero-breakage — new code only, no existing code modified. CommunicationBus bridges to existing EventBus, two systems coexist.

### Phase 2: Replace Internal Communication

- BrainCore → SelfField: switch to `CommunicationBus.request()`
- Engine → Memory: switch to `CommunicationBus.request()`
- Perception → Engine: switch to `StreamProtocol`
- EventBus subscribers: switch to `CommunicationBus.subscribe()`

### Phase 3: Unify IPC

- `AgentMessage` → `Envelope` migration
- `UnixSocketBackend` → `UnixSocketTransport` wrapper
- `IpcManager` → `TransportRouter` replacement
- Agent inter-communication uses unified `CommunicationBus`

### Phase 4: Agent Kernel Integration

- `AgentProcess` uses `CommunicationBus` for send/receive
- `AgentFork` returns results via `CommunicationBus`
- `SharedScratchpad` based on `StreamProtocol`
- `GlobalTokenPool` broadcasts budget events via PubSub

## 10. Crate Ownership

| Component | Crate | Rationale |
|-----------|-------|-----------|
| `Envelope`, `Transport`, `Protocol` traits | `aletheon-abi` | Pure trait definitions, zero implementations |
| `InProcessTransport` | `aletheon-comm` | Replaces existing KernelEventBus |
| `UnixSocketTransport` | `aletheon-comm` | Wraps existing UnixSocketBackend |
| `TransportRouter` | `aletheon-comm` | Routing logic |
| `RequestResponseProtocol` | `aletheon-comm` | Implements request-response |
| `PubSubProtocol` | `aletheon-comm` | Wraps existing EventBus |
| `StreamProtocol` | `aletheon-comm` | Stream processing |
| `ServiceRegistry` | `aletheon-comm` | Service registration |
| `MessageRouter` | `aletheon-comm` | Routing rules |
| `CommunicationBus` | `aletheon-comm` | Unified entry point |

All in `aletheon-comm`, maintaining the existing "abi defines traits, comm implements" pattern. Other crates only depend on `aletheon-abi` traits, never directly on `aletheon-comm` implementations.

## 11. Existing Mechanism Replacement Map

| Existing Mechanism | Replacement | Notes |
|--------------------|-------------|-------|
| `EventBus` trait (event_bus.rs) | `CommunicationBus.publish()` | PubSub pattern replaces EventBus |
| `EventBus::request()` stub | `CommunicationBus::request()` | Real request-response implementation |
| `KernelEventBus` (kernel_bus.rs) | `InProcessTransport` | Preserves SubscriptionRegistry + EventLog + RoutingPolicy |
| `IpcBackend` trait (ipc_types.rs) | `Transport` trait | Unified interface, UnixSocket impl preserved |
| `IpcManager` (manager.rs) | `TransportRouter` | Auto-selects InProcess vs UnixSocket |
| `AgentMessage` (ipc_types.rs) | `Envelope` | Unified format, eliminates separate IPC message type |
| mpsc channels (perception) | `StreamProtocol` | Perception data uses Stream pattern |
| `HookDispatcher` (hook/) | Preserved, but hook trigger points register via `CommunicationBus` | Hooks become a routing rule action type |
| `DelegateTool` direct call | `CommunicationBus::request()` | Agent delegation uses unified Request-Response |

## 12. Relationship to Multi-Agent Kernel Design

This protocol stack design is a prerequisite for the Multi-Agent Kernel design (`2026-06-14-multi-agent-kernel-design.md`). The kernel primitives map to protocol patterns:

| Kernel Primitive | Protocol Pattern | Transport |
|-----------------|------------------|-----------|
| `send(pid, msg)` | Request-Response | Auto-routed (InProcess or UnixSocket) |
| `recv()` | `register()` on Transport | Point-to-point mailbox |
| `broadcast(event)` | Pub-Sub | EventBus topic |
| `wait(pid)` | Request-Response with long timeout | correlation_id tracking |
| SharedScratchpad | Stream | InProcessTransport |

The Multi-Agent Kernel should be implemented on top of this protocol stack, not as a parallel system.

## 13. Open Questions

1. **Envelope clone cost**: Envelope contains `Payload::Binary(Vec<u8>)` which is expensive to clone. Consider `Arc<Payload>` for mirroring scenarios.

2. **Backward compatibility**: During Phase 1-2, both `EventBus` and `CommunicationBus` coexist. Need a clear deprecation timeline.

3. **Hook integration**: Should hooks be routing rules (as proposed) or remain a separate lifecycle mechanism? Current design makes hooks a RoutingAction variant.

4. **Serialization format**: Currently using bincode for IPC. Should we switch to a more portable format (MessagePack, Protobuf) for cross-language compatibility?

5. **Request timeout defaults**: 30s is proposed. Should different module pairs have different defaults? (e.g., SelfField review: 5s, Tool execution: 120s)

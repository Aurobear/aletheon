// crates/aletheon-comm/src/impl/in_process.rs

//! InProcessTransport — intra-process communication using tokio channels.
//! Analogous to Linux loopback interface.
//!
//! Wraps the existing KernelEventBus for event dispatch,
//! and adds point-to-point mailbox support for Envelope-based communication.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::sync::{broadcast, mpsc};

use crate::ipc::envelope::*;
use crate::events::event::{Event, EventType, Priority};
use crate::ipc::transport::{HealthStatus, Transport, TransportHealth, TransportKind};

use crate::events::event_log::EventLog;
use crate::ipc::bus::kernel_bus::KernelEventBus;
use crate::events::routing_policy::{RouteAction, RoutingPolicy};

/// Priority-aware channel wrapper for envelope delivery.
///
/// Maintains a priority heap alongside the mpsc channel to ensure
/// critical messages are delivered before lower-priority ones.
pub struct PriorityChannel {
    tx: mpsc::Sender<Envelope>,
    /// Priority heap for reordering messages before delivery.
    /// Uses parking_lot::Mutex for non-async locking.
    heap: std::sync::Mutex<std::collections::BinaryHeap<PriorityEnvelope>>,
}

/// Wrapper for Envelope with priority ordering.
#[derive(Debug)]
struct PriorityEnvelope {
    envelope: Envelope,
    /// Sequence number for FIFO within same priority.
    sequence: u64,
}

impl PartialEq for PriorityEnvelope {
    fn eq(&self, other: &Self) -> bool {
        self.envelope.priority == other.envelope.priority && self.sequence == other.sequence
    }
}

impl Eq for PriorityEnvelope {}

impl PartialOrd for PriorityEnvelope {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PriorityEnvelope {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // BinaryHeap is a max-heap, so we reverse to get min-heap behavior
        // Lower priority value = higher precedence (Critical=0, Background=4)
        other
            .envelope
            .priority
            .cmp(&self.envelope.priority)
            .then(other.sequence.cmp(&self.sequence))
    }
}

impl PriorityChannel {
    fn new(buffer: usize) -> (Self, mpsc::Receiver<Envelope>) {
        let (tx, rx) = mpsc::channel(buffer);
        let channel = Self {
            tx,
            heap: std::sync::Mutex::new(std::collections::BinaryHeap::new()),
        };
        (channel, rx)
    }

    /// Send an envelope through the priority channel.
    ///
    /// The envelope is added to the priority heap and then the highest-priority
    /// envelope is sent through the mpsc channel.
    async fn send(&self, envelope: Envelope, sequence: &mut u64) -> Result<()> {
        {
            let mut heap = self.heap.lock().unwrap();
            heap.push(PriorityEnvelope {
                envelope,
                sequence: *sequence,
            });
            *sequence += 1;
        }

        // Send the highest-priority envelope from the heap
        let to_send = {
            let mut heap = self.heap.lock().unwrap();
            heap.pop()
        };

        if let Some(entry) = to_send {
            self.tx
                .send(entry.envelope)
                .await
                .map_err(|_| anyhow::anyhow!("priority channel closed"))?;
        }

        Ok(())
    }
}

/// Transport metrics for health monitoring.
struct TransportMetrics {
    /// Total messages sent successfully.
    messages_sent: std::sync::atomic::AtomicU64,
    /// Total messages received.
    messages_received: std::sync::atomic::AtomicU64,
    /// Total errors encountered.
    errors: std::sync::atomic::AtomicU64,
    /// Total latency in microseconds.
    total_latency_us: std::sync::atomic::AtomicU64,
}

impl TransportMetrics {
    fn new() -> Self {
        Self {
            messages_sent: std::sync::atomic::AtomicU64::new(0),
            messages_received: std::sync::atomic::AtomicU64::new(0),
            errors: std::sync::atomic::AtomicU64::new(0),
            total_latency_us: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn avg_latency_us(&self) -> u64 {
        let sent = self
            .messages_sent
            .load(std::sync::atomic::Ordering::Relaxed);
        if sent == 0 {
            return 0;
        }
        let total = self
            .total_latency_us
            .load(std::sync::atomic::Ordering::Relaxed);
        total / sent
    }
}

/// Intra-process transport — based on tokio channels.
/// Wraps existing KernelEventBus for backward-compatible event dispatch.
pub struct InProcessTransport {
    /// Per-module mailboxes for point-to-point delivery.
    mailboxes: DashMap<ModuleId, PriorityChannel>,

    /// Per-agent mailboxes for point-to-point delivery.
    agent_mailboxes: DashMap<u64, PriorityChannel>,

    /// Per-topic broadcast channels for pub-sub.
    topics: DashMap<String, broadcast::Sender<Envelope>>,

    /// Existing EventBus for backward-compatible event dispatch.
    event_bus: Arc<KernelEventBus>,

    /// Event log (shared with KernelEventBus).
    event_log: Arc<RwLock<EventLog>>,

    /// Global sequence counter for priority ordering.
    sequence: std::sync::atomic::AtomicU64,

    /// Transport metrics for health monitoring.
    metrics: TransportMetrics,
}

impl InProcessTransport {
    /// Create a new InProcessTransport wrapping an existing KernelEventBus.
    pub fn new(event_bus: Arc<KernelEventBus>) -> Self {
        let event_log = event_bus.event_log();
        Self {
            mailboxes: DashMap::new(),
            agent_mailboxes: DashMap::new(),
            topics: DashMap::new(),
            event_bus,
            event_log,
            sequence: std::sync::atomic::AtomicU64::new(0),
            metrics: TransportMetrics::new(),
        }
    }

    /// Create a new InProcessTransport with cross-process transport bridging.
    ///
    /// The provided transport is passed to KernelEventBus for event bridging.
    pub fn with_transport(
        event_bus: Arc<KernelEventBus>,
        transport: Arc<dyn Transport>,
    ) -> Self {
        let event_log = event_bus.event_log();
        // Create a new KernelEventBus with the transport for bridging
        let bridged_bus = Arc::new(KernelEventBus::with_transport(
            event_log.read().capacity(),
            transport,
        ));
        Self {
            mailboxes: DashMap::new(),
            agent_mailboxes: DashMap::new(),
            topics: DashMap::new(),
            event_bus: bridged_bus,
            event_log,
            sequence: std::sync::atomic::AtomicU64::new(0),
            metrics: TransportMetrics::new(),
        }
    }

    /// Register a module mailbox for point-to-point delivery.
    /// Returns a receiver for envelopes addressed to this module.
    pub fn register_module(&self, module: ModuleId, buffer: usize) -> mpsc::Receiver<Envelope> {
        let (channel, rx) = PriorityChannel::new(buffer);
        self.mailboxes.insert(module, channel);
        rx
    }

    /// Register an agent mailbox for point-to-point delivery.
    /// Returns a receiver for envelopes addressed to this agent.
    pub async fn register_agent(&self, pid: u64, buffer: usize) -> mpsc::Receiver<Envelope> {
        let (channel, rx) = PriorityChannel::new(buffer);
        self.agent_mailboxes.insert(pid, channel);
        rx
    }

    /// Unregister an agent mailbox.
    pub async fn unregister_agent(&self, pid: &u64) {
        self.agent_mailboxes.remove(pid);
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

    /// Get a reference to the underlying EventBus.
    pub fn event_bus(&self) -> &Arc<KernelEventBus> {
        &self.event_bus
    }

    /// Deliver an envelope to its target.
    async fn deliver(&self, envelope: Envelope) -> Result<()> {
        let start = std::time::Instant::now();

        // Record in event log
        {
            let mut log = self.event_log.write();
            log.record(&OwnedEnvelopeEventAdapter::new(&envelope));
        }

        let mut seq = self
            .sequence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let result = match &envelope.target {
            Target::Module(module) => {
                if let Some(channel) = self.mailboxes.get(module) {
                    channel
                        .send(envelope, &mut seq)
                        .await
                        .map_err(|_| anyhow::anyhow!("module mailbox closed"))
                } else {
                    tracing::warn!("no mailbox registered for module {:?}", module);
                    Ok(())
                }
            }
            Target::Agent(pid) => {
                if let Some(channel) = self.agent_mailboxes.get(pid) {
                    channel
                        .send(envelope, &mut seq)
                        .await
                        .map_err(|_| anyhow::anyhow!("agent mailbox closed"))
                } else {
                    tracing::warn!("no mailbox registered for agent pid {}", pid);
                    Ok(())
                }
            }
            Target::Topic(topic) => {
                if let Some(tx) = self.topics.get(topic) {
                    let _ = tx.send(envelope); // Ignore if no receivers
                }
                Ok(())
            }
            Target::Broadcast => {
                for entry in self.mailboxes.iter() {
                    let _ = entry.value().send(envelope.clone(), &mut seq).await;
                }
                Ok(())
            }
        };

        // Track metrics
        let latency = start.elapsed().as_micros() as u64;
        self.metrics
            .messages_sent
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.metrics
            .total_latency_us
            .fetch_add(latency, std::sync::atomic::Ordering::Relaxed);

        if result.is_err() {
            self.metrics
                .errors
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        result
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
            Target::Agent(pid) => self.agent_mailboxes.contains_key(pid),
            Target::Topic(t) => self.topics.contains_key(t),
            Target::Broadcast => true,
        }
    }

    async fn send(&self, envelope: Envelope) -> Result<()> {
        // Check TTL
        if envelope.is_expired() {
            anyhow::bail!("envelope expired");
        }

        // Apply routing policy for Critical priority
        if envelope.priority == Priority::Critical {
            match RoutingPolicy::evaluate(&EventType::UserIntent, &envelope.priority) {
                RouteAction::RequireSelfFieldReview => {
                    tracing::warn!(
                        "Critical envelope requires SelfField review (Phase 1: delivering anyway)"
                    );
                }
                RouteAction::FastPath => {}
            }
        }

        self.deliver(envelope).await
    }

    fn health(&self) -> TransportHealth {
        let messages_sent = self
            .metrics
            .messages_sent
            .load(std::sync::atomic::Ordering::Relaxed);
        let errors = self
            .metrics
            .errors
            .load(std::sync::atomic::Ordering::Relaxed);
        let error_rate = if messages_sent > 0 {
            errors as f64 / messages_sent as f64
        } else {
            0.0
        };

        TransportHealth {
            status: if error_rate < 0.1 {
                HealthStatus::Healthy
            } else if error_rate < 0.5 {
                HealthStatus::Degraded
            } else {
                HealthStatus::Unhealthy
            },
            latency_ms: self.metrics.avg_latency_us() / 1000, // Convert to ms
            queue_depth: 0, // Not tracked for in-process
            error_rate,
        }
    }
}

/// Owned adapter to implement Event trait for Envelope (for event log recording).
struct OwnedEnvelopeEventAdapter {
    id: u64,
    priority: Priority,
    source: String,
    pattern: String,
    target: String,
    json: serde_json::Value,
}

impl OwnedEnvelopeEventAdapter {
    fn new(envelope: &Envelope) -> Self {
        let source = match &envelope.source {
            Endpoint::Module(m) => match m {
                ModuleId::Brain => "brain".to_string(),
                ModuleId::SelfField => "self".to_string(),
                ModuleId::Memory => "memory".to_string(),
                ModuleId::Body => "body".to_string(),
                ModuleId::Meta => "meta".to_string(),
                ModuleId::Runtime => "runtime".to_string(),
                ModuleId::Perception => "perception".to_string(),
            },
            Endpoint::Agent(_) => "agent".to_string(),
            Endpoint::System => "system".to_string(),
        };
        Self {
            id: envelope.id,
            priority: envelope.priority,
            source,
            pattern: format!("{:?}", envelope.pattern),
            target: format!("{:?}", envelope.target),
            json: serde_json::to_value(envelope).unwrap_or_default(),
        }
    }
}

impl Event for OwnedEnvelopeEventAdapter {
    fn event_type(&self) -> EventType {
        EventType::UserIntent
    }

    fn priority(&self) -> Priority {
        self.priority
    }

    fn source(&self) -> &str {
        &self.source
    }

    fn payload(&self) -> &dyn std::any::Any {
        &()
    }

    fn summary(&self) -> String {
        format!("Envelope {} {} {}", self.id, self.pattern, self.target)
    }

    fn to_json(&self) -> serde_json::Value {
        self.json.clone()
    }
}

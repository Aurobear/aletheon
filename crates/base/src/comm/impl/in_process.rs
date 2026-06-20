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

use crate::envelope::*;
use crate::event::{Event, EventType, Priority};
use crate::transport::{HealthStatus, Transport, TransportHealth, TransportKind};

use crate::comm::r#impl::event_log::EventLog;
use crate::comm::r#impl::kernel_bus::KernelEventBus;
use crate::comm::r#impl::routing_policy::{RouteAction, RoutingPolicy};

/// Intra-process transport — based on tokio channels.
/// Wraps existing KernelEventBus for backward-compatible event dispatch.
pub struct InProcessTransport {
    /// Per-module mailboxes for point-to-point delivery.
    mailboxes: DashMap<ModuleId, mpsc::Sender<Envelope>>,

    /// Per-agent mailboxes for point-to-point delivery.
    agent_mailboxes: DashMap<u64, mpsc::Sender<Envelope>>,

    /// Per-topic broadcast channels for pub-sub.
    topics: DashMap<String, broadcast::Sender<Envelope>>,

    /// Existing EventBus for backward-compatible event dispatch.
    event_bus: Arc<KernelEventBus>,

    /// Event log (shared with KernelEventBus).
    event_log: Arc<RwLock<EventLog>>,
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
        }
    }

    /// Register a module mailbox for point-to-point delivery.
    /// Returns a receiver for envelopes addressed to this module.
    pub fn register_module(&self, module: ModuleId, buffer: usize) -> mpsc::Receiver<Envelope> {
        let (tx, rx) = mpsc::channel(buffer);
        self.mailboxes.insert(module, tx);
        rx
    }

    /// Register an agent mailbox for point-to-point delivery.
    /// Returns a receiver for envelopes addressed to this agent.
    pub async fn register_agent(&self, pid: u64, buffer: usize) -> mpsc::Receiver<Envelope> {
        let (tx, rx) = mpsc::channel(buffer);
        self.agent_mailboxes.insert(pid, tx);
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
        // Record in event log
        {
            let mut log = self.event_log.write();
            log.record(&OwnedEnvelopeEventAdapter::new(&envelope));
        }

        match &envelope.target {
            Target::Module(module) => {
                if let Some(tx) = self.mailboxes.get(module) {
                    tx.send(envelope)
                        .await
                        .map_err(|_| anyhow::anyhow!("module mailbox closed"))?;
                } else {
                    tracing::warn!("no mailbox registered for module {:?}", module);
                }
            }
            Target::Agent(pid) => {
                if let Some(tx) = self.agent_mailboxes.get(pid) {
                    tx.send(envelope)
                        .await
                        .map_err(|_| anyhow::anyhow!("agent mailbox closed"))?;
                } else {
                    tracing::warn!("no mailbox registered for agent pid {}", pid);
                }
            }
            Target::Topic(topic) => {
                if let Some(tx) = self.topics.get(topic) {
                    let _ = tx.send(envelope); // Ignore if no receivers
                }
            }
            Target::Broadcast => {
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
        TransportHealth {
            status: HealthStatus::Healthy,
            latency_ms: 0,
            queue_depth: 0,
            error_rate: 0.0,
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

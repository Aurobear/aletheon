// crates/aletheon-comm/src/impl/communication_bus.rs

//! CommunicationBus — unified entry point for all communication.
//! Replaces direct trait calls and Arc<Mutex> references.
//!
//! Phase 1: Wraps InProcessTransport + RequestResponseProtocol + PubSubProtocol.
//! Future phases will add TransportRouter for automatic cross-process routing.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{broadcast, mpsc, Mutex};

use base::envelope::*;
use base::event::Event;
use base::event_bus::EventBus;
use base::protocol::Protocol;
use base::transport::Transport;

use crate::r#impl::debug_bus::DebugBusHook;
use crate::r#impl::in_process::InProcessTransport;
use crate::r#impl::kernel_bus::KernelEventBus;
use crate::r#impl::pubsub::PubSubProtocol;
use crate::r#impl::request_response::RequestResponseProtocol;

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

    /// Cross-process transports for `Target::Agent(pid)` routing.
    /// Each transport is tested via `can_reach()` before sending.
    transports: Vec<Arc<dyn Transport>>,

    /// Request-response protocol.
    request_response: Arc<RequestResponseProtocol>,

    /// Pub-sub protocol.
    pubsub: Arc<PubSubProtocol>,

    /// Optional debug hook — observes published events.
    debug_hook: Option<Arc<Mutex<DebugBusHook>>>,
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
            transports: Vec::new(),
            request_response,
            pubsub,
            debug_hook: None,
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
            transports: Vec::new(),
            request_response,
            pubsub,
            debug_hook: None,
        }
    }

    /// Add a cross-process transport to the bus.
    /// Transports are tried in order for `Target::Agent(pid)` routing.
    pub fn add_transport(&mut self, transport: Arc<dyn Transport>) {
        self.transports.push(transport);
    }

    /// Attach a debug hook to observe published events.
    ///
    /// The hook is called on every `publish()` and `publish_event()` invocation,
    /// forwarding matching events to registered sinks and optionally recording
    /// them to a bag file.
    pub fn with_debug_hook(mut self, hook: DebugBusHook) -> Self {
        self.debug_hook = Some(Arc::new(Mutex::new(hook)));
        self
    }

    // -- Module-level API --

    /// Register a module mailbox for point-to-point delivery.
    pub fn register_module(
        &self,
        module: ModuleId,
        buffer: Option<usize>,
    ) -> mpsc::Receiver<Envelope> {
        self.in_process
            .register_module(module, buffer.unwrap_or(64))
    }

    /// Register an agent mailbox for point-to-point delivery.
    pub async fn register_agent(
        &self,
        pid: u64,
        buffer: Option<usize>,
    ) -> mpsc::Receiver<Envelope> {
        self.in_process
            .register_agent(pid, buffer.unwrap_or(64))
            .await
    }

    /// Unregister an agent mailbox.
    pub async fn unregister_agent(&self, pid: &u64) {
        self.in_process.unregister_agent(pid).await
    }

    /// Subscribe to a topic for pub-sub delivery.
    pub fn subscribe_topic(
        &self,
        topic: &str,
        buffer: Option<usize>,
    ) -> broadcast::Receiver<Envelope> {
        self.in_process
            .subscribe_topic(topic, buffer.unwrap_or(256))
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
    /// For Response envelopes, the request-response protocol gets first crack
    /// at completing any pending correlated request before transport delivery.
    ///
    /// Routing:
    /// - `Target::Module(_)` / `Target::Topic(_)` / `Target::Broadcast` → InProcessTransport fast path.
    /// - `Target::Agent(pid)` → try cross-process transports that `can_reach()` the target,
    ///   then fall back to InProcessTransport.
    pub async fn send(&self, envelope: Envelope) -> Result<()> {
        // Route Response envelopes through the protocol layer first.
        // If a pending request matches the correlation_id, complete it and return.
        // Otherwise fall through to transport delivery.
        if let Pattern::Response = &envelope.pattern {
            if self.request_response.handle_response(&envelope) {
                return Ok(());
            }
        }

        // For Agent targets, try cross-process transports first.
        if let Target::Agent(_) = &envelope.target {
            for transport in &self.transports {
                if transport.can_reach(&envelope.target) {
                    return transport.send(envelope).await;
                }
            }
        }

        // Fallback: InProcessTransport (handles Module, Topic, Broadcast, and
        // Agent when no cross-process transport can reach).
        self.in_process.send(envelope).await
    }

    /// Handle an incoming response (complete a pending request).
    pub fn handle_response(&self, response: &Envelope) -> bool {
        self.request_response.handle_response(response)
    }

    // -- Backward Compatibility --

    /// Get a reference to the underlying EventBus.
    /// Used during migration to bridge old EventBus subscribers.
    pub fn event_bus(&self) -> &Arc<KernelEventBus> {
        self.in_process.event_bus()
    }

    /// Publish an Event via the underlying EventBus.
    /// Bridge method for backward compatibility with existing Event subscribers.
    ///
    /// If a debug hook is attached, a corresponding `DebugEvent` is forwarded
    /// to the hook before the EventBus dispatch.
    pub async fn publish_event(&self, event: Box<dyn Event>) -> Result<()> {
        // Notify debug hook (best-effort, non-blocking for the bus).
        if let Some(ref hook) = self.debug_hook {
            let debug_event = base::debug::DebugEvent {
                ts: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                tracepoint: format!("{:?}", event.event_type()),
                module: event.source().to_string(),
                level: base::debug::DebugLevel::Info,
                data: event.to_json(),
                session_id: None,
                agent_id: None,
            };
            hook.lock().await.on_event(&debug_event).await;
        }
        self.event_bus().publish(event).await
    }

    // -- Diagnostics --

    /// Number of pending request-response correlations.
    pub fn pending_requests(&self) -> usize {
        self.request_response.pending_count()
    }

    /// Health status of the underlying transport.
    pub fn health(&self) -> base::transport::TransportHealth {
        self.in_process.health()
    }
}

impl Default for CommunicationBus {
    fn default() -> Self {
        Self::new()
    }
}

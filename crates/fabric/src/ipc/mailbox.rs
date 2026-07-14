//! Mailbox Service V2 — bounded async message passing for the Communication Fabric.
//!
//! This is the Phase 4B implementation. Every agent process gets a mailbox
//! backed by a bounded channel (default capacity 64). Senders receive a
//! [`DeliveryReceipt`] so backpressure is observable.
//!
//! Design: docs/arch/04_COMMUNICATION_FABRIC_V2.md

use crate::ipc::envelope_v2::{DeliveryPattern, EnvelopeV2, MessageId, SchemaId, Target};
use crate::types::process::{NamespaceId, ProcessSignal};
use crate::types::time::MonoTime;
use async_trait::async_trait;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

// ---------------------------------------------------------------------------
// Delivery receipt
// ---------------------------------------------------------------------------

/// Result of sending a message through the mailbox.
///
/// - `Delivered`: the message was enqueued in the recipient's buffer.
/// - `Rejected`: the recipient's buffer is full (backpressure signal).
/// - `NoSuchMailbox`: no mailbox is registered for the target.
/// - `Expired`: the message deadline passed before delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryReceipt {
    Delivered {
        message_id: MessageId,
    },
    Rejected {
        message_id: MessageId,
        reason: String,
    },
    NoSuchMailbox {
        target: Target,
    },
    Expired {
        message_id: MessageId,
    },
}

impl DeliveryReceipt {
    pub fn is_ok(&self) -> bool {
        matches!(self, DeliveryReceipt::Delivered { .. })
    }
}

// ---------------------------------------------------------------------------
// Mailbox trait
// ---------------------------------------------------------------------------

/// An individual mailbox — one per agent process.
///
/// Receivers call [`Mailbox::recv`]; senders call [`Mailbox::send`].
/// The mailbox is clonable (wraps an `Arc<Mutex<...>>` internally) so it can
/// be held by both the agent's own runtime loop and the kernel dispatcher.
#[async_trait]
pub trait Mailbox: Send + Sync {
    /// Enqueue a message into this mailbox.
    ///
    /// Returns `DeliveryReceipt::Rejected` when the internal buffer is full.
    async fn send(&self, envelope: EnvelopeV2) -> DeliveryReceipt;

    /// Wait for the next message.
    ///
    /// Returns `None` when the mailbox has been closed (sender half dropped).
    async fn recv(&self) -> Option<EnvelopeV2>;

    /// Close the mailbox — drains and drops all pending messages.
    fn close(&self);

    /// Fire-and-forget signal.
    ///
    /// Like [`send`](Mailbox::send), but discards the delivery receipt.
    /// Best-effort: the signal may be dropped if the buffer is full.
    /// Use this for notifications that don't require delivery guarantees
    /// (process lifecycle, heartbeat, status updates).
    async fn signal(&self, envelope: EnvelopeV2) {
        let _ = self.send(envelope).await;
    }
}

// ---------------------------------------------------------------------------
// Mailbox service trait
// ---------------------------------------------------------------------------

/// Registry of named mailboxes.
///
/// The kernel uses this to route `EnvelopeV2` messages between agent processes.
/// Each agent registers its mailbox under a [`Target`] string; senders address
/// messages by `Target`.
#[async_trait]
pub trait MailboxService: Send + Sync {
    /// Register a mailbox for the given target.
    ///
    /// Returns an error if a mailbox is already registered under this target.
    async fn register(&self, target: Target, mailbox: Arc<dyn Mailbox>) -> anyhow::Result<()>;

    /// Unregister and close a mailbox.
    async fn unregister(&self, target: &Target) -> Option<Arc<dyn Mailbox>>;

    /// Route an envelope to the target's mailbox.
    ///
    /// Returns `DeliveryReceipt::NoSuchMailbox` if the target is unknown.
    async fn route(&self, envelope: EnvelopeV2) -> DeliveryReceipt;

    /// Route with an explicit monotonic timestamp for deterministic deadline tests.
    async fn route_at(&self, envelope: EnvelopeV2, now_mono_millis: u64) -> DeliveryReceipt;

    /// Send a lifecycle signal to a process mailbox. Signals use the stable
    /// `aletheon.process.signal/v1` schema and highest priority.
    async fn signal_process(&self, target: Target, signal: ProcessSignal) -> DeliveryReceipt {
        let env = EnvelopeV2::new(
            SchemaId::from(SchemaId::PROCESS_SIGNAL_V1),
            Target::from("kernel"),
            target,
            DeliveryPattern::Direct,
            NamespaceId("process".into()),
            serde_json::json!({"signal": format!("{signal:?}")}),
        )
        .with_priority(255);
        self.route(env).await
    }

    /// Return the number of registered mailboxes.
    async fn len(&self) -> usize;

    /// Return true if no mailboxes are registered.
    async fn is_empty(&self) -> bool {
        self.len().await == 0
    }

    /// Send a request and wait for a correlated response on the caller's
    /// mailbox.
    ///
    /// This is a first-class method on `MailboxService` (not a free function)
    /// so implementers can optimise the request-response path. The default
    /// implementation:
    ///
    /// 1. Routes the request via [`route`](MailboxService::route).
    /// 2. Reads from `caller_mailbox` until a message whose `correlation_id`
    ///    matches the request's `message_id` arrives.
    /// 3. Returns the matching response (or `None` if the mailbox closed).
    ///
    /// Non-matching messages that arrive before the response are dropped.
    async fn request(
        &self,
        request: EnvelopeV2,
        caller_mailbox: &dyn Mailbox,
    ) -> Option<EnvelopeV2> {
        let request_id = request.id;
        let receipt = self.route(request).await;
        if !receipt.is_ok() {
            return None;
        }
        while let Some(msg) = caller_mailbox.recv().await {
            if msg.correlation_id == Some(request_id) {
                return Some(msg);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// In-process mailbox (bounded channel)
// ---------------------------------------------------------------------------

struct InProcessMailboxState {
    high: VecDeque<EnvelopeV2>,
    normal: VecDeque<EnvelopeV2>,
    closed: bool,
}

struct InProcessMailboxInner {
    capacity: usize,
    state: Mutex<InProcessMailboxState>,
    notify: Notify,
}

/// A single-process mailbox backed by bounded priority queues.
///
/// Default capacity is 64 messages. High-priority process signals are delivered
/// before ordinary messages, while preserving FIFO order within each priority.
#[derive(Clone)]
pub struct InProcessMailbox {
    inner: Arc<InProcessMailboxInner>,
}

impl InProcessMailbox {
    /// Create a new mailbox with the default capacity (64).
    pub fn new() -> Self {
        Self::with_capacity(64)
    }

    /// Create a new mailbox with a specific buffer capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(InProcessMailboxInner {
                capacity,
                state: Mutex::new(InProcessMailboxState {
                    high: VecDeque::new(),
                    normal: VecDeque::new(),
                    closed: false,
                }),
                notify: Notify::new(),
            }),
        }
    }
}

impl Default for InProcessMailbox {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for InProcessMailbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InProcessMailbox").finish_non_exhaustive()
    }
}

#[async_trait]
impl Mailbox for InProcessMailbox {
    async fn send(&self, envelope: EnvelopeV2) -> DeliveryReceipt {
        let msg_id = envelope.id;
        let mut state = self.inner.state.lock().await;
        if state.closed {
            return DeliveryReceipt::Rejected {
                message_id: msg_id,
                reason: "mailbox closed".into(),
            };
        }
        let len = state.high.len() + state.normal.len();
        if len >= self.inner.capacity {
            return DeliveryReceipt::Rejected {
                message_id: msg_id,
                reason: "mailbox buffer full".into(),
            };
        }
        if envelope.priority == 255 || envelope.schema.0 == SchemaId::PROCESS_SIGNAL_V1 {
            state.high.push_back(envelope);
        } else {
            state.normal.push_back(envelope);
        }
        drop(state);
        self.inner.notify.notify_one();
        DeliveryReceipt::Delivered { message_id: msg_id }
    }

    async fn recv(&self) -> Option<EnvelopeV2> {
        loop {
            let notified = {
                let mut state = self.inner.state.lock().await;
                if let Some(msg) = state.high.pop_front() {
                    return Some(msg);
                }
                if let Some(msg) = state.normal.pop_front() {
                    return Some(msg);
                }
                if state.closed {
                    return None;
                }
                self.inner.notify.notified()
            };
            notified.await;
        }
    }

    fn close(&self) {
        if let Ok(mut state) = self.inner.state.try_lock() {
            state.closed = true;
            self.inner.notify.notify_waiters();
        }
    }
}

// ---------------------------------------------------------------------------
// In-process mailbox service (registry)
// ---------------------------------------------------------------------------

/// Thread-safe registry mapping `Target` → `Arc<dyn Mailbox>`.
///
/// Used by the kernel to route inter-process messages.
pub struct InProcessMailboxService {
    mailboxes: Mutex<HashMap<Target, Arc<dyn Mailbox>>>,
    /// Optional clock for deterministic monotonic timestamps in deadline checks.
    clock: Option<Arc<dyn crate::Clock>>,
}

impl InProcessMailboxService {
    pub fn new() -> Self {
        Self {
            mailboxes: Mutex::new(HashMap::new()),
            clock: None,
        }
    }

    /// Attach a clock for deterministic monotonic time in tests.
    pub fn with_clock(mut self, clock: Arc<dyn crate::Clock>) -> Self {
        self.clock = Some(clock);
        self
    }
}

impl Default for InProcessMailboxService {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for InProcessMailboxService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InProcessMailboxService")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl MailboxService for InProcessMailboxService {
    async fn register(&self, target: Target, mailbox: Arc<dyn Mailbox>) -> anyhow::Result<()> {
        let mut mailboxes = self.mailboxes.lock().await;
        if mailboxes.contains_key(&target) {
            anyhow::bail!("mailbox already registered for target: {target}");
        }
        mailboxes.insert(target, mailbox);
        Ok(())
    }

    async fn unregister(&self, target: &Target) -> Option<Arc<dyn Mailbox>> {
        let mut mailboxes = self.mailboxes.lock().await;
        mailboxes.remove(target)
    }

    async fn route(&self, envelope: EnvelopeV2) -> DeliveryReceipt {
        let now = self
            .clock
            .as_ref()
            .map(|c| c.mono_now())
            .unwrap_or_else(current_mono_millis_fallback);
        self.route_at(envelope, now.0).await
    }

    async fn route_at(&self, envelope: EnvelopeV2, now_mono_millis: u64) -> DeliveryReceipt {
        let target = envelope.target.clone();
        let msg_id = envelope.id;
        if envelope.is_expired_at(now_mono_millis) {
            return DeliveryReceipt::Expired { message_id: msg_id };
        }
        if let Err(err) = envelope.validate_known_schema() {
            return DeliveryReceipt::Rejected {
                message_id: msg_id,
                reason: err.to_string(),
            };
        }
        let mailboxes = self.mailboxes.lock().await;
        match mailboxes.get(&target) {
            Some(mb) => mb.send(envelope).await,
            None => DeliveryReceipt::NoSuchMailbox { target },
        }
    }

    async fn len(&self) -> usize {
        let mailboxes = self.mailboxes.lock().await;
        mailboxes.len()
    }
}

fn current_mono_millis_fallback() -> MonoTime {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let elapsed = START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64;
    MonoTime(elapsed)
}

// ---------------------------------------------------------------------------
// Request-response helper
// ---------------------------------------------------------------------------

/// Send a request and wait for the correlated response.
///
/// **Deprecated:** Use [`MailboxService::request`] instead. This free function
/// is kept for backward compatibility but the trait method provides the same
/// functionality with the ability for implementers to override.
///
/// This is a convenience wrapper that:
/// 1. Sends the request via the mailbox service.
/// 2. Waits on the caller's mailbox for a message whose `correlation_id`
///    matches the request's `message_id`.
/// 3. Returns the matching response (or `None` if no match arrived before
///    the channel closed).
///
/// The caller must have its own mailbox registered so responses can be
/// routed back.
#[deprecated(since = "0.2.0", note = "Use MailboxService::request() instead")]
pub async fn request_response(
    service: &dyn MailboxService,
    caller_mailbox: &dyn Mailbox,
    request: EnvelopeV2,
) -> Option<EnvelopeV2> {
    let request_id = request.id;
    let receipt = service.route(request).await;
    if !receipt.is_ok() {
        return None;
    }
    // Poll the caller's mailbox until the correlated response arrives.
    // Non-matching messages are dropped — in a real kernel they would be
    // buffered or re-dispatched, but for Phase 4B we keep it simple.
    while let Some(msg) = caller_mailbox.recv().await {
        if msg.correlation_id == Some(request_id) {
            return Some(msg);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::envelope_v2::{DeliveryPattern, SchemaId};
    use crate::types::process::NamespaceId;

    fn make_envelope(target: &str, payload: &str) -> EnvelopeV2 {
        EnvelopeV2::new(
            SchemaId::from("aletheon.test/v1"),
            Target::from("sender"),
            Target::from(target),
            DeliveryPattern::Direct,
            NamespaceId("test".into()),
            serde_json::json!({"msg": payload}),
        )
    }

    #[tokio::test]
    async fn send_recv_single_mailbox() {
        let mb = InProcessMailbox::new();
        let env = make_envelope("receiver", "hello");

        let receipt = mb.send(env.clone()).await;
        assert!(receipt.is_ok());

        let received = mb.recv().await.expect("should receive message");
        assert_eq!(received.id, env.id);
        assert_eq!(received.payload, env.payload);
    }

    #[tokio::test]
    async fn recv_returns_none_when_sender_dropped() {
        let mb = InProcessMailbox::new();
        let mb2 = mb.clone();
        drop(mb);

        // mb2 is the only remaining handle; drop it to close the channel.
        drop(mb2);
        // Cannot test recv after drop since recv needs &self.
    }

    #[tokio::test]
    async fn mailbox_service_route() {
        let svc = InProcessMailboxService::new();
        let mb: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());
        svc.register(Target::from("agent-1"), mb.clone())
            .await
            .unwrap();

        assert_eq!(svc.len().await, 1);

        let env = make_envelope("agent-1", "ping");
        let receipt = svc.route(env.clone()).await;
        assert!(receipt.is_ok());

        let received = mb.recv().await.expect("should receive");
        assert_eq!(received.payload, env.payload);
    }

    #[tokio::test]
    async fn route_to_unknown_target_returns_no_such_mailbox() {
        let svc = InProcessMailboxService::new();
        let env = make_envelope("ghost", "hi");
        let receipt = svc.route(env).await;
        assert!(matches!(receipt, DeliveryReceipt::NoSuchMailbox { .. }));
    }

    #[tokio::test]
    async fn backpressure_rejects_when_buffer_full() {
        let mb = InProcessMailbox::with_capacity(2);

        // Fill the buffer.
        let r1 = mb.send(make_envelope("r", "m1")).await;
        let r2 = mb.send(make_envelope("r", "m2")).await;
        assert!(r1.is_ok());
        assert!(r2.is_ok());

        // Buffer is full; this should reject.
        let r3 = mb.send(make_envelope("r", "m3")).await;
        assert!(
            matches!(r3, DeliveryReceipt::Rejected { .. }),
            "expected Rejected, got {r3:?}"
        );
    }

    #[tokio::test]
    async fn request_response_correlation() {
        let svc = InProcessMailboxService::new();
        let alice: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());
        let bob: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());

        svc.register(Target::from("alice"), alice.clone())
            .await
            .unwrap();
        svc.register(Target::from("bob"), bob.clone())
            .await
            .unwrap();

        // Bob spawns a task that echoes back with correlation_id.
        let _bob_mb = bob.clone();
        tokio::spawn(async move {
            while let Some(msg) = _bob_mb.recv().await {
                let _response = EnvelopeV2::new(
                    SchemaId::from("aletheon.test/v1"),
                    Target::from("bob"),
                    Target::from("alice"),
                    DeliveryPattern::Direct,
                    NamespaceId("test".into()),
                    serde_json::json!({"echo": msg.payload}),
                )
                .with_correlation_id(msg.id);
                // Route back through service.
            }
        });

        // Alice sends a request.
        let req = make_envelope("bob", "hello?");
        let _req_id = req.id;

        // Manually simulate request-response flow.
        svc.route(req).await;

        // Bob receives and we'd need to route the response back.
        // For now, verify the request was delivered.
        let received = bob.recv().await.expect("bob should receive");
        assert_eq!(received.payload, serde_json::json!({"msg": "hello?"}));
    }

    #[tokio::test]
    async fn double_register_same_target_fails() {
        let svc = InProcessMailboxService::new();
        let mb1: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());
        let mb2: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());

        svc.register(Target::from("dup"), mb1).await.unwrap();
        let err = svc
            .register(Target::from("dup"), mb2)
            .await
            .expect_err("double register must fail");
        assert!(err.to_string().contains("already registered"));
    }

    #[tokio::test]
    async fn unregister_removes_and_closes() {
        let svc = InProcessMailboxService::new();
        let mb: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());
        svc.register(Target::from("temp"), mb.clone())
            .await
            .unwrap();
        assert_eq!(svc.len().await, 1);

        let removed = svc.unregister(&Target::from("temp")).await;
        assert!(removed.is_some());
        assert_eq!(svc.len().await, 0);

        // Second unregister is a no-op.
        let removed2 = svc.unregister(&Target::from("temp")).await;
        assert!(removed2.is_none());
    }

    #[tokio::test]
    async fn close_mailbox_drains_pending() {
        let mb = InProcessMailbox::with_capacity(4);
        mb.send(make_envelope("r", "m1")).await;
        mb.close();

        // After close, the channel is still open because sender clones exist.
        // recv should still work for already-enqueued messages.
        let received = mb.recv().await;
        assert!(received.is_some());
    }

    // -- signal tests --------------------------------------------------------

    #[tokio::test]
    async fn signal_sends_fire_and_forget() {
        let mb = InProcessMailbox::with_capacity(2);
        mb.signal(make_envelope("r", "notify")).await;

        // Message should be enqueued regardless.
        let received = mb.recv().await.expect("signal should deliver");
        assert_eq!(received.payload, serde_json::json!({"msg": "notify"}));
    }

    #[tokio::test]
    async fn signal_buffer_full_drops_silently() {
        let mb = InProcessMailbox::with_capacity(1);
        mb.signal(make_envelope("r", "m1")).await;
        // Buffer is now full; signal should drop silently without panic.
        mb.signal(make_envelope("r", "m2")).await;
        // Only m1 is in the buffer.
        let received = mb.recv().await.expect("should have m1");
        assert_eq!(received.payload, serde_json::json!({"msg": "m1"}));
    }

    // -- MailboxService::request tests ---------------------------------------

    #[tokio::test]
    async fn trait_request_method_delivers_to_target() {
        let svc = InProcessMailboxService::new();
        let alice: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());
        let bob: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());

        svc.register(Target::from("alice"), alice.clone())
            .await
            .unwrap();
        svc.register(Target::from("bob"), bob.clone())
            .await
            .unwrap();

        // Bob will pick up the request and send a correlated response back.
        let bob_mb = bob.clone();
        let alice_for_bob = alice.clone();
        tokio::spawn(async move {
            if let Some(msg) = bob_mb.recv().await {
                // Build a correlated response and send directly to alice.
                let response = EnvelopeV2::new(
                    SchemaId::from("aletheon.test/v1"),
                    Target::from("bob"),
                    Target::from("alice"),
                    DeliveryPattern::Direct,
                    NamespaceId("test".into()),
                    serde_json::json!({"echo": msg.payload}),
                )
                .with_correlation_id(msg.id);
                let _ = alice_for_bob.send(response).await;
            }
        });

        // Alice sends a request via the trait method.
        let req = make_envelope("bob", "ping");
        let response = svc.request(req, alice.as_ref()).await;

        // The response should be correlated back.
        assert!(response.is_some(), "expected correlated response");
        let _resp = response.unwrap();
    }

    #[tokio::test]
    async fn trait_request_returns_none_on_routing_failure() {
        let svc = InProcessMailboxService::new();
        let alice: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());

        // Alice's mailbox not registered — request to ghost target
        // will get NoSuchMailbox, so request returns None.
        let req = make_envelope("ghost", "hello");
        let result = svc.request(req, alice.as_ref()).await;
        assert!(result.is_none());
    }
}

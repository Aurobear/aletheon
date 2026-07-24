//! Gmail event-sourced ingest as a first-class channel-registry capability.
//!
//! Gmail is event-sourced (`ExternalEvent::MailReceived` → sender-verify →
//! classify → draft Goal), not a duplex chat channel. This handler is a thin
//! wrapper around the existing [`GmailGoalEventIngress::ingest`] pipeline —
//! it delegates entirely and does not reimplement any of Gmail's own stores,
//! `(account_id, message_id)` idempotency, or `GmailSenderPolicy`
//! deny-by-default verification.
//!
//! Registered under [`IntentKind::ExternalEventIngest`] in an
//! `EventCapabilityRegistry`, and invoked directly by
//! `google/event_dispatcher.rs` on `MailReceived` — never through
//! `ChannelDispatcher::process` (no inbox dedup,
//! no `complete_inbound`, no `transport.send`). Any Telegram notification for
//! a matching subscription is already persisted by
//! `DurableGoogleNotificationSink` independently of this handler, so
//! `handle` always returns `Ok(vec![])` on success.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::ExternalEventEnvelope;
use tokio_util::sync::CancellationToken;

use gateway::effect::OutboundEffect;
use gateway::registry::{EventCapabilityHandler, IntentKind};

use super::super::gmail::GmailGoalEventIngress;

/// Thin [`EventCapabilityHandler`] wrapper around [`GmailGoalEventIngress`].
pub struct ExternalEventIngestHandler {
    ingress: Arc<GmailGoalEventIngress>,
}

impl ExternalEventIngestHandler {
    pub fn new(ingress: Arc<GmailGoalEventIngress>) -> Self {
        Self { ingress }
    }
}

#[async_trait]
impl EventCapabilityHandler for ExternalEventIngestHandler {
    fn intent_kind(&self) -> IntentKind {
        IntentKind::ExternalEventIngest
    }

    async fn handle(
        &self,
        event: &ExternalEventEnvelope,
        cancel: &CancellationToken,
    ) -> anyhow::Result<Vec<OutboundEffect>> {
        self.ingress
            .ingest(event, cancel)
            .await
            .map_err(|error| anyhow::anyhow!(error))?;
        Ok(Vec::new())
    }
}

//! Capability registry: dispatches a classified [`Intent`] to a typed
//! [`CapabilityHandler`], mirroring `LifecycleRegistry` /
//! `LifecycleContributor` (`service/lifecycle_contributors.rs`).
//!
//! The registry itself carries zero domain knowledge — it only knows how
//! to look a handler up by [`IntentKind`] and hand it a bounded
//! [`HandlerContext`]. Domain logic (chat/goal/approval/greeting) lives in
//! `capability/`.

use std::collections::BTreeMap;

use async_trait::async_trait;

use crate::channel::{ConversationId, InboundMessage, MessageId};

use super::effect::OutboundEffect;
use super::intent::Intent;

/// Fieldless discriminant of [`Intent`], used as the [`CapabilityRegistry`]
/// lookup key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntentKind {
    Greeting,
    Chat,
    GoalCommand,
    Unsupported,
    /// Event-sourced external event ingest (`ExternalEvent::MailReceived`). Never
    /// produced by [`classify_intent`](super::intent::classify_intent) —
    /// this key exists only so external event ingest can be registered and looked up
    /// through the same [`IntentKind`] namespace as chat capabilities,
    /// without going through duplex [`super::router::ChannelRouter::process`].
    /// Concrete external-event dispatch is owned by its provider adapter.
    ExternalEventIngest,
}

impl From<&Intent> for IntentKind {
    fn from(intent: &Intent) -> Self {
        match intent {
            Intent::Greeting => Self::Greeting,
            Intent::Chat(_) => Self::Chat,
            Intent::GoalCommand { .. } => Self::GoalCommand,
            Intent::Unsupported(_) => Self::Unsupported,
        }
    }
}

/// Minimal shared context a [`CapabilityHandler`] needs. Deliberately
/// bounded — no store handle, no transport handle: handlers only see what
/// they need to compute an [`OutboundEffect`].
#[derive(Debug, Clone)]
pub struct HandlerContext {
    pub channel: String,
    pub principal: String,
    pub conversation_id: ConversationId,
    pub message_id: MessageId,
    pub correlation_id: String,
    pub timestamp_ms: i64,
}

/// A typed capability, keyed by the [`Intent`] kind it handles.
#[async_trait]
pub trait CapabilityHandler: Send + Sync {
    fn intent_kind(&self) -> IntentKind;

    async fn handle(
        &self,
        ctx: &HandlerContext,
        inbound: &InboundMessage,
        intent: &Intent,
    ) -> anyhow::Result<Vec<OutboundEffect>>;
}

/// Registry of [`CapabilityHandler`]s keyed by [`IntentKind`].
///
/// At most one handler per kind: re-registering the same kind replaces the
/// previous handler (unlike `LifecycleRegistry`, which fans out to many
/// contributors per phase, exactly one capability answers a given intent).
#[derive(Default)]
pub struct CapabilityRegistry {
    handlers: BTreeMap<IntentKind, std::sync::Arc<dyn CapabilityHandler>>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, handler: std::sync::Arc<dyn CapabilityHandler>) {
        self.handlers.insert(handler.intent_kind(), handler);
    }

    /// Look up the handler for the classified `intent` and invoke it.
    ///
    /// Returns `Ok(None)` when no handler is registered for this intent's
    /// kind (e.g. [`IntentKind::Unsupported`]) — callers keep whatever
    /// reply text they already have.
    pub async fn dispatch(
        &self,
        ctx: &HandlerContext,
        inbound: &InboundMessage,
        intent: &Intent,
    ) -> anyhow::Result<Option<Vec<OutboundEffect>>> {
        let kind = IntentKind::from(intent);
        let Some(handler) = self.handlers.get(&kind) else {
            return Ok(None);
        };
        let effects = handler.handle(ctx, inbound, intent).await?;
        Ok(Some(effects))
    }
}

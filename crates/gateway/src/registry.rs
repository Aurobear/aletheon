//! Capability registry: dispatches a classified [`Intent`] to a typed
//! [`CapabilityHandler`], mirroring `LifecycleRegistry` /
//! `LifecycleContributor` (`service/lifecycle_contributors.rs`).
//!
//! The registry itself carries zero domain knowledge — it only knows how
//! to look a handler up by [`IntentKind`] and hand it a bounded
//! [`HandlerContext`]. Domain logic (chat/goal/approval/greeting) lives in
//! `handlers/`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use async_trait::async_trait;

use fabric::channel::{ConversationId, InboundMessage, MessageId};
use fabric::ApprovalCategory;

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
    /// Event-sourced Gmail ingest (`GoogleEvent::MailReceived`). Never
    /// produced by [`classify_intent`](super::intent::classify_intent) —
    /// this key exists only so Gmail ingest can be registered and looked up
    /// through the same [`IntentKind`] namespace as chat capabilities,
    /// without going through duplex [`super::dispatcher::ChannelDispatcher::process`].
    /// See [`EventCapabilityHandler`] / [`EventCapabilityRegistry`].
    GmailIngest,
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

// ---------------------------------------------------------------------------
// Event-capability seam (non-duplex; e.g. Gmail ingest)
// ---------------------------------------------------------------------------

/// A capability triggered by an inbound *event* rather than a classified
/// chat [`Intent`] — e.g. Gmail's `GoogleEvent::MailReceived` ingest.
///
/// Deliberately separate from [`CapabilityHandler`]: event capabilities are
/// invoked directly by their event source (see `google/event_dispatcher.rs`),
/// never through [`CapabilityRegistry::dispatch`] or
/// `ChannelDispatcher::process` (no inbox dedup, no `complete_inbound`, no
/// `transport.send`). Implementations keep their own stores, idempotency,
/// and security checks entirely — this seam only makes them a first-class,
/// registry-addressable capability instead of a hardcoded sink field.
#[async_trait]
pub trait EventCapabilityHandler: Send + Sync {
    fn intent_kind(&self) -> IntentKind;

    async fn handle(
        &self,
        event: &fabric::ExternalEventEnvelope,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> anyhow::Result<Vec<OutboundEffect>>;
}

/// Registry of [`EventCapabilityHandler`]s keyed by [`IntentKind`]. Held
/// alongside (not instead of) [`CapabilityRegistry`]: event capabilities are
/// looked up by their event source, not by `dispatch()`.
#[derive(Default)]
pub struct EventCapabilityRegistry {
    handlers: BTreeMap<IntentKind, std::sync::Arc<dyn EventCapabilityHandler>>,
}

impl EventCapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, handler: std::sync::Arc<dyn EventCapabilityHandler>) {
        self.handlers.insert(handler.intent_kind(), handler);
    }

    /// Look up the handler registered for `kind`, if any.
    pub fn get(&self, kind: IntentKind) -> Option<std::sync::Arc<dyn EventCapabilityHandler>> {
        self.handlers.get(&kind).cloned()
    }
}

// ---------------------------------------------------------------------------
// Approval resolver seam
// ---------------------------------------------------------------------------

/// Executes the side effect of a resolved approval (post commit) and,
/// optionally, revises a draft awaiting fresh confirmation.
///
/// Replaces the former `ChannelApprovalExecutor` + `GmailDraftApprovalExecutor`
/// traits and the `ActivateGoal` special-case fork in `execute_approval_action`:
/// resolvers register per [`ApprovalCategory`], with an optional default for
/// categories that don't need a dedicated resolver.
#[async_trait]
pub trait ApprovalResolver: Send + Sync {
    async fn execute_resolved(
        &self,
        approval: &fabric::ApprovalSnapshot,
        action: &str,
        now_ms: i64,
    ) -> anyhow::Result<()>;

    /// Revise a draft awaiting fresh confirmation. Only meaningful for
    /// resolvers that back editable drafts (e.g. Gmail's `ActivateGoal`
    /// resolver); the default implementation is unsupported.
    async fn revise_draft(
        &self,
        _owner: &str,
        _goal_id: fabric::GoalId,
        _intent: &str,
        _now_ms: i64,
    ) -> anyhow::Result<fabric::ApprovalSnapshot> {
        anyhow::bail!("revision is not supported by this approval resolver")
    }
}

/// Small registry of [`ApprovalResolver`]s keyed by [`ApprovalCategory`],
/// plus one optional default for categories without a dedicated resolver.
///
/// Interior mutability (`Mutex`) lets a single `Arc<ApprovalResolverRegistry>`
/// be shared between the dispatcher (approval callback path) and any
/// capability handler that also needs resolver access (e.g. `GoalHandler`'s
/// `/edit`), regardless of registration order.
#[derive(Default)]
pub struct ApprovalResolverRegistry {
    by_category: Mutex<HashMap<ApprovalCategory, std::sync::Arc<dyn ApprovalResolver>>>,
    default: Mutex<Option<std::sync::Arc<dyn ApprovalResolver>>>,
}

impl ApprovalResolverRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        category: ApprovalCategory,
        resolver: std::sync::Arc<dyn ApprovalResolver>,
    ) {
        self.by_category.lock().unwrap().insert(category, resolver);
    }

    pub fn set_default(&self, resolver: std::sync::Arc<dyn ApprovalResolver>) {
        *self.default.lock().unwrap() = Some(resolver);
    }

    /// Resolve a resolver registered specifically for `category`, falling
    /// back to the default resolver (if any).
    pub fn resolve(
        &self,
        category: ApprovalCategory,
    ) -> Option<std::sync::Arc<dyn ApprovalResolver>> {
        self.by_category
            .lock()
            .unwrap()
            .get(&category)
            .cloned()
            .or_else(|| self.default.lock().unwrap().clone())
    }

    /// Resolve a resolver registered specifically for `category`, ignoring
    /// the default. Used where a missing category-specific resolver must be
    /// a hard error (e.g. Gmail's `ActivateGoal` resolver).
    pub fn resolve_category_only(
        &self,
        category: ApprovalCategory,
    ) -> Option<std::sync::Arc<dyn ApprovalResolver>> {
        self.by_category.lock().unwrap().get(&category).cloned()
    }
}

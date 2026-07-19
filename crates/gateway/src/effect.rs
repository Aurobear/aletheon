//! Bounded effect returned by a [`super::registry::CapabilityHandler`].
//!
//! Mirrors `LifecycleEffect` (`service/lifecycle_contributors.rs`): the
//! dispatcher only ever sees this closed enum, never a handler's internal
//! state, so adding a new capability can never widen the dispatcher's
//! surface.

use fabric::channel::OutboundMessage;

/// Effect a [`super::registry::CapabilityHandler`] asks the dispatcher to
/// perform after handling an inbound message.
#[derive(Debug, Clone)]
pub enum OutboundEffect {
    /// Reply directly to the inbound message that triggered this handler.
    Reply(OutboundMessage),
    /// Enqueue a message for later delivery (not a direct reply).
    Enqueue(OutboundMessage),
    /// No outbound effect.
    None,
}

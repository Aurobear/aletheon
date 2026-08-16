//! Identifiers for bounded Runtime event-projection subscriptions.

use serde::{Deserialize, Serialize};

/// Unique identifier for an event subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubscriptionId(pub u64);

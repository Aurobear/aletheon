//! Runtime owner ports (RA-01).
//!
//! Narrow ports the Runtime exposes to its composition root and to owners.
//! These define contract/port only — nothing appends, spawns, or cuts over a
//! legacy writer in this seam.

use crate::command::{CommandReceipt, RuntimeCommand};
use crate::error::RuntimeError;
use crate::event::RuntimeEvent;
use crate::query::RuntimeQuery;
use async_trait::async_trait;

/// Runtime command boundary.  Implemented by the Runtime composition root;
/// consumed by adapters (Gateway/ACP/TUI) that translate caller requests.
#[async_trait]
pub trait RuntimeCommandPort: Send + Sync {
    async fn dispatch(&self, command: RuntimeCommand) -> Result<CommandReceipt, RuntimeError>;
}

/// Runtime query boundary.
#[async_trait]
pub trait RuntimeQueryPort: Send + Sync {
    async fn query(&self, query: RuntimeQuery) -> Result<serde_json::Value, RuntimeError>;
}

/// Runtime event subscription boundary (snapshot + cursor recovery).
#[async_trait]
pub trait RuntimeEventPort: Send + Sync {
    async fn next_event(&self) -> Result<RuntimeEvent, RuntimeError>;
}

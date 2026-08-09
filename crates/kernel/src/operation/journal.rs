//! K1 durable Operation authority seam (Agent Kernel V2).
//!
//! Contract/port only — nothing writes here yet.  The legacy in-memory
//! `OperationTable` (kernel/src/operation/table.rs) remains authoritative;
//! V2 only shadows/replays/compares until K4's single admit/invoke/receipt/
//! recovery path.  No new writer and the legacy table is never written by
//! this module.
//!
//! Runtime stores only Operation binding/receipt refs; it never copies the
//! Kernel terminal.

use serde::{Deserialize, Serialize};

/// Durable operation state machine (epoch-gated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OperationState {
    Submitted,
    Running,
    Succeeded,
    Failed,
    Panicked,
}

impl OperationState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            OperationState::Succeeded | OperationState::Failed | OperationState::Panicked
        )
    }
}

/// Generation/epoch for an operation's writer generation (prevents stale
/// writers from mutating a newer epoch's record).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OperationEpoch(pub u64);

/// Durable operation record.  This is the Kernel-owned authority shape; the
/// legacy `OperationRecord` in table.rs is the in-memory view until K4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRecord {
    pub id: fabric::OperationId,
    pub state: OperationState,
    pub epoch: OperationEpoch,
    pub parent: Option<fabric::OperationId>,
}

/// Operation receipt returned after admission.  Runtime/owners hold the ref;
/// the Kernel owns the terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationReceipt {
    pub id: fabric::OperationId,
    pub epoch: OperationEpoch,
}

/// Kernel-owned command to create/advance an operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OperationCommand {
    Submit {
        id: fabric::OperationId,
        parent: Option<fabric::OperationId>,
    },
    Start {
        id: fabric::OperationId,
    },
    Settle {
        id: fabric::OperationId,
        state: OperationState,
    },
}

/// Durable operation journal port.  Implemented by the Kernel composition root;
/// consumed by owners to read/replay.  No writer here (K1 seam is read/shadow).
#[async_trait::async_trait]
pub trait ExecutionJournal: Send + Sync {
    /// Read a single operation record by id.
    async fn read(&self, id: fabric::OperationId) -> Option<OperationRecord>;
    /// Replay all records after a given epoch (for restart recovery/shadow).
    async fn replay_after(&self, after_epoch: OperationEpoch) -> Vec<OperationRecord>;
}

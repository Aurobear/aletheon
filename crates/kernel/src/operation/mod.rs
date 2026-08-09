//! Operation table, structured task groups, and the K1 durable Operation
//! authority seam.

pub mod journal;
pub(crate) mod table;
pub mod task_group;

pub use journal::{
    ExecutionJournal, OperationCommand, OperationEpoch, OperationReceipt, OperationRecord,
    OperationState,
};
pub(crate) use table::OperationTable;
pub use task_group::{
    operation_scope_metrics, OperationScope, OperationScopeCleanupKind,
    OperationScopeCleanupReport, OperationScopeMetrics, TaskExit,
};

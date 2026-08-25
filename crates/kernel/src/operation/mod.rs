//! Authoritative operation table and structured task groups.

pub(crate) mod table;
pub mod task_group;

pub(crate) use table::OperationTable;
pub use task_group::{
    operation_scope_metrics, OperationScope, OperationScopeCleanupKind,
    OperationScopeCleanupReport, OperationScopeMetrics, TaskExit,
};

//! Operation table and structured task groups.

pub mod table;
pub mod task_group;

pub use table::OperationTable;
pub use task_group::{OperationScope, TaskExit};

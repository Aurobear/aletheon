//! Kernel execution primitives: process table, operation tree, chronos, supervision.

pub mod admission;
pub mod capability;
pub mod chronos;
pub mod debug;
pub mod debug_bus;
pub mod enforcement;
pub mod lifecycle;
pub mod operation;
pub mod process;
pub mod runtime;
pub mod space;
pub mod supervision;

pub use admission::{AdmissionController, BudgetController, LeaseManager};
pub use lifecycle::{OperationHandle, OperationManager, ProcessHandle, ProcessManager};
pub use runtime::{KernelRuntime, LifecycleFaultInjector};

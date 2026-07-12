//! Kernel execution primitives: process table, operation tree, chronos, supervision.

pub mod admission;
pub mod capability;
pub mod chronos;
pub mod operation;
pub mod process;
pub mod service;
pub mod space;
pub mod supervision;

// Backward compatibility: old path aletheon_kernel::service::ServicePorts
pub use service as service_ports;

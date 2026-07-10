//! Inter-process communication — protocol, transport, and bus.

pub mod backends;
pub mod bus;
pub mod envelope;
pub mod ipc_msg;
pub mod ipc_types;
pub mod protocol;
pub mod transport;

// Backward compatibility: re-export ipc_msg types at this level
// so `fabric::ipc::IpcMessage` still works (was `fabric::ipc::IpcMessage` before).
pub use ipc_msg::*;

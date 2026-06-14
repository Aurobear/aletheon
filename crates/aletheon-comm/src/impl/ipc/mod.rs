//! # IPC — Inter-Process Communication backends
//!
//! Provides Unix socket, io_uring, and shared memory
//! IPC backends with auto-detection and runtime fallback.

pub mod unix_socket;
pub mod io_uring;
pub mod shared_mem;
pub mod json_rpc;
pub mod priority_queue;
pub mod manager;

pub use manager::{IpcManager, IpcBackendKind, Environment};
pub use priority_queue::PriorityQueue;
pub use json_rpc::JsonRpcAdapter;

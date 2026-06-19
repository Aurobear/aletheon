//! # IPC — Inter-Process Communication backends
//!
//! Provides Unix socket, io_uring, and shared memory
//! IPC backends with auto-detection and runtime fallback.

pub mod io_uring;
pub mod json_rpc;
pub mod manager;
pub mod priority_queue;
pub mod shared_mem;
pub mod unix_socket;

pub use json_rpc::JsonRpcAdapter;
pub use manager::{Environment, IpcBackendKind, IpcManager};
pub use priority_queue::PriorityQueue;

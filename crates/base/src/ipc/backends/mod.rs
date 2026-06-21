//! IpcBackend implementations (legacy, being replaced by Transport).

pub mod io_uring;
pub mod json_rpc;
pub mod manager;
pub mod priority_queue;
pub mod shared_mem;
pub mod transport_adapter;
pub mod unix_socket;

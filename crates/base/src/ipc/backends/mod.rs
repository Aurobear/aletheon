//! IpcBackend implementations (legacy, being replaced by Transport).

pub mod io_uring;
pub mod io_uring_transport;
pub mod json_rpc;
pub mod json_rpc_transport;
pub mod manager;
pub mod priority_queue;
pub mod shared_mem;
pub mod shared_mem_transport;
pub mod transport_adapter;
pub mod unix_socket;

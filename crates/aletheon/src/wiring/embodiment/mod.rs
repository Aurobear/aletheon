//! Binary-owned composition adapter for governed embodiment execution.

pub mod authority;
pub mod service;

pub use authority::{
    build_embodiment_invoker, ActiveEmbodimentOperation, ActiveEmbodimentOperations,
};
pub use service::EmbodimentService;

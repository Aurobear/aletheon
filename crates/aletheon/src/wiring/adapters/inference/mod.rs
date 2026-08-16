//! Host-owned HTTP inference transports and provider construction.

mod anthropic;
pub(crate) mod backpressure;
pub mod factory;
mod ollama;
mod openai_provider;
mod provider;
pub mod registry;
mod utf8_stream;

pub use registry::ProviderRegistry;

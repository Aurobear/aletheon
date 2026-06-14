//! Implementation layer — concrete providers, routers, and registries.

pub mod llm;
pub mod inference;
pub mod learning;
pub mod grounding;
pub mod provider_registry;

pub use provider_registry::ProviderRegistry;

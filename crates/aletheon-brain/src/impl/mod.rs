//! Implementation layer — concrete providers, routers, and registries.

pub mod event_handlers;
pub mod grounding;
pub mod inference;
pub mod learning;
pub mod llm;
pub mod provider_registry;

pub use provider_registry::ProviderRegistry;

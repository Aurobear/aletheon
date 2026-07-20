//! Re-exports of the canonical LLM provider trait and types.
//!
//! These items now live in `fabric` (RFC-018 Phase 4, resolves D4) since they
//! are a shared client abstraction, not cognit-specific implementation. This
//! shim keeps every existing cognit-internal path
//! (`crate::r#impl::llm::provider::LlmProvider`, `cognit::r#impl::llm::LlmProvider`,
//! `cognit::llm::provider::LlmProvider`, etc.) resolving unchanged.
pub use fabric::{LlmProvider, LlmResponse, LlmStream, ModelInfo, StopReason, StreamChunk, Usage};

/// Tool definition sent to the LLM.
pub use fabric::ToolDefinition;

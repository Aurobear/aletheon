//! Cognitive engine module - the ReAct loop for agent reasoning and action.
//!
//! This module implements the cognitive engine, split into:
//! - config.rs: EngineConfig struct and Default impl
//! - cognitive_loop.rs: Main Engine struct, run() method, ReAct loop
//! - tool_dispatch.rs: Tool selection, execution, result handling (docs only)
//! - memory_integration.rs: Memory read/write, learning outcome recording
//! - streaming.rs: LLM streaming, chunk handling

pub mod config;
pub mod cognitive_loop;
pub mod tool_dispatch;
pub mod memory_integration;
pub mod streaming;

// Re-export key types
pub use config::EngineConfig;
pub use cognitive_loop::Engine;

//! Cognitive engine module - the ReAct loop for agent reasoning and action.
//!
//! # Deprecated
//!
//! This module is deprecated. It has been superseded by `ReActLoop` in
//! `crate::core::react_loop`. New code should use `ReActLoop` instead of `Engine`.
//!
//! The module is retained for backward compatibility only and will be removed
//! in a future release.
//!
//! This module implements the cognitive engine, split into:
//! - config.rs: EngineConfig struct and Default impl
//! - cognitive_loop.rs: Main Engine struct, run() method, ReAct loop
//! - tool_dispatch.rs: Tool selection, execution, result handling (docs only)
//! - memory_integration.rs: Memory read/write, learning outcome recording
//! - streaming.rs: LLM streaming, chunk handling

pub mod cognitive_loop;
pub mod config;
pub mod memory_integration;
pub mod modules;
pub mod streaming;
pub mod tool_dispatch;

// Re-export key types
pub use cognitive_loop::{Engine, TurnResult};
pub use config::EngineConfig;

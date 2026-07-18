//! Shared test infrastructure for executive integration tests.
//!
//! # Modules
//!
//! - `mock_llm_provider`: Deterministic mock LLM with pre-configured response sequences.
//! - `mock_sandbox`: Deterministic mock sandbox with FIFO tool result queues.
//! - `test_aletheon_builder`: Builder for fully wired test Aletheon instances.
//! - `conscious_core_harness`: Conscious core test harness (legacy).

#![allow(dead_code)]

pub mod conscious_core_harness;
pub mod mock_llm_provider;
pub mod mock_sandbox;
pub mod test_aletheon_builder;

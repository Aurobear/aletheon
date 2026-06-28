//! Tool dispatch and execution helpers for the cognitive engine.
//!
//! The actual tool dispatch logic is integrated into the ReAct loop in
//! cognitive_loop.rs and streaming.rs. This module exists for future
//! extraction of tool-related helper functions if needed.

// Currently, tool dispatch is handled inline in:
// - Engine::run_turn() in cognitive_loop.rs
// - Engine::run_turn_streaming() in streaming.rs
//
// The dispatch logic includes:
// 1. delegate_task routing through DelegateTool when agent_registry is configured
// 2. Guarded execution via ToolRunnerWithGuard when security is enabled
// 3. Direct tool execution as fallback
//
// Error handling covers:
// - ToolError::PolicyDenied
// - ToolError::LoopBlocked
// - ToolError::EscalateToHuman
// - ToolError::InterruptTurn

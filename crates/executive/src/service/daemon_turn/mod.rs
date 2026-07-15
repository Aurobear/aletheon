//! Daemon Turn Orchestrator — the macro-kernel turn pipeline for daemon chat.
//!
//! This module extracts the orchestration logic from `RequestHandler::handle_chat`
//! into the service layer. The handler becomes a thin delegation layer that:
//!
//! 1. Parses the JSON-RPC request
//! 2. Delegates to `DaemonTurnOrchestrator::execute_turn()`
//! 3. Formats the JSON-RPC response
//!
//! # Module structure
//!
//! | File | Purpose |
//! |------|---------|
//! | `orchestrator.rs` | Struct definition + `new()` |
//! | `execute.rs` | `execute_turn()` — main orchestration entry point |
//! | `lifecycle.rs` | Kernel process management (`ensure_main_agent`) |
//! | `session.rs` | Session manager helpers |
//! | `self_field.rs` | SelfField review + narrate + memory block |
//! | `injection.rs` | Pre-turn message injection (skills, facts, memory) |
//! | `post_phases.rs` | Post-turn hooks, reflection, evolution, Agora |
//! | `helpers.rs` | Text helpers and size constants |
//!
//! # Kernel primitives wired
//!
//! - **KernelRuntime process API**: main agent process is registered and tracked.
//! - **KernelRuntime operation API**: each turn creates an operation for cancellation tracking.
//! - **SupervisorTree**: agent process has a restart policy.
//! - **AdmissionController**: tool execution gates through admission (production).
//! - **MailboxService**: registered per agent process for future inter-process comms.

mod execute;
pub mod gbrain;
pub(crate) mod helpers;
mod lifecycle;
mod orchestrator;
mod self_field;
mod session;

pub use orchestrator::DaemonTurnOrchestrator;

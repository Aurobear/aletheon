//! Concrete infrastructure adapters.
//!
//! Application modules depend on ports; composition is the only production
//! layer allowed to construct these implementations.

pub mod agent_memory;
pub mod approved_apply;
pub mod capability;
pub mod channel;
pub mod cognitive;
pub mod conscious;
// (context_memory and gbrain adapters moved to the standalone `adapters-gbrain` crate)
pub mod context_source;
pub mod evaluation;
pub mod external;
pub mod goal_evaluation;
pub mod goal_progress;
pub mod hooks;
// (inference adapter moved to the standalone `adapters-inference` crate)
pub mod memory_projection;
pub mod post_turn;

pub mod turn_postflight;
pub mod turn_preflight;

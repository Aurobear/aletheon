//! Channel-local capability adapters.
//!
//! These handlers are intentionally limited to syntax classification and
//! typed Application ports. They do not own persistence, session/turn
//! authority, or business state transitions.

pub mod chat;
pub mod goal;
pub mod greeting;

pub use chat::ChatHandler;
pub use goal::GoalHandler;
pub use greeting::GreetingHandler;

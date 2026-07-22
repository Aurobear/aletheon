//! Concrete infrastructure adapters.
//!
//! Application modules depend on ports; composition is the only production
//! layer allowed to construct these implementations.

pub mod agent_control;
pub mod artifact;
pub mod channel;
pub mod external;
pub mod gbrain;
pub mod google;
pub mod runtime;

pub(crate) mod events;
pub(crate) mod plugin;
pub(crate) mod session;

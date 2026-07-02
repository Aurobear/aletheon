//! Subsystem trait contracts — like Linux kernel's include/ directory.
//!
//! Each file defines the trait contract for one subsystem.

pub mod body;
pub mod brain;
pub mod event_bus;
pub mod memory;
pub mod meta;
pub mod runtime;
pub mod self_field;
pub mod subsystem;
pub mod plugin;
